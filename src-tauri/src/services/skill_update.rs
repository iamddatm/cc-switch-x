//! 从上游 GitHub 仓库检查并更新已安装的 Skill。
//!
//! 同一个仓库在一次批量里只下载一次；本地哈希现场计算，不信任缓存，
//! 因此用户手改过 `SKILL.md` 也会被识别为“与上游不同”。

use crate::domain::skill::parse_metadata;
use crate::domain::skill_meta::{read_meta, write_meta, SkillMeta};
use crate::domain::AssetType;
use crate::error::AppError;
use crate::net::archive::{self, ScannedSkill};
use crate::net::{parse_github_input, RepoFetcher, RepoRef};
use crate::services::skill_source::{
    new_staging_dir, refresh_deployed, replace_dir, SkillItemFailure,
};
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
    meta: SkillMeta,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GroupKey {
    owner: String,
    name: String,
    branch: Option<String>,
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

fn load_targets(store: &Store, ids: Option<&[String]>) -> Result<LoadReport, AppError> {
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
            meta,
        });
    }
    targets.sort_by(|left, right| left.id.cmp(&right.id));
    Ok((targets, skipped, failures))
}

fn group_targets(targets: Vec<Target>) -> BTreeMap<GroupKey, Vec<Target>> {
    let mut groups = BTreeMap::new();
    for target in targets {
        let Some(owner) = target
            .meta
            .repo_owner
            .clone()
            .filter(|v| !v.trim().is_empty())
        else {
            continue;
        };
        let Some(name) = target
            .meta
            .repo_name
            .clone()
            .filter(|v| !v.trim().is_empty())
        else {
            continue;
        };
        let branch = target
            .meta
            .repo_branch
            .clone()
            .filter(|value| !value.trim().is_empty());
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
    meta: &SkillMeta,
    local_dir: &Path,
    display_name: &str,
) -> Result<&'a ScannedSkill, String> {
    if let Some(path) = meta.repo_path.as_deref().map(normalize_path) {
        if let Some(found) = scanned
            .iter()
            .find(|candidate| candidate.relative_path == path)
        {
            return Ok(found);
        }
    }
    if let Some(url) = meta.readme_url.as_deref() {
        let matches = scanned
            .iter()
            .filter(|candidate| readme_matches(url, &candidate.relative_path))
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            return Ok(matches[0]);
        }
    }
    if let Some(directory) = meta.source_directory.as_deref().map(normalize_path) {
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

fn persist_check(
    target: &Target,
    branch: Option<&str>,
    repo_path: Option<&str>,
    latest_hash: Option<&str>,
    // None 表示这次没拿到远端结果，保留上一次的可更新状态。
    update_available: Option<bool>,
    error: Option<&str>,
) -> Result<(), AppError> {
    let mut meta = target.meta.clone();
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
    write_meta(&target.dir, &meta)?;
    Ok(())
}

pub fn check<F: RepoFetcher>(
    store: &Store,
    fetcher: &F,
    ids: Option<&[String]>,
) -> Result<SkillCheckReport, AppError> {
    let (targets, skipped, mut skill_failures) = load_targets(store, ids)?;
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
                        persist_check(target, None, None, None, None, Some(&message))
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
                    if let Err(error) = persist_check(
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
            let matched = match match_remote(&scanned, &target.meta, &target.dir, &target.name) {
                Ok(found) => found,
                Err(error) => {
                    skill_failures.push(SkillItemFailure {
                        id: target.id.clone(),
                        error: error.clone(),
                    });
                    if let Err(write_error) = persist_check(
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
                    if let Err(write_error) = persist_check(
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
            if let Err(error) = persist_check(
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
    Ok(report)
}

pub fn update<F: RepoFetcher>(
    store: &Store,
    home: &Path,
    fetcher: &F,
    ids: &[String],
) -> Result<SkillUpdateReport, AppError> {
    let (targets, skipped, mut failures) = load_targets(store, Some(ids))?;
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
                    let suffix = persist_check(&target, None, None, None, None, Some(&message))
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
                    let suffix = persist_check(
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
                store,
                home,
                &target,
                fetched.dir.path(),
                &fetched.branch,
                &scanned,
            ));
        }
    }
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
    store: &Store,
    home: &Path,
    target: &Target,
    repo_root: &Path,
    branch: &str,
    scanned: &[ScannedSkill],
) -> SkillUpdateOutcome {
    let matched = match match_remote(scanned, &target.meta, &target.dir, &target.name) {
        Ok(found) => found,
        Err(error) => {
            let suffix = persist_check(target, Some(branch), None, None, None, Some(&error))
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
            let suffix = persist_check(
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
        let warnings = match persist_check(
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

    let staging = match new_staging_dir(store, &format!("update-{}", target.id)) {
        Ok(path) => path,
        Err(error) => return failed(target, error.to_string(), None),
    };
    let prepared = staging.join("prepared");
    let mut backup: Option<PathBuf> = None;
    let result = (|| -> Result<(), AppError> {
        copy_dir(&source, &prepared)?;
        let mut meta = target.meta.clone();
        meta.repo_branch = Some(branch.to_string());
        meta.repo_path = Some(matched.relative_path.clone());
        meta.content_hash = Some(remote_hash.clone());
        meta.updated_at = Some(now_ms());
        meta.last_checked_at = Some(now_ms());
        meta.latest_hash = Some(remote_hash.clone());
        meta.update_available = Some(false);
        meta.last_check_error = None;
        if meta.readme_url.is_none() && meta.is_updatable() {
            let owner = meta.repo_owner.as_deref().unwrap_or_default();
            let name = meta.repo_name.as_deref().unwrap_or_default();
            let suffix = if matched.relative_path == "." {
                String::new()
            } else {
                format!("{}/", matched.relative_path)
            };
            meta.readme_url = Some(format!(
                "https://github.com/{owner}/{name}/blob/{branch}/{suffix}SKILL.md"
            ));
        }
        write_meta(&prepared, &meta)?;
        backup = store.backup_skill(&target.id)?;
        replace_dir(&target.dir, &prepared, &staging)?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            let warnings = refresh_deployed(store, home, &target.id);
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
    use crate::domain::skill_meta::{SkillMeta, SkillSourceKind};
    use crate::import::update_matrix;
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
        seed_skill(&store, "v1");
        let fetcher = FakeFetcher::new(remote_fixture(&dir.path().join("remote"), "v2"));

        let report = check(&store, &fetcher, None).unwrap();
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

        let report = update(&store, &home, &fetcher, &["demo".to_string()]).unwrap();
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

        let second = update(&store, &home, &fetcher, &["demo".to_string()]).unwrap();
        assert_eq!(second.outcomes[0].status, "upToDate");
    }

    #[test]
    fn check_groups_by_repo_and_fetches_once() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
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

        let report = check(&store, &fetcher, None).unwrap();
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
        let report = update(&store, &home, &fetcher, &["demo".to_string()]).unwrap();
        assert_eq!(report.outcomes[0].status, "updated");
        assert!(fs::read_to_string(target.join("SKILL.md"))
            .unwrap()
            .contains("v2"));
    }
}
