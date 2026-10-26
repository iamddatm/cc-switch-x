//! Skill 来源安装、卸载与批量启停。
//!
//! 安装先落到与真源同卷的 `state/staging`，写好 `.meta.json` 后再 rename 到
//! `assets/skills/<id>`；覆盖安装前必须成功备份，失败绝不继续。

use crate::adapters::skills::{self, Deployed, RefreshOutcome, Removed};
use crate::domain::agent::{builtin_agents, AgentDescriptor};
use crate::domain::skill_meta::{read_meta, write_meta, SkillMeta, SkillSourceKind};
use crate::domain::{AssetKind, AssetType};
use crate::error::AppError;
use crate::import::mcp::ConflictPolicy;
use crate::import::{safe_asset_id, slug, update_matrix};
use crate::net::archive::{self, ArchiveLimits};
use crate::net::{parse_github_input, RepoFetcher, RepoRef};
use crate::services::agents::{ensure_agent_detected, load_state, read_matrix};
use crate::store::{copy_dir, Store};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const PREVIEW_RECORD: &str = "preview.json";
const CONTENT_DIR: &str = "content";
/// 与发现页共用同一量级：Composio 这类聚合仓库单仓就有数百个 SKILL.md，
/// 上限太低会让「发现后直接安装」在预览阶段就失败。
const MAX_CANDIDATES: usize = 2000;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SkillSourceInput {
    Github {
        repo: String,
        branch: Option<String>,
    },
    Zip {
        path: PathBuf,
    },
    Directory {
        path: PathBuf,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillCandidate {
    pub relative_path: String,
    pub name: String,
    pub description: Option<String>,
    pub suggested_id: String,
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillSourcePreview {
    pub kind: SkillSourceKind,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub staging_id: Option<String>,
    pub candidates: Vec<SkillCandidate>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallSelection {
    pub relative_path: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallOutcome {
    pub id: String,
    pub name: String,
    /// installed / overwritten / skipped / failed
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 空数组也必须序列化：前端按数组渲染，省略字段会把整个面板打崩。
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallReport {
    pub outcomes: Vec<SkillInstallOutcome>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillItemFailure {
    pub id: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillUninstallOutcome {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
    pub disabled_agents: Vec<String>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillUninstallReport {
    pub outcomes: Vec<SkillUninstallOutcome>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillBatchOk {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillBatchReport {
    pub ok: Vec<SkillBatchOk>,
    pub failed: Vec<SkillItemFailure>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviewRecord {
    kind: SkillSourceKind,
    repo_owner: Option<String>,
    repo_name: Option<String>,
    branch: Option<String>,
    source_url: Option<String>,
}

#[derive(Debug, Clone)]
struct SourceContext {
    kind: SkillSourceKind,
    /// 候选目录的根：预览的 content/ 或本地目录。
    root: PathBuf,
    repo: Option<RepoRef>,
    branch: Option<String>,
    source_url: Option<String>,
    /// Git/ZIP 预览暂存；安装结束后删除。
    staging: Option<PathBuf>,
}

static STAGING_COUNTER: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn staging_name(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}",
        now_ms(),
        STAGING_COUNTER.fetch_add(1, Ordering::SeqCst)
    )
}

/// 暂存 ID 只允许我们生成的格式，绝不能从 ID 里拼出路径。
fn staging_path(store: &Store, id: &str) -> Result<PathBuf, AppError> {
    if id.is_empty()
        || id == "."
        || id == ".."
        || id.contains('/')
        || id.contains('\\')
        || !id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    {
        return Err(AppError::InvalidInput("非法的预览暂存 ID".into()));
    }
    let path = store.staging_dir().join(id);
    if !path.starts_with(store.staging_dir()) {
        return Err(AppError::InvalidInput("非法的预览暂存路径".into()));
    }
    Ok(path)
}

pub fn preview<F: RepoFetcher>(
    store: &Store,
    fetcher: &F,
    input: &SkillSourceInput,
) -> Result<SkillSourcePreview, AppError> {
    match input {
        SkillSourceInput::Github { repo, branch } => {
            let parsed = parse_github_input(repo, branch.clone())?;
            let fetched = fetcher.fetch(&parsed)?;
            let staging_id = staging_name("preview");
            let staging = staging_path(store, &staging_id)?;
            let content = staging.join(CONTENT_DIR);
            let candidates = finish_preview(&staging, || {
                fs::create_dir_all(&content)?;
                copy_dir(fetched.dir.path(), &content)?;
                let record = PreviewRecord {
                    kind: SkillSourceKind::Github,
                    repo_owner: Some(parsed.owner.clone()),
                    repo_name: Some(parsed.name.clone()),
                    branch: Some(fetched.branch.clone()),
                    source_url: Some(format!(
                        "https://github.com/{}/{}",
                        parsed.owner, parsed.name
                    )),
                };
                write_preview_record(&staging, &record)?;
                collect_candidates(
                    store,
                    &content,
                    CandidateNaming::Github {
                        owner: &parsed.owner,
                        repo: &parsed.name,
                    },
                )
            })?;
            Ok(SkillSourcePreview {
                kind: SkillSourceKind::Github,
                repo: Some(parsed.slug()),
                branch: Some(fetched.branch),
                staging_id: Some(staging_id),
                candidates,
                warnings: Vec::new(),
            })
        }
        SkillSourceInput::Zip { path } => {
            if !path.is_file() {
                return Err(AppError::Source(format!(
                    "ZIP 文件不存在：{}",
                    path.display()
                )));
            }
            let staging_id = staging_name("preview");
            let staging = staging_path(store, &staging_id)?;
            let content = staging.join(CONTENT_DIR);
            let candidates = finish_preview(&staging, || {
                archive::extract_archive(path, &content, &ArchiveLimits::default())?;
                let record = PreviewRecord {
                    kind: SkillSourceKind::Zip,
                    repo_owner: None,
                    repo_name: None,
                    branch: None,
                    source_url: Some(path.display().to_string()),
                };
                write_preview_record(&staging, &record)?;
                let stem = path
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or("skill");
                collect_candidates(store, &content, CandidateNaming::Zip { fallback: stem })
            })?;
            Ok(SkillSourcePreview {
                kind: SkillSourceKind::Zip,
                repo: None,
                branch: None,
                staging_id: Some(staging_id),
                candidates,
                warnings: Vec::new(),
            })
        }
        SkillSourceInput::Directory { path } => {
            let root = canonical_dir(path)?;
            let fallback = root
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("skill");
            let candidates =
                collect_candidates(store, &root, CandidateNaming::Directory { fallback })?;
            Ok(SkillSourcePreview {
                kind: SkillSourceKind::Local,
                repo: None,
                branch: None,
                staging_id: None,
                candidates,
                warnings: Vec::new(),
            })
        }
    }
}

/// 预览失败时不能把半成品暂存留在 `state/staging` 里。
fn finish_preview<T>(
    staging: &Path,
    build: impl FnOnce() -> Result<T, AppError>,
) -> Result<T, AppError> {
    match build() {
        Ok(value) => Ok(value),
        Err(error) => {
            let _ = fs::remove_dir_all(staging);
            Err(error)
        }
    }
}

enum CandidateNaming<'a> {
    Github { owner: &'a str, repo: &'a str },
    Zip { fallback: &'a str },
    Directory { fallback: &'a str },
}

fn collect_candidates(
    store: &Store,
    root: &Path,
    naming: CandidateNaming<'_>,
) -> Result<Vec<SkillCandidate>, AppError> {
    let scanned = archive::scan_skills(root, MAX_CANDIDATES)?;
    if scanned.is_empty() {
        return Err(AppError::Source("来源里没有找到含 SKILL.md 的目录".into()));
    }
    let mut out = Vec::with_capacity(scanned.len());
    for skill in scanned {
        let suggested_id = candidate_id(&skill.relative_path, &naming);
        let exists = store
            .entry_path(AssetType::Skill, &suggested_id)
            .map(|path| path.exists())
            .unwrap_or(false);
        out.push(SkillCandidate {
            relative_path: skill.relative_path,
            name: skill.name,
            description: skill.description,
            suggested_id,
            exists,
        });
    }
    Ok(out)
}

fn candidate_id(relative_path: &str, naming: &CandidateNaming<'_>) -> String {
    match naming {
        CandidateNaming::Github { owner, repo } => github_candidate_id(relative_path, owner, repo),
        CandidateNaming::Zip { fallback } | CandidateNaming::Directory { fallback } => {
            let raw = if relative_path == "." {
                (*fallback).to_string()
            } else {
                relative_path.to_string()
            };
            slug(&raw).unwrap_or_else(|| safe_asset_id(fallback))
        }
    }
}

/// 发现与安装必须算出同一个 ID，否则「已安装」判断会和实际落盘对不上。
pub(crate) fn github_candidate_id(relative_path: &str, owner: &str, repo: &str) -> String {
    let raw = if relative_path == "." {
        format!("{owner}-{repo}")
    } else {
        format!("{owner}-{repo}-{relative_path}")
    };
    // slug 对全中文返回 None；GitHub 前缀全是 ASCII，可作为稳定兜底。
    slug(&raw).unwrap_or_else(|| safe_asset_id(&format!("{owner}-{repo}")))
}

/// 发现页直接安装时用来定位候选：优先精确路径，其次目录/skillId，最后唯一的名称。
pub(crate) fn pick_candidate<'a>(
    candidates: &'a [SkillCandidate],
    relative_path: Option<&str>,
    directory: Option<&str>,
    name: Option<&str>,
) -> Option<&'a SkillCandidate> {
    if let Some(relative) = relative_path.map(str::trim).filter(|v| !v.is_empty()) {
        if let Some(found) = candidates
            .iter()
            .find(|item| item.relative_path == relative)
        {
            return Some(found);
        }
    }
    if let Some(directory) = directory.map(str::trim).filter(|v| !v.is_empty()) {
        let directory = directory.trim_matches('/');
        if let Some(found) = candidates
            .iter()
            .find(|item| item.relative_path == directory)
        {
            return Some(found);
        }
        if let Some(found) = candidates.iter().find(|item| {
            item.relative_path
                .rsplit('/')
                .next()
                .is_some_and(|leaf| leaf == directory)
        }) {
            return Some(found);
        }
        if let Some(found) = candidates
            .iter()
            .find(|item| item.suggested_id == directory)
        {
            return Some(found);
        }
    }
    if let Some(name) = name.map(str::trim).filter(|v| !v.is_empty()) {
        let needle = name.to_lowercase();
        let mut matched = candidates
            .iter()
            .filter(|item| item.name.to_lowercase() == needle);
        if let (Some(found), None) = (matched.next(), matched.next()) {
            return Some(found);
        }
    }
    if candidates.len() == 1 {
        return candidates.first();
    }
    None
}

fn write_preview_record(staging: &Path, record: &PreviewRecord) -> Result<(), AppError> {
    let bytes = serde_json::to_vec_pretty(record)?;
    crate::store::atomic::write_atomic(&staging.join(PREVIEW_RECORD), &bytes)?;
    Ok(())
}

fn read_preview_record(staging: &Path) -> Result<PreviewRecord, AppError> {
    let path = staging.join(PREVIEW_RECORD);
    let bytes = fs::read(&path)
        .map_err(|_| AppError::Source("预览已过期或不存在，请重新扫描来源".into()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| AppError::Source(format!("预览信息损坏：{error}")))
}

fn canonical_dir(path: &Path) -> Result<PathBuf, AppError> {
    if !path.is_dir() {
        return Err(AppError::Source(format!(
            "选择的目录不存在：{}",
            path.display()
        )));
    }
    path.canonicalize()
        .map_err(|error| AppError::Source(format!("无法读取目录：{error}")))
}

fn resolve_context(
    input: &SkillSourceInput,
    staging_id: Option<&str>,
    store: &Store,
) -> Result<SourceContext, AppError> {
    match input {
        SkillSourceInput::Github { repo, branch } => {
            let parsed = parse_github_input(repo, branch.clone())?;
            let staging_id =
                staging_id.ok_or_else(|| AppError::Source("缺少预览信息，请先扫描仓库".into()))?;
            let staging = staging_path(store, staging_id)?;
            let record = read_preview_record(&staging)?;
            if record.kind != SkillSourceKind::Github
                || record.repo_owner.as_deref() != Some(parsed.owner.as_str())
                || record.repo_name.as_deref() != Some(parsed.name.as_str())
            {
                return Err(AppError::Source(
                    "预览内容与当前仓库不一致，请重新扫描".into(),
                ));
            }
            Ok(SourceContext {
                kind: SkillSourceKind::Github,
                root: staging.join(CONTENT_DIR),
                repo: Some(parsed),
                branch: record.branch,
                source_url: record.source_url,
                staging: Some(staging),
            })
        }
        SkillSourceInput::Zip { path } => {
            let staging_id =
                staging_id.ok_or_else(|| AppError::Source("缺少预览信息，请先扫描 ZIP".into()))?;
            let staging = staging_path(store, staging_id)?;
            let record = read_preview_record(&staging)?;
            if record.kind != SkillSourceKind::Zip
                || record.source_url.as_deref() != Some(path.display().to_string().as_str())
            {
                return Err(AppError::Source(
                    "预览内容与当前 ZIP 不一致，请重新扫描".into(),
                ));
            }
            Ok(SourceContext {
                kind: SkillSourceKind::Zip,
                root: staging.join(CONTENT_DIR),
                repo: None,
                branch: None,
                source_url: record.source_url,
                staging: Some(staging),
            })
        }
        SkillSourceInput::Directory { path } => {
            let root = canonical_dir(path)?;
            Ok(SourceContext {
                kind: SkillSourceKind::Local,
                source_url: Some(root.display().to_string()),
                root,
                repo: None,
                branch: None,
                staging: None,
            })
        }
    }
}

/// 发现页直接安装：重新拉一次仓库（不信任前端路径），定位候选后复用预览暂存安装。
pub fn install_discovered<F: RepoFetcher>(
    sink: SkillSink<'_>,
    fetcher: &F,
    repo: &str,
    branch: Option<String>,
    relative_path: Option<&str>,
    directory: Option<&str>,
    name: Option<&str>,
    policy: ConflictPolicy,
) -> Result<SkillInstallOutcome, AppError> {
    let store = sink.store();
    let input = SkillSourceInput::Github {
        repo: repo.to_string(),
        branch,
    };
    let preview = preview(store, fetcher, &input)?;
    let staging_id = preview.staging_id.clone();
    let result: Result<SkillInstallOutcome, AppError> = (|| {
        let candidate = pick_candidate(&preview.candidates, relative_path, directory, name)
            .ok_or_else(|| {
                AppError::Source("仓库里没有找到对应的 Skill，可能已改名，请刷新发现列表".into())
            })?;
        let staging_id = staging_id
            .as_deref()
            .ok_or_else(|| AppError::Source("缺少预览信息，请重试".into()))?;
        let selection = InstallSelection {
            relative_path: candidate.relative_path.clone(),
            id: candidate.suggested_id.clone(),
        };
        let report = install(sink, &input, Some(staging_id), &[selection], policy)?;
        report
            .outcomes
            .into_iter()
            .next()
            .ok_or_else(|| AppError::InvalidInput("安装没有返回结果".into()))
    })();
    // 成功安装由 install() 清理暂存；定位/安装失败时这里兜底，避免留下整份仓库副本。
    if result.is_err() {
        if let Some(id) = staging_id.as_deref() {
            if let Ok(path) = staging_path(store, id) {
                let _ = fs::remove_dir_all(path);
            }
        }
    }
    result
}

/// 安装目的：中央库（用户级真源）或某个项目的真源。
///
/// 两者只差四件事，其余流程（预览、下载、校验、暂存、换目录）完全共用：
/// 1. 目录怎么算：`assets/skills/<id>` vs `<project>/.ccswitchx/skills/<id>`。
/// 2. 备份放哪：`backups/<ms>/skills/…` vs `backups/projects/<pid>/<ms>/canonical/<id>`。
/// 3. 提交用的暂存目录必须**与目的同卷**（`replace_dir` 靠 rename）：项目在另一块盘时
///    用 store 的 staging 会直接失败，所以项目分支把暂存放进真源目录内部。
/// 4. 写不写 `.meta.json`：项目真源**不写**。它随仓库提交，既污染仓库，又会让每次
///    检查更新都改变目录指纹、把全部渲染目标刷成「待更新」。检查状态只存本机缓存。
pub enum SkillSink<'a> {
    Store {
        store: &'a Store,
        home: &'a Path,
    },
    Project {
        store: &'a Store,
        project_root: &'a Path,
        project_id: &'a str,
        timestamp: i64,
    },
}

impl<'a> SkillSink<'a> {
    /// 返回的 `&'a Store` 只绑在 sink 携带的生命周期上，不绑在 `&self` 上：
    /// 这样调用方可以先取 store 用着，再把 sink 本身移动给 `install`。
    pub fn store(&self) -> &'a Store {
        match self {
            SkillSink::Store { store, .. } | SkillSink::Project { store, .. } => store,
        }
    }

    /// 某个 skill id 在目的端的目录；ID 不合法时报错（两侧同一套 ID 安全规则）。
    pub fn entry_dir(&self, id: &str) -> Result<PathBuf, AppError> {
        match self {
            SkillSink::Store { store, .. } => Ok(store.entry_path(AssetType::Skill, id)?),
            SkillSink::Project { project_root, .. } => {
                crate::domain::project::canonical_skill_dir(project_root, id)
            }
        }
    }

    /// 提交用的暂存目录：必须与 `entry_dir` 同卷（`replace_dir` 靠 rename）。
    /// `label` 只用于目录名，方便排查是哪一次写入留下的。
    pub fn staging(&self, label: &str) -> Result<PathBuf, AppError> {
        match self {
            SkillSink::Store { store, .. } => new_staging_dir(store, label),
            SkillSink::Project { project_root, .. } => {
                let root = crate::domain::project::canonical_root(project_root);
                let path = root.join(format!(
                    ".staging-{label}-{}-{}",
                    self.timestamp_of(project_root),
                    std::process::id()
                ));
                fs::create_dir_all(&path)?;
                Ok(path)
            }
        }
    }

    fn timestamp_of(&self, _project_root: &Path) -> i64 {
        match self {
            SkillSink::Store { .. } => 0,
            SkillSink::Project { timestamp, .. } => *timestamp,
        }
    }

    /// 覆盖前的备份；返回实际落地的备份路径（没有可备份的内容时为 None）。
    pub fn backup(&self, id: &str) -> Result<Option<PathBuf>, AppError> {
        match self {
            SkillSink::Store { store, .. } => Ok(store.backup_skill(id)?),
            SkillSink::Project {
                store,
                project_root,
                project_id,
                timestamp,
            } => {
                let source = crate::domain::project::canonical_skill_dir(project_root, id)?;
                if !source.is_dir() {
                    return Ok(None);
                }
                let target = store
                    .root()
                    .join("backups")
                    .join("projects")
                    .join(*project_id)
                    .join(timestamp.to_string())
                    .join("canonical")
                    .join(id);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                copy_dir(&source, &target)?;
                Ok(Some(target))
            }
        }
    }

    /// KeepBoth 时另起一个不冲突的 ID。
    pub fn next_unique_id(&self, base: &str) -> Option<String> {
        (2..=99)
            .map(|suffix| format!("{base}-{suffix}"))
            .find(|candidate| self.entry_dir(candidate).map(|path| !path.exists()).unwrap_or(false))
    }

    /// 是否要在 skill 目录里写 `.meta.json`（见类型文档第 4 点）。
    pub fn writes_meta(&self) -> bool {
        matches!(self, SkillSink::Store { .. })
    }

    /// 写入成功后要做的事：中央库要刷新各 agent 的部署副本，
    /// 项目真源则把渲染留给同步引擎（唯一入口永远是 sync_project）。
    pub fn after_write(&self, id: &str) -> Vec<String> {
        match self {
            SkillSink::Store { store, home } => refresh_deployed(store, home, id),
            SkillSink::Project { .. } => Vec::new(),
        }
    }
}

pub fn install(
    sink: SkillSink<'_>,
    input: &SkillSourceInput,
    staging_id: Option<&str>,
    selections: &[InstallSelection],
    policy: ConflictPolicy,
) -> Result<SkillInstallReport, AppError> {
    if selections.is_empty() {
        return Err(AppError::InvalidInput("没有选择要安装的 Skill".into()));
    }
    let context = resolve_context(input, staging_id, sink.store())?;
    let mut outcomes = Vec::with_capacity(selections.len());
    for (index, selection) in selections.iter().enumerate() {
        outcomes.push(install_one(&sink, &context, selection, policy, index));
    }
    if let Some(staging) = &context.staging {
        let _ = fs::remove_dir_all(staging);
    }
    Ok(SkillInstallReport { outcomes })
}

fn install_one(
    sink: &SkillSink<'_>,
    context: &SourceContext,
    selection: &InstallSelection,
    policy: ConflictPolicy,
    index: usize,
) -> SkillInstallOutcome {
    let name = selection.relative_path.clone();
    let outcome = |status: &str,
                   id: String,
                   backup: Option<PathBuf>,
                   error: Option<String>,
                   warnings: Vec<String>| SkillInstallOutcome {
        id,
        name: name.clone(),
        status: status.to_string(),
        backup_path: backup.map(|path| path.display().to_string()),
        error,
        warnings,
    };

    let id = selection.id.trim().to_string();
    if sink.entry_dir(&id).is_err() {
        return outcome(
            "failed",
            id,
            None,
            Some("Skill ID 不合法".into()),
            Vec::new(),
        );
    }

    let source_dir = match resolve_candidate_dir(&context.root, &selection.relative_path) {
        Ok(path) => path,
        Err(error) => return outcome("failed", id, None, Some(error.to_string()), Vec::new()),
    };

    let target = match sink.entry_dir(&id) {
        Ok(path) => path,
        Err(error) => return outcome("failed", id, None, Some(error.to_string()), Vec::new()),
    };
    let existed = target.exists();
    let final_id = match (existed, policy) {
        (true, ConflictPolicy::Skip) => {
            return outcome("skipped", id, None, None, Vec::new());
        }
        (true, ConflictPolicy::KeepBoth) => match sink.next_unique_id(&id) {
            Some(unique) => unique,
            None => {
                return outcome(
                    "failed",
                    id,
                    None,
                    Some("同名条目过多，无法生成新 ID".into()),
                    Vec::new(),
                )
            }
        },
        _ => id.clone(),
    };
    let target = match sink.entry_dir(&final_id) {
        Ok(path) => path,
        Err(error) => {
            return outcome(
                "failed",
                final_id,
                None,
                Some(error.to_string()),
                Vec::new(),
            )
        }
    };
    let overwriting = target.exists();

    let backup = if overwriting {
        match sink.backup(&final_id) {
            Ok(path) => path,
            Err(error) => {
                return outcome(
                    "failed",
                    final_id,
                    None,
                    Some(format!("覆盖前备份失败：{error}")),
                    Vec::new(),
                )
            }
        }
    } else {
        None
    };

    let staging_root = match sink.staging(&format!("install-{index}")) {
        Ok(path) => path,
        Err(error) => {
            return outcome(
                "failed",
                final_id,
                backup,
                Some(error.to_string()),
                Vec::new(),
            )
        }
    };
    let prepared = staging_root.join("prepared");
    let result = (|| -> Result<(), AppError> {
        copy_dir(&source_dir, &prepared)?;
        if sink.writes_meta() {
            let old_meta = read_meta(&target);
            let mut meta = build_meta(context, selection, old_meta.as_ref(), &prepared)?;
            meta.updated_at = Some(now_ms());
            write_meta(&prepared, &meta)?;
        }
        replace_dir(&target, &prepared, &staging_root)?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            let warnings = sink.after_write(&final_id);
            let _ = fs::remove_dir_all(&staging_root);
            outcome(
                if overwriting {
                    "overwritten"
                } else {
                    "installed"
                },
                final_id,
                backup,
                None,
                warnings,
            )
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&staging_root);
            outcome(
                "failed",
                final_id,
                backup,
                Some(error.to_string()),
                Vec::new(),
            )
        }
    }
}

fn resolve_candidate_dir(root: &Path, relative_path: &str) -> Result<PathBuf, AppError> {
    let relative = Path::new(relative_path);
    if relative_path.is_empty() {
        return Err(AppError::Source("候选路径不能为空".into()));
    }
    if relative_path != "."
        && relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(AppError::Source(format!(
            "候选路径越界，拒绝安装：{relative_path}"
        )));
    }
    let candidate = if relative_path == "." {
        root.to_path_buf()
    } else {
        root.join(relative)
    };
    if !candidate.join("SKILL.md").is_file() {
        return Err(AppError::Source(format!(
            "候选目录里没有 SKILL.md：{relative_path}"
        )));
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|error| AppError::Source(format!("无法读取来源目录：{error}")))?;
    let canonical = candidate
        .canonicalize()
        .map_err(|error| AppError::Source(format!("无法读取候选目录：{error}")))?;
    if !canonical.starts_with(&canonical_root) {
        return Err(AppError::Source("候选目录不在来源范围内".into()));
    }
    Ok(canonical)
}

fn build_meta(
    context: &SourceContext,
    selection: &InstallSelection,
    old: Option<&SkillMeta>,
    prepared: &Path,
) -> Result<SkillMeta, AppError> {
    let now = now_ms();
    let mut meta = SkillMeta::default();
    if let Some(old) = old {
        meta.extra = old.extra.clone();
        meta.installed_at = old.installed_at;
    }
    meta.source_kind = Some(context.kind);
    meta.source_url = context.source_url.clone();
    meta.installed_at = meta.installed_at.or(Some(now));
    meta.updated_at = Some(now);
    meta.content_hash = Some(archive::compute_dir_hash(prepared)?);
    meta.update_available = Some(false);
    meta.last_check_error = None;

    if let Some(repo) = &context.repo {
        let branch = context.branch.clone();
        meta.repo_owner = Some(repo.owner.clone());
        meta.repo_name = Some(repo.name.clone());
        meta.repo_branch = branch.clone();
        meta.repo_path = Some(selection.relative_path.clone());
        meta.readme_url = branch.map(|branch| {
            let suffix = if selection.relative_path == "." {
                String::new()
            } else {
                format!("{}/", selection.relative_path)
            };
            format!(
                "https://github.com/{}/{}/blob/{branch}/{suffix}SKILL.md",
                repo.owner, repo.name
            )
        });
    } else {
        // 从别的来源覆盖时清掉旧的 GitHub 信息，避免继续指向不相关的上游。
        meta.repo_owner = None;
        meta.repo_name = None;
        meta.repo_branch = None;
        meta.repo_path = None;
        meta.readme_url = None;
        meta.latest_hash = None;
        meta.last_checked_at = None;
    }
    Ok(meta)
}

pub(crate) fn new_staging_dir(store: &Store, prefix: &str) -> Result<PathBuf, AppError> {
    let path = store.staging_dir().join(staging_name(prefix));
    fs::create_dir_all(&path)?;
    Ok(path)
}

pub(crate) fn replace_dir(
    target: &Path,
    prepared: &Path,
    staging_root: &Path,
) -> Result<(), AppError> {
    if !target.exists() {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(prepared, target)?;
        return Ok(());
    }
    let trash = staging_root.join("old");
    fs::rename(target, &trash)?;
    match fs::rename(prepared, target) {
        Ok(()) => {
            let _ = fs::remove_dir_all(&trash);
            Ok(())
        }
        Err(error) => {
            // 新目录没换上去，必须把旧目录放回原位。
            let _ = fs::rename(&trash, target);
            Err(error.into())
        }
    }
}

/// 更新/覆盖后刷新已部署的副本；刷新失败只报告，不回滚已经成功的真源更新。
pub(crate) fn refresh_deployed(store: &Store, home: &Path, id: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    for agent in builtin_agents() {
        if !agent.switchable(AssetKind::Skill) {
            continue;
        }
        let matrix = match read_matrix(store, &agent.id) {
            Ok(matrix) => matrix,
            Err(error) => {
                warnings.push(format!(
                    "读取 {} 的启用矩阵失败：{error}",
                    agent.display_name
                ));
                continue;
            }
        };
        if !matrix
            .enabled_for(AssetType::Skill)
            .iter()
            .any(|value| value == id)
        {
            continue;
        }
        let base = match load_state(store, &agent.id) {
            Ok(state) => state.base_override,
            Err(error) => {
                warnings.push(format!(
                    "读取 {} 的本机状态失败：{error}",
                    agent.display_name
                ));
                continue;
            }
        };
        match skills::refresh(store, &agent, id, home, base.as_deref()) {
            Ok(RefreshOutcome::Linked | RefreshOutcome::Copied | RefreshOutcome::Absent) => {}
            Ok(RefreshOutcome::Foreign) => warnings.push(format!(
                "{} 的 skills/{id} 不是本工具部署的，未刷新",
                agent.display_name
            )),
            Err(error) => warnings.push(format!("刷新 {} 的副本失败：{error}", agent.display_name)),
        }
    }
    warnings
}

pub fn uninstall(
    store: &Store,
    home: &Path,
    ids: &[String],
) -> Result<SkillUninstallReport, AppError> {
    let mut outcomes = Vec::with_capacity(ids.len());
    for id in ids {
        outcomes.push(uninstall_one(store, home, id));
    }
    Ok(SkillUninstallReport { outcomes })
}

fn uninstall_one(store: &Store, home: &Path, id: &str) -> SkillUninstallOutcome {
    let mut outcome = SkillUninstallOutcome {
        id: id.to_string(),
        backup_path: None,
        disabled_agents: Vec::new(),
        warnings: Vec::new(),
        error: None,
    };
    let target = match store.entry_path(AssetType::Skill, id) {
        Ok(path) => path,
        Err(error) => {
            outcome.error = Some(error.to_string());
            return outcome;
        }
    };
    if !target.exists() {
        outcome.error = Some("真源里没有这个 Skill".into());
        return outcome;
    }
    match store.backup_skill(id) {
        Ok(Some(path)) => outcome.backup_path = Some(path.display().to_string()),
        Ok(None) => {}
        Err(error) => {
            outcome.error = Some(format!("删除前备份失败，未做任何改动：{error}"));
            return outcome;
        }
    }

    let mut enabled_agents = Vec::new();
    for agent in builtin_agents() {
        if !agent.switchable(AssetKind::Skill) {
            continue;
        }
        let matrix = match read_matrix(store, &agent.id) {
            Ok(matrix) => matrix,
            Err(error) => {
                outcome.error = Some(format!(
                    "读取 {} 的启用矩阵失败，未删除真源：{error}",
                    agent.display_name
                ));
                return outcome;
            }
        };
        if !matrix
            .enabled_for(AssetType::Skill)
            .iter()
            .any(|value| value == id)
        {
            continue;
        }
        let base = match load_state(store, &agent.id) {
            Ok(state) => state.base_override,
            Err(error) => {
                outcome.error = Some(format!(
                    "读取 {} 的本机状态失败，未删除真源：{error}",
                    agent.display_name
                ));
                return outcome;
            }
        };
        match skills::remove(store, &agent, id, home, base.as_deref()) {
            Ok(Removed::Ours(_) | Removed::Absent) => enabled_agents.push(agent.id.clone()),
            Ok(Removed::Foreign) => {
                outcome.warnings.push(format!(
                    "{} 的 skills/{id} 不是本工具部署的，已保留",
                    agent.display_name
                ));
                enabled_agents.push(agent.id.clone());
            }
            Err(error) => {
                outcome.error = Some(format!(
                    "从 {} 撤下失败，未删除真源：{error}",
                    agent.display_name
                ));
                return outcome;
            }
        }
    }

    if let Err(error) = store.delete(AssetType::Skill, id) {
        outcome.error = Some(format!("删除真源失败：{error}"));
        return outcome;
    }
    for agent_id in &enabled_agents {
        if let Err(error) = update_matrix(store, agent_id, |matrix| {
            matrix.disable(AssetType::Skill, id);
        }) {
            outcome
                .warnings
                .push(format!("清理 {agent_id} 的启用记录失败：{error}"));
        }
    }
    outcome.disabled_agents = enabled_agents;
    outcome
}

pub fn set_enabled(
    store: &Store,
    home: &Path,
    ids: &[String],
    agent_id: &str,
    enabled: bool,
) -> Result<SkillBatchReport, AppError> {
    let descriptor: AgentDescriptor = builtin_agents()
        .into_iter()
        .find(|agent| agent.id == agent_id && agent.switchable(AssetKind::Skill))
        .ok_or_else(|| AppError::Unsupported(format!("{agent_id} 的 skill 落点未收录或未实测")))?;
    let base = load_state(store, agent_id)?.base_override;
    if enabled {
        ensure_agent_detected(store, home, agent_id)?;
    }
    let mut report = SkillBatchReport {
        ok: Vec::new(),
        failed: Vec::new(),
    };
    for id in ids {
        let outcome = if enabled {
            skills::deploy(store, &descriptor, id, home, base.as_deref())
        } else {
            skills::remove(store, &descriptor, id, home, base.as_deref()).map(|removed| {
                match removed {
                    Removed::Ours(_) => Deployed::Already,
                    Removed::Absent => Deployed::Already,
                    Removed::Foreign => Deployed::Foreign,
                }
            })
        };
        match outcome {
            Ok(Deployed::Linked(kind)) => report.ok.push(SkillBatchOk {
                id: id.clone(),
                label: format!("{kind:?}").to_lowercase(),
            }),
            Ok(Deployed::Already) => report.ok.push(SkillBatchOk {
                id: id.clone(),
                label: if enabled { "already" } else { "removed" }.to_string(),
            }),
            Ok(Deployed::Foreign) => report.failed.push(SkillItemFailure {
                id: id.clone(),
                error: format!("{agent_id} 的 skills/{id} 不是本工具部署的，未做改动"),
            }),
            Err(error) => report.failed.push(SkillItemFailure {
                id: id.clone(),
                error: error.to_string(),
            }),
        }
    }
    let succeeded = report
        .ok
        .iter()
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    if !succeeded.is_empty() {
        update_matrix(store, agent_id, |matrix| {
            for id in &succeeded {
                if enabled {
                    matrix.enable(AssetType::Skill, id);
                } else {
                    matrix.disable(AssetType::Skill, id);
                }
            }
        })?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::Index;
    use tempfile::tempdir;

    fn fixture() -> (tempfile::TempDir, Store, PathBuf) {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        (dir, store, home)
    }

    fn local_source(root: &Path) -> PathBuf {
        let skill = root.join("source/demo");
        fs::create_dir_all(skill.join("references")).unwrap();
        fs::write(skill.join("SKILL.md"), "---\nname: Demo\n---\nv1\n").unwrap();
        fs::write(skill.join("references/a.md"), "a\n").unwrap();
        skill
    }

    #[test]
    fn preview_and_install_a_local_directory_records_its_source() {
        let (dir, store, home) = fixture();
        let source = local_source(&dir.path().join("incoming"));
        let input = SkillSourceInput::Directory {
            path: source.clone(),
        };
        let preview = preview(&store, &NoFetch, &input).unwrap();
        assert_eq!(preview.kind, SkillSourceKind::Local);
        assert!(preview.staging_id.is_none());
        assert_eq!(preview.candidates[0].relative_path, ".");
        assert_eq!(preview.candidates[0].suggested_id, "demo");

        let report = install(
            SkillSink::Store {
                store: &store,
                home: &home,
            },
            &input,
            None,
            &[InstallSelection {
                relative_path: ".".into(),
                id: "demo".into(),
            }],
            ConflictPolicy::Skip,
        )
        .unwrap();
        assert_eq!(report.outcomes[0].status, "installed");
        let installed = store.asset_dir(AssetType::Skill).join("demo");
        assert!(installed.join("SKILL.md").is_file());
        let meta = read_meta(&installed).unwrap();
        assert_eq!(meta.source_kind, Some(SkillSourceKind::Local));
        assert!(meta
            .source_url
            .as_deref()
            .is_some_and(|url| url.contains("demo")));
        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        index.rebuild(&store).unwrap();
        assert_eq!(index.list(Some(AssetType::Skill)).unwrap().len(), 1);
    }

    #[test]
    fn conflicts_follow_the_policy_and_overwrite_backs_up() {
        let (dir, store, home) = fixture();
        let source = local_source(&dir.path().join("incoming"));
        let input = SkillSourceInput::Directory {
            path: source.clone(),
        };
        let selection = InstallSelection {
            relative_path: ".".into(),
            id: "demo".into(),
        };
        install(
            SkillSink::Store {
                store: &store,
                home: &home,
            },
            &input,
            None,
            std::slice::from_ref(&selection),
            ConflictPolicy::Skip,
        )
        .unwrap();
        std::fs::write(source.join("SKILL.md"), "---\nname: Demo\n---\nv2\n").unwrap();

        let skipped = install(
            SkillSink::Store {
                store: &store,
                home: &home,
            },
            &input,
            None,
            std::slice::from_ref(&selection),
            ConflictPolicy::Skip,
        )
        .unwrap();
        assert_eq!(skipped.outcomes[0].status, "skipped");

        let overwritten = install(
            SkillSink::Store {
                store: &store,
                home: &home,
            },
            &input,
            None,
            std::slice::from_ref(&selection),
            ConflictPolicy::Overwrite,
        )
        .unwrap();
        assert_eq!(overwritten.outcomes[0].status, "overwritten");
        assert!(overwritten.outcomes[0].backup_path.is_some());
        assert_eq!(
            std::fs::read_to_string(store.asset_dir(AssetType::Skill).join("demo/SKILL.md"))
                .unwrap(),
            "---\nname: Demo\n---\nv2\n"
        );

        let both = install(
            SkillSink::Store {
                store: &store,
                home: &home,
            },
            &input,
            None,
            std::slice::from_ref(&selection),
            ConflictPolicy::KeepBoth,
        )
        .unwrap();
        assert_eq!(both.outcomes[0].status, "installed");
        assert_eq!(both.outcomes[0].id, "demo-2");
    }

    #[test]
    fn install_rejects_paths_outside_the_source() {
        let (dir, store, home) = fixture();
        let source = local_source(&dir.path().join("incoming"));
        let input = SkillSourceInput::Directory {
            path: source.clone(),
        };
        let report = install(
            SkillSink::Store {
                store: &store,
                home: &home,
            },
            &input,
            None,
            &[InstallSelection {
                relative_path: "../../etc".into(),
                id: "evil".into(),
            }],
            ConflictPolicy::Skip,
        )
        .unwrap();
        assert_eq!(report.outcomes[0].status, "failed");
        assert!(!store.asset_dir(AssetType::Skill).join("evil").exists());
    }

    #[test]
    fn uninstall_backs_up_removes_deployment_and_clears_matrix() {
        let (dir, store, home) = fixture();
        let source = local_source(&dir.path().join("incoming"));
        let input = SkillSourceInput::Directory { path: source };
        install(
            SkillSink::Store {
                store: &store,
                home: &home,
            },
            &input,
            None,
            &[InstallSelection {
                relative_path: ".".into(),
                id: "demo".into(),
            }],
            ConflictPolicy::Skip,
        )
        .unwrap();
        let deployed = skills::deploy(
            &store,
            &builtin_agents()
                .into_iter()
                .find(|agent| agent.id == "claude")
                .unwrap(),
            "demo",
            &home,
            None,
        )
        .unwrap();
        assert!(matches!(deployed, Deployed::Linked(_)));
        update_matrix(&store, "claude", |matrix| {
            matrix.enable(AssetType::Skill, "demo");
        })
        .unwrap();

        let report = uninstall(&store, &home, &["demo".to_string()]).unwrap();
        assert!(
            report.outcomes[0].error.is_none(),
            "{:?}",
            report.outcomes[0]
        );
        assert!(report.outcomes[0].backup_path.is_some());
        assert!(!store.asset_dir(AssetType::Skill).join("demo").exists());
        assert!(!home.join(".claude/skills/demo").exists());
        let matrix = read_matrix(&store, "claude").unwrap();
        assert!(matrix.enabled_for(AssetType::Skill).is_empty());
        let _ = dir;
    }

    #[test]
    fn batch_enable_reports_foreign_directories_without_touching_them() {
        let (dir, store, home) = fixture();
        let source = local_source(&dir.path().join("incoming"));
        let input = SkillSourceInput::Directory { path: source };
        install(
            SkillSink::Store {
                store: &store,
                home: &home,
            },
            &input,
            None,
            &[InstallSelection {
                relative_path: ".".into(),
                id: "demo".into(),
            }],
            ConflictPolicy::Skip,
        )
        .unwrap();
        let foreign = home.join(".claude/skills/demo");
        fs::create_dir_all(&foreign).unwrap();
        fs::write(foreign.join("SKILL.md"), "别人的\n").unwrap();

        let report = set_enabled(&store, &home, &["demo".into()], "claude", true).unwrap();
        assert!(report.ok.is_empty());
        assert_eq!(report.failed.len(), 1);
        assert_eq!(
            std::fs::read_to_string(foreign.join("SKILL.md")).unwrap(),
            "别人的\n"
        );
    }

    struct NoFetch;

    impl RepoFetcher for NoFetch {
        fn fetch(&self, _repo: &RepoRef) -> Result<crate::net::FetchedRepo, AppError> {
            Err(AppError::Network("测试不触网".into()))
        }
    }

    struct FakeFetcher {
        fixture: PathBuf,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl FakeFetcher {
        fn new(fixture: PathBuf) -> Self {
            Self {
                fixture,
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    impl RepoFetcher for FakeFetcher {
        fn fetch(&self, _repo: &RepoRef) -> Result<crate::net::FetchedRepo, AppError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let dir = tempfile::tempdir()?;
            copy_dir(&self.fixture, dir.path())?;
            Ok(crate::net::FetchedRepo {
                dir,
                branch: "main".into(),
            })
        }
    }

    fn github_fixture(root: &Path) -> PathBuf {
        let repo = root.join("repo");
        let skill = repo.join("skills/demo");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "---\nname: Demo\n---\n上游\n").unwrap();
        repo
    }

    #[test]
    fn github_preview_and_install_record_repo_branch_and_staging() {
        let (dir, store, home) = fixture();
        let fetcher = FakeFetcher::new(github_fixture(&dir.path().join("remote")));
        let input = SkillSourceInput::Github {
            repo: "acme/demo".into(),
            branch: None,
        };

        let preview = preview(&store, &fetcher, &input).unwrap();
        assert_eq!(preview.kind, SkillSourceKind::Github);
        assert_eq!(preview.repo.as_deref(), Some("acme/demo"));
        assert_eq!(preview.branch.as_deref(), Some("main"));
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(preview.candidates[0].relative_path, "skills/demo");
        let staging_id = preview.staging_id.clone().unwrap();
        assert!(store.staging_dir().join(&staging_id).is_dir());

        let report = install(
            SkillSink::Store {
                store: &store,
                home: &home,
            },
            &input,
            Some(&staging_id),
            &[InstallSelection {
                relative_path: "skills/demo".into(),
                id: preview.candidates[0].suggested_id.clone(),
            }],
            ConflictPolicy::Skip,
        )
        .unwrap();
        assert_eq!(report.outcomes[0].status, "installed");
        let installed = store
            .asset_dir(AssetType::Skill)
            .join(&report.outcomes[0].id);
        let meta = read_meta(&installed).unwrap();
        assert_eq!(meta.source_kind, Some(SkillSourceKind::Github));
        assert_eq!(meta.repo_owner.as_deref(), Some("acme"));
        assert_eq!(meta.repo_branch.as_deref(), Some("main"));
        assert_eq!(meta.repo_path.as_deref(), Some("skills/demo"));
        assert!(meta
            .readme_url
            .as_deref()
            .unwrap()
            .contains("/blob/main/skills/demo/SKILL.md"));
        assert!(
            !store.staging_dir().join(&staging_id).exists(),
            "安装后清掉暂存"
        );
    }

    #[test]
    fn discovered_install_locates_candidate_by_directory() {
        let (dir, store, home) = fixture();
        let fetcher = FakeFetcher::new(github_fixture(&dir.path().join("remote")));
        let outcome = install_discovered(
            SkillSink::Store {
                store: &store,
                home: &home,
            },
            &fetcher,
            "acme/demo",
            None,
            None,
            Some("demo"),
            None,
            ConflictPolicy::Skip,
        )
        .unwrap();
        assert_eq!(outcome.status, "installed");
        assert!(store
            .asset_dir(AssetType::Skill)
            .join(&outcome.id)
            .join("SKILL.md")
            .is_file());
    }

    #[test]
    fn install_report_always_serializes_empty_arrays() {
        let report = SkillInstallReport {
            outcomes: vec![SkillInstallOutcome {
                id: "demo".into(),
                name: "demo".into(),
                status: "installed".into(),
                backup_path: None,
                error: None,
                warnings: Vec::new(),
            }],
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(
            json.contains("\"warnings\":[]"),
            "空 warnings 不能省略，前端按数组渲染：{json}"
        );
    }

    #[test]
    fn zip_preview_and_install_keep_multiple_candidates() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;

        let (dir, store, home) = fixture();
        let zip_path = dir.path().join("skills.zip");
        let file = fs::File::create(&zip_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, body) in [
            ("alpha/SKILL.md", "---\nname: Alpha\n---\na\n"),
            ("beta/SKILL.md", "---\nname: Beta\n---\nb\n"),
        ] {
            writer.start_file(name, options).unwrap();
            writer.write_all(body.as_bytes()).unwrap();
        }
        writer.finish().unwrap();

        let input = SkillSourceInput::Zip {
            path: zip_path.clone(),
        };
        let preview = preview(&store, &NoFetch, &input).unwrap();
        assert_eq!(preview.kind, SkillSourceKind::Zip);
        assert_eq!(preview.candidates.len(), 2);
        assert_eq!(preview.candidates[0].relative_path, "alpha");
        assert_eq!(preview.candidates[1].suggested_id, "beta");

        let report = install(
            SkillSink::Store {
                store: &store,
                home: &home,
            },
            &input,
            preview.staging_id.as_deref(),
            &[InstallSelection {
                relative_path: "beta".into(),
                id: "beta".into(),
            }],
            ConflictPolicy::Skip,
        )
        .unwrap();
        assert_eq!(report.outcomes[0].status, "installed");
        let meta = read_meta(&store.asset_dir(AssetType::Skill).join("beta")).unwrap();
        assert_eq!(meta.source_kind, Some(SkillSourceKind::Zip));
        assert!(meta.source_url.as_deref().unwrap().contains("skills.zip"));
        assert!(meta.repo_owner.is_none());
    }

    #[test]
    fn pick_candidate_prefers_path_then_directory_then_unique_name() {
        let candidate = |relative: &str, name: &str, id: &str| SkillCandidate {
            relative_path: relative.into(),
            name: name.into(),
            description: None,
            suggested_id: id.into(),
            exists: false,
        };
        let candidates = vec![
            candidate("skills/pdf", "PDF", "acme-demo-skills-pdf"),
            candidate("writing", "Writing", "acme-demo-writing"),
        ];
        assert_eq!(
            pick_candidate(&candidates, Some("writing"), None, None)
                .unwrap()
                .relative_path,
            "writing"
        );
        assert_eq!(
            pick_candidate(&candidates, None, Some("pdf"), None)
                .unwrap()
                .relative_path,
            "skills/pdf"
        );
        assert_eq!(
            pick_candidate(&candidates, None, Some("acme-demo-writing"), None)
                .unwrap()
                .relative_path,
            "writing"
        );
        assert_eq!(
            pick_candidate(&candidates, None, None, Some("pdf"))
                .unwrap()
                .relative_path,
            "skills/pdf"
        );
        assert!(pick_candidate(&candidates, Some("nope"), Some("nope"), Some("nope")).is_none());
        // 只有一个候选时允许无提示定位（skills.sh 的 skillId 与目录名不一致时兜底）。
        assert_eq!(
            pick_candidate(&candidates[..1], None, None, None)
                .unwrap()
                .relative_path,
            "skills/pdf"
        );
    }
}
