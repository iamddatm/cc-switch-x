//! 从上游 GitHub 仓库检查并更新已安装的 Skill。
//!
//! 同一个仓库在一次批量里只下载一次；本地哈希现场计算，不信任缓存，
//! 因此用户手改过 `SKILL.md` 也会被识别为“与上游不同”。
//!
//! 目的端有两种（`UpdateSink`），差别只在**来源与检查状态存在哪**：
//! - 中央库：`.meta.json` 就写在 skill 目录里，更新完当场刷新各 agent 的部署副本。
//! - 项目真源：来源取台账 `skills.json` 的 `source`，检查状态只写本机缓存
//!   （`.meta.json` 不能进仓库：它污染提交内容，还会让每次检查都改变目录指纹、
//!   把全部渲染目标刷成「待更新」），铺开交给同步引擎，更新本身不写渲染产物。

use crate::domain::project::{ProjectLedger, ProjectPaths};
use crate::domain::skill::parse_metadata;
use crate::domain::skill_meta::{read_meta, write_meta, SkillMeta};
use crate::domain::AssetType;
use crate::error::AppError;
use crate::net::archive::{self, ScannedSkill};
use crate::net::{parse_github_input, RepoFetcher, RepoRef};
use crate::services::project_scan;
use crate::services::skill_source::{replace_dir, SkillItemFailure, SkillSink};
use crate::store::project_checks::{ProjectCheckCache, ProjectSkillCheck};
use crate::store::{copy_dir, Store};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillUpdateInfo {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_hash: Option<String>,
    pub remote_hash: String,
    pub missing_local: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillSkipped {
    pub id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepoFailure {
    pub repo: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillCheckReport {
    pub checked_at: i64,
    pub updates: Vec<SkillUpdateInfo>,
    pub up_to_date: Vec<String>,
    pub skipped: Vec<SkillSkipped>,
    pub repo_failures: Vec<SkillRepoFailure>,
    pub skill_failures: Vec<SkillItemFailure>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillUpdateOutcome {
    pub id: String,
    pub name: String,
    /// updated / upToDate / failed
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
    /// 空数组也必须序列化，前端按数组渲染。
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillUpdateReport {
    pub outcomes: Vec<SkillUpdateOutcome>,
}

struct Target {
    id: String,
    name: String,
    dir: PathBuf,
    provenance: Provenance,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GroupKey {
    owner: String,
    name: String,
    branch: Option<String>,
}

/// 一个 skill 的上游来源。两个分支各自只有一处真源，这里只做统一读取：
/// 中央库读 skill 目录里的 `.meta.json`，项目真源读台账的 `source`。
#[derive(Debug, Clone)]
pub enum Provenance {
    /// 原样持有 `.meta.json`：写回时要保留 `installed_at` 与未知键，不能被投影磨平。
    Meta(SkillMeta),
    Project {
        repo: RepoRef,
        /// 上游仓库里的相对目录（台账 `source.subpath`）。
        subpath: Option<String>,
    },
}

impl Provenance {
    fn owner(&self) -> Option<&str> {
        match self {
            Provenance::Meta(meta) => non_empty(meta.repo_owner.as_deref()),
            Provenance::Project { repo, .. } => Some(repo.owner.as_str()),
        }
    }

    fn name(&self) -> Option<&str> {
        match self {
            Provenance::Meta(meta) => non_empty(meta.repo_name.as_deref()),
            Provenance::Project { repo, .. } => Some(repo.name.as_str()),
        }
    }

    fn branch(&self) -> Option<&str> {
        match self {
            Provenance::Meta(meta) => non_empty(meta.repo_branch.as_deref()),
            Provenance::Project { repo, .. } => non_empty(repo.branch.as_deref()),
        }
    }

    /// 上次定位到的上游目录；项目分支就是台账里的 `subpath`。
    fn repo_path(&self) -> Option<&str> {
        match self {
            Provenance::Meta(meta) => meta.repo_path.as_deref(),
            Provenance::Project { subpath, .. } => subpath.as_deref(),
        }
    }

    /// cc-switch 导入留下的来源目录名；项目真源没有这个概念。
    fn source_directory(&self) -> Option<&str> {
        match self {
            Provenance::Meta(meta) => meta.source_directory.as_deref(),
            Provenance::Project { .. } => None,
        }
    }

    fn readme_url(&self) -> Option<&str> {
        match self {
            Provenance::Meta(meta) => meta.readme_url.as_deref(),
            Provenance::Project { .. } => None,
        }
    }

    /// 把这次检查的结果折成新的 `.meta.json`；项目分支没有 meta 文件，返回 None。
    fn checked_meta(
        &self,
        branch: Option<&str>,
        repo_path: Option<&str>,
        latest_hash: Option<&str>,
        // None 表示这次没拿到远端结果，保留上一次的可更新状态。
        update_available: Option<bool>,
        error: Option<&str>,
    ) -> Option<SkillMeta> {
        let Provenance::Meta(meta) = self else {
            return None;
        };
        let mut meta = meta.clone();
        if let Some(branch) = branch.filter(|value| !value.trim().is_empty()) {
            meta.repo_branch = Some(branch.to_string());
        }
        if let Some(path) = repo_path {
            meta.repo_path = Some(path.to_string());
        }
        meta.last_checked_at = Some(now_ms());
        if latest_hash.is_some() {
            meta.latest_hash = latest_hash.map(str::to_string);
        }
        if let Some(available) = update_available {
            meta.update_available = Some(available);
        }
        meta.last_check_error = error.map(str::to_string);
        Some(meta)
    }

    /// 更新成功后的 `.meta.json`：来源与内容指纹一起刷，安装时间与未知键保持不动。
    fn updated_meta(&self, branch: &str, repo_path: &str, remote_hash: &str) -> Option<SkillMeta> {
        let Provenance::Meta(meta) = self else {
            return None;
        };
        let now = now_ms();
        let mut meta = meta.clone();
        meta.repo_branch = Some(branch.to_string());
        meta.repo_path = Some(repo_path.to_string());
        meta.content_hash = Some(remote_hash.to_string());
        meta.updated_at = Some(now);
        meta.last_checked_at = Some(now);
        meta.latest_hash = Some(remote_hash.to_string());
        meta.update_available = Some(false);
        meta.last_check_error = None;
        if meta.readme_url.is_none() && meta.is_updatable() {
            let owner = meta.repo_owner.as_deref().unwrap_or_default();
            let name = meta.repo_name.as_deref().unwrap_or_default();
            let suffix = if repo_path == "." {
                String::new()
            } else {
                format!("{repo_path}/")
            };
            meta.readme_url = Some(format!(
                "https://github.com/{owner}/{name}/blob/{branch}/{suffix}SKILL.md"
            ));
        }
        Some(meta)
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}

/// 更新目的端：中央库（用户级真源）或某个项目的真源。
///
/// 下载、定位上游目录、算哈希、备份、换目录全部共用；差别只在来源与检查状态的存法
/// （`Provenance` + `record_check`）、更新后谁铺开（`SkillSink::after_write`），
/// 以及暂存/备份的落点（复用 `SkillSink`，两侧都在 store 卷上）。
pub enum UpdateSink<'a> {
    Store {
        store: &'a Store,
        home: &'a Path,
    },
    Project {
        store: &'a Store,
        paths: &'a ProjectPaths,
        /// 同一批更新共用一个时间戳，备份才会落在同一个目录里。
        timestamp: i64,
        /// 来源真源：读 `source` 定位上游，更新后刷 `source`。
        ledger: &'a mut ProjectLedger,
        /// 易变的检查状态：只落本机 `local/`，不进台账。
        cache: &'a mut ProjectCheckCache,
    },
}

impl<'a> UpdateSink<'a> {
    /// 目录、备份、暂存、铺开规则都来自 `SkillSink`，这里不再抄一遍。
    fn dest(&self) -> SkillSink<'_> {
        match self {
            UpdateSink::Store { store, home } => SkillSink::Store { store, home },
            UpdateSink::Project {
                store,
                paths,
                timestamp,
                ..
            } => SkillSink::Project {
                store,
                paths,
                timestamp: *timestamp,
            },
        }
    }

    fn load(&self, ids: Option<&[String]>) -> Result<LoadReport, AppError> {
        match self {
            UpdateSink::Store { store, .. } => load_store_targets(store, ids),
            UpdateSink::Project { paths, ledger, .. } => load_project_targets(ledger, paths, ids),
        }
    }

    /// 记下这次检查：中央库写 `.meta.json`，项目只改内存里的本机缓存。
    fn record_check(
        &mut self,
        target: &Target,
        branch: Option<&str>,
        repo_path: Option<&str>,
        latest_hash: Option<&str>,
        update_available: Option<bool>,
        error: Option<&str>,
    ) -> Result<(), AppError> {
        match self {
            UpdateSink::Store { .. } => {
                if let Some(meta) =
                    target
                        .provenance
                        .checked_meta(branch, repo_path, latest_hash, update_available, error)
                {
                    write_meta(&target.dir, &meta)?;
                }
                Ok(())
            }
            // 这次探测到的分支与上游路径**不**回填台账：一次只读的「检查更新」
            // 不该在台账里留下无谓的写入。真更新时才由 after_update 回写。
            UpdateSink::Project { cache, .. } => {
                Self::merge_check(cache, &target.id, latest_hash, update_available, error);
                Ok(())
            }
        }
    }

    /// 合并本机缓存里的检查状态：`latest_hash`/`update_available` 为 None 表示这次没拿到
    /// 远端结果，保留上一次的判断；错误则总是覆盖（与 `.meta.json` 的规则一致）。
    fn merge_check(
        cache: &mut ProjectCheckCache,
        id: &str,
        latest_hash: Option<&str>,
        update_available: Option<bool>,
        error: Option<&str>,
    ) {
        let mut record: ProjectSkillCheck = cache.get(id).cloned().unwrap_or_default();
        record.updatable = true;
        record.last_checked_at = Some(now_ms());
        if latest_hash.is_some() {
            record.latest_hash = latest_hash.map(str::to_string);
        }
        if let Some(available) = update_available {
            record.update_available = available;
        }
        record.last_check_error = error.map(str::to_string);
        cache.record(id, record);
    }

    /// 更新成功后的状态回写。中央库的来源与指纹已经写进 `.meta.json`（在换目录之前），
    /// 这里无事可做；项目的来源在台账、检查状态在本机缓存，两处都得刷。
    fn after_update(&mut self, target: &Target, branch: &str, repo_path: &str, remote_hash: &str) {
        let UpdateSink::Project { ledger, cache, .. } = self else {
            return;
        };
        if let Some(source) = ledger
            .skills
            .get_mut(&target.id)
            .and_then(|entry| entry.source.as_mut())
        {
            source.branch = Some(branch.to_string());
            source.subpath = Some(repo_path.to_string());
            source.tree_hash = Some(remote_hash.to_string());
        }
        // 真源基线 entry.canonical 保持不动：那正是「目标格子等待更新」的判据，
        // 刷了就等于替同步引擎宣布已经铺开完毕。
        ledger.updated_at = chrono::Utc::now().to_rfc3339();
        Self::merge_check(cache, &target.id, Some(remote_hash), Some(false), None);
    }

    fn flush_checks(&mut self) -> Result<(), AppError> {
        match self {
            UpdateSink::Store { .. } => Ok(()),
            UpdateSink::Project {
                store,
                paths,
                cache,
                ..
            } => cache.save(store, &paths.id),
        }
    }

    fn flush_ledger(&mut self) -> Result<(), AppError> {
        match self {
            UpdateSink::Store { .. } => Ok(()),
            UpdateSink::Project { paths, ledger, .. } => project_scan::save_ledger(paths, ledger),
        }
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn display_name(dir: &Path, id: &str) -> String {
    fs::read_to_string(dir.join("SKILL.md"))
        .ok()
        .and_then(|text| parse_metadata(&text).name)
        .unwrap_or_else(|| id.to_string())
}

type LoadReport = (Vec<Target>, Vec<SkillSkipped>, Vec<SkillItemFailure>);

fn load_store_targets(store: &Store, ids: Option<&[String]>) -> Result<LoadReport, AppError> {
    let all = store.list_ids(AssetType::Skill)?;
    let requested: Option<BTreeSet<&str>> =
        ids.map(|values| values.iter().map(String::as_str).collect());
    let mut targets = Vec::new();
    let mut skipped = Vec::new();
    let mut failures = Vec::new();

    if let Some(requested) = &requested {
        for id in requested.iter() {
            if !all.iter().any(|existing| existing == id) {
                failures.push(SkillItemFailure {
                    id: (*id).to_string(),
                    error: "真源里没有这个 Skill".into(),
                });
            }
        }
    }

    for id in all {
        if requested
            .as_ref()
            .is_some_and(|requested| !requested.contains(id.as_str()))
        {
            continue;
        }
        let dir = store.entry_path(AssetType::Skill, &id)?;
        let Some(meta) = read_meta(&dir) else {
            skipped.push(SkillSkipped {
                id,
                reason: "没有来源元数据".into(),
            });
            continue;
        };
        if !meta.is_updatable() {
            skipped.push(SkillSkipped {
                id,
                reason: "没有上游 GitHub 仓库".into(),
            });
            continue;
        }
        let raw_repo = format!(
            "{}/{}",
            meta.repo_owner.as_deref().unwrap_or_default(),
            meta.repo_name.as_deref().unwrap_or_default()
        );
        if let Err(error) = parse_github_input(&raw_repo, meta.repo_branch.clone()) {
            failures.push(SkillItemFailure {
                id,
                error: format!("来源仓库非法：{error}"),
            });
            continue;
        }
        let name = display_name(&dir, &id);
        targets.push(Target {
            id,
            name,
            dir,
            provenance: Provenance::Meta(meta),
        });
    }
    targets.sort_by(|left, right| left.id.cmp(&right.id));
    Ok((targets, skipped, failures))
}

/// 项目分支的候选：来源在台账 `source` 里，目录必须是磁盘上真实存在的真源。
/// 台账记着有、磁盘上却没有（未迁移或没「收进真源」）只能跳过 —— 凭空建目录再写，
/// 等于替用户决定这个项目该有什么。
fn load_project_targets(
    ledger: &ProjectLedger,
    paths: &ProjectPaths,
    ids: Option<&[String]>,
) -> Result<LoadReport, AppError> {
    let requested: Option<BTreeSet<&str>> =
        ids.map(|values| values.iter().map(String::as_str).collect());
    let mut targets = Vec::new();
    let mut skipped = Vec::new();
    let mut failures = Vec::new();

    if let Some(requested) = &requested {
        for id in requested.iter() {
            if !ledger.skills.contains_key(*id) {
                failures.push(SkillItemFailure {
                    id: (*id).to_string(),
                    error: "项目台账里没有这个 Skill".into(),
                });
            }
        }
    }

    for id in ledger.skills.keys() {
        if requested
            .as_ref()
            .is_some_and(|requested| !requested.contains(id.as_str()))
        {
            continue;
        }
        let dir = match paths.canonical_skill_dir(id) {
            Ok(path) => path,
            Err(error) => {
                failures.push(SkillItemFailure {
                    id: id.clone(),
                    error: error.to_string(),
                });
                continue;
            }
        };
        let entry = &ledger.skills[id];
        let Some(source) = entry
            .source
            .as_ref()
            .filter(|source| source.kind == "github")
        else {
            skipped.push(SkillSkipped {
                id: id.clone(),
                reason: "来源不是 GitHub，没有可更新的上游".into(),
            });
            continue;
        };
        if !dir.join("SKILL.md").is_file() {
            skipped.push(SkillSkipped {
                id: id.clone(),
                reason: "项目真源还不存在（未迁移或没收进真源）".into(),
            });
            continue;
        }
        let raw_repo = source.repo.clone().unwrap_or_default();
        let repo = match parse_github_input(&raw_repo, source.branch.clone()) {
            Ok(repo) => repo,
            Err(error) => {
                failures.push(SkillItemFailure {
                    id: id.clone(),
                    error: format!("来源仓库非法：{error}"),
                });
                continue;
            }
        };
        let name = display_name(&dir, id);
        targets.push(Target {
            id: id.clone(),
            name,
            dir,
            provenance: Provenance::Project {
                repo,
                subpath: source.subpath.clone(),
            },
        });
    }
    targets.sort_by(|left, right| left.id.cmp(&right.id));
    Ok((targets, skipped, failures))
}

fn group_targets(targets: Vec<Target>) -> BTreeMap<GroupKey, Vec<Target>> {
    let mut groups = BTreeMap::new();
    for target in targets {
        let Some(owner) = target.provenance.owner().map(str::to_string) else {
            continue;
        };
        let Some(name) = target.provenance.name().map(str::to_string) else {
            continue;
        };
        let branch = target.provenance.branch().map(str::to_string);
        groups
            .entry(GroupKey {
                owner,
                name,
                branch,
            })
            .or_insert_with(Vec::new)
            .push(target);
    }
    groups
}

fn repo_of(key: &GroupKey) -> RepoRef {
    RepoRef {
        owner: key.owner.clone(),
        name: key.name.clone(),
        branch: key.branch.clone(),
    }
}

fn scan_repo(root: &Path) -> Result<Vec<ScannedSkill>, AppError> {
    archive::scan_skills(root, 2_000)
}

fn remote_dir(root: &Path, relative_path: &str) -> PathBuf {
    if relative_path == "." {
        root.to_path_buf()
    } else {
        root.join(relative_path)
    }
}

fn normalize_path(value: &str) -> String {
    value
        .trim()
        .trim_matches('/')
        .trim_end_matches("/SKILL.md")
        .trim_end_matches('/')
        .to_string()
}

/// 远端定位顺序：显式 repoPath → readme 路径 → 目录名 basename → frontmatter 名称 → 唯一候选。
fn match_remote<'a>(
    scanned: &'a [ScannedSkill],
    provenance: &Provenance,
    local_dir: &Path,
    display_name: &str,
) -> Result<&'a ScannedSkill, String> {
    if let Some(path) = provenance.repo_path().map(normalize_path) {
        if let Some(found) = scanned
            .iter()
            .find(|candidate| candidate.relative_path == path)
        {
            return Ok(found);
        }
    }
    if let Some(url) = provenance.readme_url() {
        let matches = scanned
            .iter()
            .filter(|candidate| readme_matches(url, &candidate.relative_path))
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            return Ok(matches[0]);
        }
    }
    if let Some(directory) = provenance.source_directory().map(normalize_path) {
        if let Some(found) = scanned
            .iter()
            .find(|candidate| candidate.relative_path == directory)
        {
            return Ok(found);
        }
        let basename = directory.rsplit('/').next().unwrap_or(&directory);
        let matches = scanned
            .iter()
            .filter(|candidate| candidate_basename(&candidate.relative_path) == basename)
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            return Ok(matches[0]);
        }
    }
    let local_dir_name = local_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let matches = scanned
        .iter()
        .filter(|candidate| candidate_basename(&candidate.relative_path) == local_dir_name)
        .collect::<Vec<_>>();
    if matches.len() == 1 {
        return Ok(matches[0]);
    }
    let matches = scanned
        .iter()
        .filter(|candidate| candidate.name == display_name)
        .collect::<Vec<_>>();
    if !display_name.is_empty() && matches.len() == 1 {
        return Ok(matches[0]);
    }
    if scanned.len() == 1 {
        return Ok(&scanned[0]);
    }
    Err(format!(
        "无法在上游仓库里唯一定位（候选 {} 个）",
        scanned.len()
    ))
}

fn candidate_basename(relative_path: &str) -> &str {
    if relative_path == "." {
        ""
    } else {
        relative_path.rsplit('/').next().unwrap_or(relative_path)
    }
}

fn readme_matches(url: &str, relative_path: &str) -> bool {
    if relative_path == "." {
        url.ends_with("/SKILL.md")
    } else {
        url.ends_with(&format!("/{relative_path}/SKILL.md"))
    }
}

pub fn check<F: RepoFetcher>(
    mut sink: UpdateSink<'_>,
    fetcher: &F,
    ids: Option<&[String]>,
) -> Result<SkillCheckReport, AppError> {
    let (targets, skipped, mut skill_failures) = sink.load(ids)?;
    let mut report = SkillCheckReport {
        checked_at: now_ms(),
        updates: Vec::new(),
        up_to_date: Vec::new(),
        skipped,
        repo_failures: Vec::new(),
        skill_failures: Vec::new(),
    };

    for (key, group) in group_targets(targets) {
        let repo = repo_of(&key);
        let fetched = match fetcher.fetch(&repo) {
            Ok(fetched) => fetched,
            Err(error) => {
                let message = error.to_string();
                report.repo_failures.push(SkillRepoFailure {
                    repo: format!("{}/{}", key.owner, key.name),
                    branch: key.branch.clone(),
                    error: message.clone(),
                });
                for target in &group {
                    if let Err(error) =
                        sink.record_check(target, None, None, None, None, Some(&message))
                    {
                        report.skill_failures.push(SkillItemFailure {
                            id: target.id.clone(),
                            error: format!("写入检查状态失败：{error}"),
                        });
                    }
                }
                continue;
            }
        };
        let scanned = match scan_repo(fetched.dir.path()) {
            Ok(scanned) => scanned,
            Err(error) => {
                let message = error.to_string();
                report.repo_failures.push(SkillRepoFailure {
                    repo: format!("{}/{}", key.owner, key.name),
                    branch: Some(fetched.branch.clone()),
                    error: message.clone(),
                });
                for target in &group {
                    if let Err(error) = sink.record_check(
                        target,
                        Some(&fetched.branch),
                        None,
                        None,
                        None,
                        Some(&message),
                    ) {
                        report.skill_failures.push(SkillItemFailure {
                            id: target.id.clone(),
                            error: format!("写入检查状态失败：{error}"),
                        });
                    }
                }
                continue;
            }
        };

        for target in group {
            let matched = match match_remote(&scanned, &target.provenance, &target.dir, &target.name)
            {
                Ok(found) => found,
                Err(error) => {
                    skill_failures.push(SkillItemFailure {
                        id: target.id.clone(),
                        error: error.clone(),
                    });
                    if let Err(write_error) = sink.record_check(
                        &target,
                        Some(&fetched.branch),
                        None,
                        None,
                        None,
                        Some(&error),
                    ) {
                        skill_failures.push(SkillItemFailure {
                            id: target.id.clone(),
                            error: format!("写入检查状态失败：{write_error}"),
                        });
                    }
                    continue;
                }
            };
            let remote_hash = match archive::compute_dir_hash(&remote_dir(
                fetched.dir.path(),
                &matched.relative_path,
            )) {
                Ok(hash) => hash,
                Err(error) => {
                    skill_failures.push(SkillItemFailure {
                        id: target.id.clone(),
                        error: error.to_string(),
                    });
                    if let Err(write_error) = sink.record_check(
                        &target,
                        Some(&fetched.branch),
                        Some(&matched.relative_path),
                        None,
                        None,
                        Some(&error.to_string()),
                    ) {
                        skill_failures.push(SkillItemFailure {
                            id: target.id.clone(),
                            error: format!("写入检查状态失败：{write_error}"),
                        });
                    }
                    continue;
                }
            };
            let local_hash = archive::compute_dir_hash(&target.dir).ok();
            let missing_local = local_hash.is_none();
            let available = local_hash.as_deref() != Some(remote_hash.as_str());
            if let Err(error) = sink.record_check(
                &target,
                Some(&fetched.branch),
                Some(&matched.relative_path),
                Some(&remote_hash),
                Some(available),
                None,
            ) {
                skill_failures.push(SkillItemFailure {
                    id: target.id.clone(),
                    error: format!("写入检查结果失败：{error}"),
                });
            }
            if available {
                report.updates.push(SkillUpdateInfo {
                    id: target.id.clone(),
                    name: target.name.clone(),
                    current_hash: local_hash,
                    remote_hash,
                    missing_local,
                });
            } else {
                report.up_to_date.push(target.id.clone());
            }
        }
    }

    report.skill_failures = skill_failures;
    // 中央库分支已经逐条写进 .meta.json；项目分支的检查状态全在内存里，这里落盘。
    sink.flush_checks()?;
    Ok(report)
}

pub fn update<F: RepoFetcher>(
    mut sink: UpdateSink<'_>,
    fetcher: &F,
    ids: &[String],
) -> Result<SkillUpdateReport, AppError> {
    let (targets, skipped, mut failures) = sink.load(Some(ids))?;
    let mut outcomes = Vec::new();
    for item in skipped {
        outcomes.push(SkillUpdateOutcome {
            id: item.id.clone(),
            name: item.id,
            status: "failed".into(),
            backup_path: None,
            warnings: Vec::new(),
            error: Some(format!("没有可更新的上游来源：{}", item.reason)),
        });
    }
    for item in failures.drain(..) {
        outcomes.push(SkillUpdateOutcome {
            id: item.id.clone(),
            name: item.id,
            status: "failed".into(),
            backup_path: None,
            warnings: Vec::new(),
            error: Some(item.error),
        });
    }

    for (key, group) in group_targets(targets) {
        let repo = repo_of(&key);
        let fetched = match fetcher.fetch(&repo) {
            Ok(fetched) => fetched,
            Err(error) => {
                let message = error.to_string();
                for target in group {
                    let suffix = sink
                        .record_check(&target, None, None, None, None, Some(&message))
                        .err()
                        .map(|error| format!("；写入检查状态失败：{error}"))
                        .unwrap_or_default();
                    outcomes.push(failed(&target, format!("{message}{suffix}"), None));
                }
                continue;
            }
        };
        let scanned = match scan_repo(fetched.dir.path()) {
            Ok(scanned) => scanned,
            Err(error) => {
                let message = error.to_string();
                for target in group {
                    let suffix = sink
                        .record_check(
                            &target,
                            Some(&fetched.branch),
                            None,
                            None,
                            None,
                            Some(&message),
                        )
                        .err()
                        .map(|error| format!("；写入检查状态失败：{error}"))
                        .unwrap_or_default();
                    outcomes.push(failed(&target, format!("{message}{suffix}"), None));
                }
                continue;
            }
        };
        for target in group {
            outcomes.push(update_one(
                &mut sink,
                &target,
                fetched.dir.path(),
                &fetched.branch,
                &scanned,
            ));
        }
    }

    // 台账只在真有更新时重写：空跑不该给用户的 git diff 添一行 updatedAt。
    let changed = outcomes.iter().any(|outcome| outcome.status == "updated");
    if changed {
        sink.flush_ledger()
            .map_err(|error| AppError::Project(format!("真源已更新，但写回台账失败：{error}")))?;
    }
    sink.flush_checks()
        .map_err(|error| AppError::Project(format!("真源已更新，但写入检查缓存失败：{error}")))?;
    Ok(SkillUpdateReport { outcomes })
}

fn failed(target: &Target, error: String, backup: Option<PathBuf>) -> SkillUpdateOutcome {
    SkillUpdateOutcome {
        id: target.id.clone(),
        name: target.name.clone(),
        status: "failed".into(),
        backup_path: backup.map(|path| path.display().to_string()),
        warnings: Vec::new(),
        error: Some(error),
    }
}

fn update_one(
    sink: &mut UpdateSink<'_>,
    target: &Target,
    repo_root: &Path,
    branch: &str,
    scanned: &[ScannedSkill],
) -> SkillUpdateOutcome {
    let matched = match match_remote(scanned, &target.provenance, &target.dir, &target.name) {
        Ok(found) => found,
        Err(error) => {
            let suffix = sink
                .record_check(target, Some(branch), None, None, None, Some(&error))
                .err()
                .map(|error| format!("；写入检查状态失败：{error}"))
                .unwrap_or_default();
            return failed(target, format!("{error}{suffix}"), None);
        }
    };
    let source = remote_dir(repo_root, &matched.relative_path);
    let remote_hash = match archive::compute_dir_hash(&source) {
        Ok(hash) => hash,
        Err(error) => {
            let message = error.to_string();
            let suffix = sink
                .record_check(
                    target,
                    Some(branch),
                    Some(&matched.relative_path),
                    None,
                    None,
                    Some(&message),
                )
                .err()
                .map(|error| format!("；写入检查状态失败：{error}"))
                .unwrap_or_default();
            return failed(target, format!("{message}{suffix}"), None);
        }
    };
    let local_hash = archive::compute_dir_hash(&target.dir).ok();
    if local_hash.as_deref() == Some(remote_hash.as_str()) {
        let warnings = match sink.record_check(
            target,
            Some(branch),
            Some(&matched.relative_path),
            Some(&remote_hash),
            Some(false),
            None,
        ) {
            Ok(()) => Vec::new(),
            Err(error) => vec![format!("写入检查结果失败：{error}")],
        };
        return SkillUpdateOutcome {
            id: target.id.clone(),
            name: target.name.clone(),
            status: "upToDate".into(),
            backup_path: None,
            warnings,
            error: None,
        };
    }

    let staging = match sink.dest().staging(&format!("update-{}", target.id)) {
        Ok(path) => path,
        Err(error) => return failed(target, error.to_string(), None),
    };
    let prepared = staging.join("prepared");
    let mut backup: Option<PathBuf> = None;
    // 写入流水线：复制上游 → 落来源（只有中央库有 meta）→ 先备份 → 原子换目录 → 回写来源。
    // 顺序不能调：备份必须在换目录之前成功，失败时 replace_dir 自己把旧目录放回去。
    let result = (|| -> Result<(), AppError> {
        copy_dir(&source, &prepared)?;
        if let Some(meta) = target
            .provenance
            .updated_meta(branch, &matched.relative_path, &remote_hash)
        {
            write_meta(&prepared, &meta)?;
        }
        backup = sink.dest().backup(&target.id)?;
        replace_dir(&target.dir, &prepared, &staging)?;
        sink.after_update(target, branch, &matched.relative_path, &remote_hash);
        Ok(())
    })();

    match result {
        Ok(()) => {
            let warnings = sink.dest().after_write(&target.id);
            let _ = fs::remove_dir_all(&staging);
            SkillUpdateOutcome {
                id: target.id.clone(),
                name: target.name.clone(),
                status: "updated".into(),
                backup_path: backup.map(|path| path.display().to_string()),
                warnings,
                error: None,
            }
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            failed(target, error.to_string(), backup)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::project::{
        ProjectLedger, ProjectPaths, ProjectSkillEntry, ProjectSkillSource,
        PROJECT_LEDGER_VERSION,
    };
    use crate::domain::skill_meta::{SkillMeta, SkillSourceKind};
    use crate::import::update_matrix;
    use crate::services::project_scan;
    use crate::services::project_sync;
    use crate::store::project_checks::ProjectCheckCache;
    use crate::store::Store;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::tempdir;

    struct FakeFetcher {
        fixture: PathBuf,
        calls: AtomicUsize,
        branch: String,
    }

    impl FakeFetcher {
        fn new(fixture: PathBuf) -> Self {
            Self {
                fixture,
                calls: AtomicUsize::new(0),
                branch: "main".into(),
            }
        }
    }

    impl RepoFetcher for FakeFetcher {
        fn fetch(&self, _repo: &RepoRef) -> Result<crate::net::FetchedRepo, AppError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let dir = tempdir()?;
            copy_dir(&self.fixture, dir.path())?;
            Ok(crate::net::FetchedRepo {
                dir,
                branch: self.branch.clone(),
            })
        }
    }

    fn remote_fixture(root: &Path, version: &str) -> PathBuf {
        let repo = root.join("repo");
        let skill = repo.join("skills/demo");
        fs::create_dir_all(skill.join("references")).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            format!("---\nname: Demo\ndescription: 演示\n---\n{version}\n"),
        )
        .unwrap();
        fs::write(skill.join("references/a.md"), version).unwrap();
        repo
    }

    fn seed_skill(store: &Store, version: &str) -> PathBuf {
        let dir = store.asset_dir(AssetType::Skill).join("demo");
        fs::create_dir_all(dir.join("references")).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: Demo\n---\n{version}\n"),
        )
        .unwrap();
        fs::write(dir.join("references/a.md"), version).unwrap();
        write_meta(
            &dir,
            &SkillMeta {
                repo_owner: Some("acme".into()),
                repo_name: Some("demo".into()),
                repo_branch: Some("main".into()),
                source_directory: Some("demo".into()),
                source_kind: Some(SkillSourceKind::CcSwitch),
                installed_at: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        dir
    }

    #[test]
    fn check_reports_updates_and_trusts_fetched_branch() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        seed_skill(&store, "v1");
        let fetcher = FakeFetcher::new(remote_fixture(&dir.path().join("remote"), "v2"));

        let report = check(store_sink(&store, &home), &fetcher, None).unwrap();
        assert_eq!(report.updates.len(), 1);
        assert_eq!(report.updates[0].id, "demo");
        assert!(report.repo_failures.is_empty());
        let meta = read_meta(&store.asset_dir(AssetType::Skill).join("demo")).unwrap();
        assert_eq!(meta.update_available, Some(true));
        assert_eq!(meta.repo_path.as_deref(), Some("skills/demo"));
        assert_eq!(meta.repo_branch.as_deref(), Some("main"));
        assert_eq!(meta.last_checked_at, Some(meta.last_checked_at.unwrap()));
    }

    #[test]
    fn update_backs_up_replaces_meta_and_is_idempotent() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        seed_skill(&store, "v1");
        let fetcher = FakeFetcher::new(remote_fixture(&dir.path().join("remote"), "v2"));

        let report = update(store_sink(&store, &home), &fetcher, &["demo".to_string()]).unwrap();
        assert_eq!(report.outcomes[0].status, "updated");
        assert!(report.outcomes[0].backup_path.is_some());
        let installed = store.asset_dir(AssetType::Skill).join("demo");
        assert!(fs::read_to_string(installed.join("SKILL.md"))
            .unwrap()
            .contains("v2"));
        let meta = read_meta(&installed).unwrap();
        assert_eq!(meta.update_available, Some(false));
        assert_eq!(meta.installed_at, Some(1), "安装时间不能被更新覆盖");

        let backup = PathBuf::from(report.outcomes[0].backup_path.as_ref().unwrap());
        assert!(fs::read_to_string(backup.join("SKILL.md"))
            .unwrap()
            .contains("v1"));

        let second = update(store_sink(&store, &home), &fetcher, &["demo".to_string()]).unwrap();
        assert_eq!(second.outcomes[0].status, "upToDate");
    }

    #[test]
    fn check_groups_by_repo_and_fetches_once() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        seed_skill(&store, "v1");
        let second = store.asset_dir(AssetType::Skill).join("demo-two");
        fs::create_dir_all(second.join("nested")).unwrap();
        fs::write(second.join("SKILL.md"), "---\nname: Demo Two\n---\nv1\n").unwrap();
        write_meta(
            &second,
            &SkillMeta {
                repo_owner: Some("acme".into()),
                repo_name: Some("demo".into()),
                repo_branch: Some("main".into()),
                source_directory: Some("nested".into()),
                source_kind: Some(SkillSourceKind::CcSwitch),
                ..Default::default()
            },
        )
        .unwrap();

        let fixture = remote_fixture(&dir.path().join("remote"), "v2");
        // demo-two 在仓库里叫 nested；给 source_directory 一个精确路径。
        let nested = fixture.join("skills/nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("SKILL.md"), "---\nname: Demo Two\n---\nv2\n").unwrap();
        let fetcher = FakeFetcher::new(fixture);

        let report = check(store_sink(&store, &home), &fetcher, None).unwrap();
        assert_eq!(report.updates.len(), 2);
        assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn update_refreshes_copy_deployments() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        let source = seed_skill(&store, "v1");
        let target = home.join(".claude/skills/demo");
        copy_dir(&source, &target).unwrap();
        fs::write(target.join(".ccswitchx"), source.display().to_string()).unwrap();
        update_matrix(&store, "claude", |matrix| {
            matrix.enable(AssetType::Skill, "demo");
        })
        .unwrap();

        let fetcher = FakeFetcher::new(remote_fixture(&dir.path().join("remote"), "v2"));
        let report = update(store_sink(&store, &home), &fetcher, &["demo".to_string()]).unwrap();
        assert_eq!(report.outcomes[0].status, "updated");
        assert!(fs::read_to_string(target.join("SKILL.md"))
            .unwrap()
            .contains("v2"));
    }

    fn store_sink<'a>(store: &'a Store, home: &'a Path) -> UpdateSink<'a> {
        UpdateSink::Store { store, home }
    }

    /// v3 的落点：真源与台账在 store 的 `local/projects/app-1/` 下，ID 与 sink 传的一致。
    fn project_paths(store: &Store, root: &Path) -> ProjectPaths {
        ProjectPaths::resolve(store.root(), "app-1", root).unwrap()
    }

    /// 项目场景：`local/projects/app-1/skills/demo` 是项目真源，台账里记着 GitHub 来源。
    fn seed_project(store: &Store, root: &Path, version: &str, source: ProjectSkillSource) {
        let paths = project_paths(store, root);
        let canonical = paths.canonical_skill_dir("demo").unwrap();
        fs::create_dir_all(canonical.join("references")).unwrap();
        fs::write(
            canonical.join("SKILL.md"),
            format!("---\nname: Demo\n---\n{version}\n"),
        )
        .unwrap();
        fs::write(canonical.join("references/a.md"), format!("{version}\n"))
            .unwrap();
        let mut ledger = ProjectLedger {
            version: PROJECT_LEDGER_VERSION,
            ..Default::default()
        };
        ledger.skills.insert(
            "demo".to_string(),
            ProjectSkillEntry {
                source: Some(source),
                canonical: project_scan::tree_state(&canonical).unwrap(),
                ..Default::default()
            },
        );
        project_scan::save_ledger(&paths, &ledger).unwrap();
    }

    fn github_source() -> ProjectSkillSource {
        ProjectSkillSource {
            kind: "github".to_string(),
            repo: Some("acme/demo".to_string()),
            branch: Some("main".to_string()),
            subpath: Some("skills/demo".to_string()),
            tree_hash: None,
        }
    }

    fn project_sink<'a>(
        store: &'a Store,
        paths: &'a ProjectPaths,
        ledger: &'a mut ProjectLedger,
        cache: &'a mut ProjectCheckCache,
    ) -> UpdateSink<'a> {
        UpdateSink::Project {
            store,
            paths,
            timestamp: 7_000,
            ledger,
            cache,
        }
    }

    /// 检查更新是只读动作：台账与真源一个字节都不能变，结果只落本机缓存。
    #[test]
    fn project_check_writes_only_the_local_cache() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let root = dir.path().join("home/work/app");
        let paths = project_paths(&store, &root);
        seed_project(&store, &root, "v1", github_source());
        let ledger_file = paths.ledger_path().to_path_buf();
        let ledger_before = fs::read(&ledger_file).unwrap();
        let fetcher = FakeFetcher::new(remote_fixture(&dir.path().join("remote"), "v2"));
        let mut ledger = project_scan::load_ledger(&paths).unwrap();
        let mut cache = ProjectCheckCache::default();

        let report = check(
            project_sink(&store, &paths, &mut ledger, &mut cache),
            &fetcher,
            None,
        )
        .unwrap();

        assert_eq!(report.updates.len(), 1);
        assert_eq!(report.updates[0].id, "demo");
        assert!(report.repo_failures.is_empty());
        assert_eq!(
            fs::read(&ledger_file).unwrap(),
            ledger_before,
            "检查更新不能碰台账"
        );
        let canonical = paths.canonical_skill_dir("demo").unwrap();
        assert!(fs::read_to_string(canonical.join("SKILL.md"))
            .unwrap()
            .contains("v1"));
        assert!(
            !canonical.join(".meta.json").exists(),
            "项目真源不写 .meta.json：来源已在台账里，再写会让每次检查都改变目录指纹"
        );

        let mut cached = ProjectCheckCache::load(&store, "app-1");
        let recorded = cached.skills.remove("demo").expect("检查结果必须落在本机缓存里");
        assert!(recorded.update_available);
        assert!(recorded.updatable);
        assert!(recorded.latest_hash.is_some());
        assert!(recorded.last_check_error.is_none());
    }

    /// 更新只写真源：渲染产物不动，等用户下一次同步；台账里的来源指纹跟着刷新。
    #[test]
    fn project_update_replaces_the_canonical_and_refreshes_the_source() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let root = dir.path().join("home/work/app");
        let paths = project_paths(&store, &root);
        seed_project(&store, &root, "v1", github_source());
        let before = project_scan::load_ledger(&paths)
            .unwrap()
            .skills["demo"]
            .canonical
            .clone();
        let rendered = root.join(".claude/skills/demo");
        fs::create_dir_all(&rendered).unwrap();
        fs::write(rendered.join("SKILL.md"), "旧的渲染产物").unwrap();
        let fetcher = FakeFetcher::new(remote_fixture(&dir.path().join("remote"), "v2"));
        let canonical = paths.canonical_skill_dir("demo").unwrap();

        let mut ledger = project_scan::load_ledger(&paths).unwrap();
        let mut cache = ProjectCheckCache::default();
        let report = update(
            project_sink(&store, &paths, &mut ledger, &mut cache),
            &fetcher,
            &["demo".to_string()],
        )
        .unwrap();
        assert_eq!(report.outcomes[0].status, "updated", "{:?}", report.outcomes);
        assert!(report.outcomes[0].warnings.is_empty());

        assert!(fs::read_to_string(canonical.join("SKILL.md"))
            .unwrap()
            .contains("v2"));
        assert!(fs::read_to_string(canonical.join("references/a.md"))
            .unwrap()
            .contains("v2"));
        assert!(!canonical.join(".meta.json").exists());
        assert_eq!(
            fs::read_to_string(rendered.join("SKILL.md")).unwrap(),
            "旧的渲染产物",
            "更新不越权渲染，铺开是同步引擎的事"
        );
        let leftovers: Vec<String> = canonical
            .parent()
            .unwrap()
            .read_dir()
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.starts_with(".staging-"))
            .collect();
        assert!(leftovers.is_empty(), "暂存在 store 的 staging 区，真源旁边不该留任何东西");

        let backup = PathBuf::from(report.outcomes[0].backup_path.as_ref().unwrap());
        assert!(backup.starts_with(store.root().join("backups").join("projects").join("app-1")));
        assert!(fs::read_to_string(backup.join("SKILL.md"))
            .unwrap()
            .contains("v1"));

        let saved = project_scan::load_ledger(&paths).unwrap();
        let entry = &saved.skills["demo"];
        let source = entry.source.as_ref().unwrap();
        assert_eq!(source.branch.as_deref(), Some("main"));
        assert_eq!(source.subpath.as_deref(), Some("skills/demo"));
        assert_eq!(
            source.tree_hash.as_deref(),
            archive::compute_dir_hash(&canonical).ok().as_deref(),
            "来源哈希取更新那一刻上游目录的 compute_dir_hash"
        );
        assert_eq!(
            entry.canonical.tree_hash, before.tree_hash,
            "真源基线不刷：目标格子才会被判成待更新"
        );
        assert!(!ProjectCheckCache::load(&store, "app-1")
            .get("demo")
            .unwrap()
            .update_available);

        let mut ledger = project_scan::load_ledger(&paths).unwrap();
        let mut cache = ProjectCheckCache::default();
        let second = update(
            project_sink(&store, &paths, &mut ledger, &mut cache),
            &fetcher,
            &["demo".to_string()],
        )
        .unwrap();
        assert_eq!(second.outcomes[0].status, "upToDate");
    }

    #[test]
    fn project_check_skips_a_skill_without_a_github_upstream() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let root = dir.path().join("home/work/app");
        let paths = project_paths(&store, &root);
        seed_project(
            &store,
            &root,
            "v1",
            ProjectSkillSource {
                kind: "local".to_string(),
                repo: None,
                branch: None,
                subpath: None,
                tree_hash: None,
            },
        );
        let ledger_file = paths.ledger_path().to_path_buf();
        let ledger_before = fs::read(&ledger_file).unwrap();
        let mut ledger = project_scan::load_ledger(&paths).unwrap();
        let mut cache = ProjectCheckCache::default();

        let report = check(
            project_sink(&store, &paths, &mut ledger, &mut cache),
            &FakeFetcher::new(remote_fixture(&dir.path().join("remote"), "v2")),
            None,
        )
        .unwrap();

        assert!(report.updates.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].reason.contains("GitHub"), "{:?}", report.skipped);
        assert_eq!(fs::read(&ledger_file).unwrap(), ledger_before);
        assert!(ProjectCheckCache::load(&store, "app-1")
            .get("demo")
            .is_none());
    }

    /// 账本记着来源、磁盘上还没有真源（未迁移或未收进真源）：只能跳过，不能凭空写。
    #[test]
    fn project_update_reports_a_canonical_that_is_not_on_disk() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let root = dir.path().join("home/work/app");
        let paths = project_paths(&store, &root);
        seed_project(&store, &root, "v1", github_source());
        fs::remove_dir_all(paths.canonical_skill_dir("demo").unwrap()).unwrap();
        let mut ledger = project_scan::load_ledger(&paths).unwrap();
        let mut cache = ProjectCheckCache::default();

        let report = update(
            project_sink(&store, &paths, &mut ledger, &mut cache),
            &FakeFetcher::new(remote_fixture(&dir.path().join("remote"), "v2")),
            &["demo".to_string()],
        )
        .unwrap();

        assert_eq!(report.outcomes[0].status, "failed");
        assert!(
            !paths.canonical_skill_dir("demo").unwrap().exists(),
            "跳过意味着一个字都不写"
        );
    }

    #[test]
    fn project_update_reports_an_unknown_skill_id() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let root = dir.path().join("home/work/app");
        let paths = project_paths(&store, &root);
        seed_project(&store, &root, "v1", github_source());
        let mut ledger = project_scan::load_ledger(&paths).unwrap();
        let mut cache = ProjectCheckCache::default();

        let report = update(
            project_sink(&store, &paths, &mut ledger, &mut cache),
            &FakeFetcher::new(remote_fixture(&dir.path().join("remote"), "v2")),
            &["ghost".to_string()],
        )
        .unwrap();

        assert_eq!(report.outcomes.len(), 1);
        assert_eq!(report.outcomes[0].id, "ghost");
        assert_eq!(report.outcomes[0].status, "failed");
        assert!(report.outcomes[0].error.is_some(), "{:?}", report.outcomes);
    }

    /// 更新与铺开是两个动作：更新只换真源，把「目标落后了」留给下一次同步判断。
    #[test]
    fn project_update_leaves_the_targets_for_the_next_sync() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let root = dir.path().join("home/work/app");
        let paths = project_paths(&store, &root);
        seed_project(&store, &root, "v1", github_source());
        let allowed = vec!["claude".to_string()];
        let enabled = vec!["claude".to_string()];
        let rendered = root.join(".claude/skills/demo");

        project_sync::set_targets(&paths, "demo", &allowed, &allowed).unwrap();
        project_sync::sync_project(&store, &paths, &enabled, &allowed, &[]).unwrap();
        assert!(fs::read_to_string(rendered.join("SKILL.md"))
            .unwrap()
            .contains("v1"));

        let mut ledger = project_scan::load_ledger(&paths).unwrap();
        let mut cache = ProjectCheckCache::default();
        update(
            project_sink(&store, &paths, &mut ledger, &mut cache),
            &FakeFetcher::new(remote_fixture(&dir.path().join("remote"), "v2")),
            &["demo".to_string()],
        )
        .unwrap();
        assert!(
            fs::read_to_string(rendered.join("SKILL.md"))
                .unwrap()
                .contains("v1"),
            "更新不越权渲染"
        );

        let report = project_sync::sync_project(&store, &paths, &enabled, &allowed, &[]);
        let report = report.unwrap();
        assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
        assert!(
            report
                .outcomes
                .iter()
                .any(|item| item.status == "ok" && item.action == "render"),
            "{:?}",
            report.outcomes
        );
        assert!(fs::read_to_string(rendered.join("SKILL.md"))
            .unwrap()
            .contains("v2"));
    }
}
