use crate::domain::project::{
    ProjectDirState, ProjectLedger, ProjectSkillEntry, ProjectSkillOp, PROJECT_SKILL_DIRS,
};
use crate::error::AppError;
use crate::services::project_scan::{self, ProjectScan};
use crate::store::{copy_dir, is_safe_id, Store};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Add,
    Update,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirChange {
    pub dir: String,
    pub kind: ChangeKind,
    /// 该目录当前的扫描结果，冲突对比与推荐都按它判断。
    pub state: ProjectDirState,
    pub mtime: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncAction {
    pub skill_id: String,
    /// copy | delete
    pub action: String,
    pub dir: String,
    /// copy 的来源目录；delete 时为 None。
    pub source_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncConflict {
    pub skill_id: String,
    pub dirs: Vec<String>,
    /// mtime 最新的一侧，仅作推荐；有删除参与时不推荐。
    pub recommended: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncPlan {
    pub actions: Vec<SyncAction>,
    pub conflicts: Vec<SyncConflict>,
    pub warnings: Vec<String>,
}

/// 冲突的人工处置：`dir` 是胜出目录，`action` 为 `use` 或 `keepBoth`。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolution {
    pub skill_id: String,
    pub dir: String,
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncOutcome {
    pub skill_id: String,
    pub dir: String,
    /// copy | delete | keepBoth
    pub action: String,
    /// ok | failed
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub outcomes: Vec<SyncOutcome>,
    pub conflicts: Vec<SyncConflict>,
    pub warnings: Vec<String>,
}

/// 按账本 targets 比较基线与当前状态：基线无+当前有 = Add，基线有+当前无 = Delete，哈希不同 = Update。
pub fn diff_entry(
    entry: &ProjectSkillEntry,
    current: &BTreeMap<String, ProjectDirState>,
) -> Vec<DirChange> {
    let mut changes = Vec::new();
    for dir in &entry.targets {
        let baseline = entry.dirs.get(dir);
        let now = current.get(dir).cloned().unwrap_or_default();
        let baseline_present = baseline.is_some_and(|state| state.present);
        let kind = match (baseline_present, now.present) {
            (false, true) => ChangeKind::Add,
            (true, false) => ChangeKind::Delete,
            (true, true) => {
                if baseline.and_then(|state| state.tree_hash.as_deref())
                    != now.tree_hash.as_deref()
                {
                    ChangeKind::Update
                } else {
                    continue;
                }
            }
            (false, false) => continue,
        };
        let mtime = if now.present { now.mtime } else { 0 };
        changes.push(DirChange {
            dir: dir.clone(),
            kind,
            state: now,
            mtime,
        });
    }
    changes
}

fn same_state(left: &ProjectDirState, right: &ProjectDirState) -> bool {
    left.present == right.present && left.tree_hash == right.tree_hash
}

/// 变化目录里排除不可用的（agent 被停用等）；不可用目录既不判定也不写入。
fn actionable_changes(
    entry: &ProjectSkillEntry,
    current: &BTreeMap<String, ProjectDirState>,
    unavailable: &[String],
) -> Vec<DirChange> {
    diff_entry(entry, current)
        .into_iter()
        .filter(|change| !unavailable.contains(&change.dir))
        .collect()
}

/// 多个目录变成同一份内容（或一起消失）＝同一次操作，只需刷新基线。
fn identical_operation(changes: &[DirChange]) -> bool {
    changes.len() > 1
        && changes.windows(2).all(|pair| {
            same_state(&pair[0].state, &pair[1].state)
                && (pair[0].kind == ChangeKind::Delete) == (pair[1].kind == ChangeKind::Delete)
        })
}

/// 把「最后操作为准」的规则换算成可执行计划：单变化传播，多变化要么视为同一次操作、要么冲突。
pub fn plan_sync(ledger: &ProjectLedger, scan: &ProjectScan, unavailable: &[String]) -> SyncPlan {
    let mut plan = SyncPlan::default();

    for (skill_id, entry) in &ledger.skills {
        let changes = actionable_changes(entry, &current_dirs(scan, skill_id), unavailable);
        if changes.is_empty() {
            continue;
        }

        if changes.len() == 1 {
            let change = &changes[0];
            for target in &entry.targets {
                if target == &change.dir {
                    continue;
                }
                if unavailable.contains(target) {
                    plan.warnings
                        .push(format!("{skill_id}：目标 {target} 当前不可用，同步已跳过"));
                    continue;
                }
                plan.actions.push(match change.kind {
                    ChangeKind::Add | ChangeKind::Update => SyncAction {
                        skill_id: skill_id.clone(),
                        action: "copy".into(),
                        dir: target.clone(),
                        source_dir: Some(change.dir.clone()),
                    },
                    ChangeKind::Delete => SyncAction {
                        skill_id: skill_id.clone(),
                        action: "delete".into(),
                        dir: target.clone(),
                        source_dir: None,
                    },
                });
            }
            continue;
        }

        if identical_operation(&changes) {
            continue;
        }

        let has_delete = changes.iter().any(|change| change.kind == ChangeKind::Delete);
        let recommended = if has_delete {
            None
        } else {
            changes
                .iter()
                .max_by_key(|change| change.mtime)
                .map(|change| change.dir.clone())
        };
        plan.conflicts.push(SyncConflict {
            skill_id: skill_id.clone(),
            dirs: changes.iter().map(|change| change.dir.clone()).collect(),
            recommended,
        });
    }

    plan
}

fn current_dirs(scan: &ProjectScan, skill_id: &str) -> BTreeMap<String, ProjectDirState> {
    scan.skills
        .get(skill_id)
        .map(|snapshot| snapshot.dirs.clone())
        .unwrap_or_default()
}

/// 一次同步里的一个待执行步骤。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    /// 冲突「两边都保留」：落败目录先另存为未跟踪副本。
    Preserve { skill_id: String, dir: String },
    Apply(SyncAction),
}

/// 已落盘的改动，失败时按它回滚。
struct Applied {
    /// 本次新建的目录：撤销＝直接删掉。
    created: Option<PathBuf>,
    /// 被替换或删除的目录 + 备份路径：撤销＝删除现状再从备份还原。
    restored: Option<(PathBuf, PathBuf)>,
}

fn spec_relative_path(dir: &str) -> Option<&'static str> {
    PROJECT_SKILL_DIRS
        .iter()
        .find(|spec| spec.key.as_str() == dir)
        .map(|spec| spec.relative_path)
}

/// 项目内的 skill 目录：目录 key 必须是已知落点，skill id 必须能当文件名，且不得越出项目根。
fn skill_dir(project_root: &Path, dir: &str, skill_id: &str) -> Result<PathBuf, AppError> {
    let relative = spec_relative_path(dir)
        .ok_or_else(|| AppError::Project(format!("未知的项目目录：{dir}")))?;
    if !is_safe_id(skill_id) {
        return Err(AppError::Project(format!(
            "Skill ID 不能用作目录名：{skill_id}"
        )));
    }
    let path = project_root.join(relative).join(skill_id);
    if !path.starts_with(project_root) {
        return Err(AppError::Project(format!(
            "路径越出项目根：{}",
            path.display()
        )));
    }
    Ok(path)
}

fn backup_target(
    store: &Store,
    project_id: &str,
    timestamp: i64,
    dir: &str,
    skill_id: &str,
    target: &Path,
) -> Result<PathBuf, AppError> {
    let path = store
        .root()
        .join("backups")
        .join("projects")
        .join(project_id)
        .join(timestamp.to_string())
        .join(dir)
        .join(skill_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    copy_dir(target, &path)?;
    Ok(path)
}

fn reject_link(path: &Path, what: &str) -> Result<(), AppError> {
    if project_scan::is_link(path) {
        return Err(AppError::Project(format!(
            "拒绝{what}符号链接目录：{}",
            path.display()
        )));
    }
    Ok(())
}

fn apply_copy(
    store: &Store,
    project_id: &str,
    timestamp: i64,
    project_root: &Path,
    action: &SyncAction,
) -> Result<(SyncOutcome, Applied), AppError> {
    let source_dir = action
        .source_dir
        .as_deref()
        .ok_or_else(|| AppError::Project(format!("{} 缺少同步来源", action.skill_id)))?;
    let source = skill_dir(project_root, source_dir, &action.skill_id)?;
    reject_link(&source, "同步来源")?;
    if !source.is_dir() {
        return Err(AppError::Project(format!(
            "同步源不存在：{}",
            source.display()
        )));
    }

    let target = skill_dir(project_root, &action.dir, &action.skill_id)?;
    reject_link(&target, "同步目标")?;
    let parent = target
        .parent()
        .ok_or_else(|| AppError::Project("目标路径没有父目录".into()))?;
    std::fs::create_dir_all(parent)?;

    let existed = target.is_dir();
    let backup = if existed {
        Some(backup_target(
            store,
            project_id,
            timestamp,
            &action.dir,
            &action.skill_id,
            &target,
        )?)
    } else {
        None
    };

    // 先写同目录下的临时名再替换，避免中途失败留下半个 skill。
    let staging = parent.join(format!(".ccswitchx-{}-{}", timestamp, std::process::id()));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    let result = (|| -> Result<(), AppError> {
        copy_dir(&source, &staging)?;
        if existed {
            std::fs::remove_dir_all(&target)?;
        }
        std::fs::rename(&staging, &target)?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&staging);
        if existed && !target.exists() {
            if let Some(backup) = &backup {
                copy_dir(backup, &target)?;
            }
        }
        return Err(error);
    }

    Ok((
        SyncOutcome {
            skill_id: action.skill_id.clone(),
            dir: action.dir.clone(),
            action: "copy".into(),
            status: "ok".into(),
            backup_path: backup.clone(),
            error: None,
        },
        Applied {
            created: if existed { None } else { Some(target.clone()) },
            restored: if existed {
                backup.clone().map(|backup| (target.clone(), backup))
            } else {
                None
            },
        },
    ))
}

/// 删除只针对基线确认属于本项目的目录；plan_sync 已保证其他 target 仍等于基线，
/// 否则那次变化本身就会被判成冲突。
fn apply_delete(
    store: &Store,
    project_id: &str,
    timestamp: i64,
    project_root: &Path,
    action: &SyncAction,
) -> Result<(SyncOutcome, Applied), AppError> {
    let target = skill_dir(project_root, &action.dir, &action.skill_id)?;
    reject_link(&target, "删除目标")?;
    if !target.is_dir() {
        return Ok((
            SyncOutcome {
                skill_id: action.skill_id.clone(),
                dir: action.dir.clone(),
                action: "delete".into(),
                status: "ok".into(),
                backup_path: None,
                error: None,
            },
            Applied {
                created: None,
                restored: None,
            },
        ));
    }
    let backup = backup_target(
        store,
        project_id,
        timestamp,
        &action.dir,
        &action.skill_id,
        &target,
    )?;
    std::fs::remove_dir_all(&target)?;
    Ok((
        SyncOutcome {
            skill_id: action.skill_id.clone(),
            dir: action.dir.clone(),
            action: "delete".into(),
            status: "ok".into(),
            backup_path: Some(backup.clone()),
            error: None,
        },
        Applied {
            created: None,
            restored: Some((target, backup)),
        },
    ))
}

/// 「两边都保留」：落败目录另存为 `<skill>-copy`（重名递增），不写入 targets。
fn apply_preserve(
    project_root: &Path,
    skill_id: &str,
    dir: &str,
) -> Result<(SyncOutcome, Applied), AppError> {
    let source = skill_dir(project_root, dir, skill_id)?;
    reject_link(&source, "保留副本来源")?;
    let outcome = SyncOutcome {
        skill_id: skill_id.to_string(),
        dir: dir.to_string(),
        action: "keepBoth".into(),
        status: "ok".into(),
        backup_path: None,
        error: None,
    };
    if !source.is_dir() {
        return Ok((
            outcome,
            Applied {
                created: None,
                restored: None,
            },
        ));
    }
    let parent = source
        .parent()
        .ok_or_else(|| AppError::Project("保留路径没有父目录".into()))?;
    let mut attempt = 1usize;
    let created = loop {
        let name = if attempt == 1 {
            format!("{skill_id}-copy")
        } else {
            format!("{skill_id}-copy-{attempt}")
        };
        let path = parent.join(name);
        if !path.exists() {
            break path;
        }
        attempt += 1;
    };
    copy_dir(&source, &created)?;
    Ok((
        outcome,
        Applied {
            created: Some(created),
            restored: None,
        },
    ))
}

fn rollback(applied: &Applied) -> Result<(), AppError> {
    if let Some(path) = &applied.created {
        if path.is_dir() {
            std::fs::remove_dir_all(path)?;
        }
    }
    if let Some((target, backup)) = &applied.restored {
        if target.is_dir() {
            std::fs::remove_dir_all(target)?;
        }
        copy_dir(backup, target)?;
    }
    Ok(())
}

/// 手动/自动同步的唯一入口：算计划 → 落盘（先备份，失败整批回滚）→ 刷新基线。
pub fn sync_project(
    store: &Store,
    project_id: &str,
    project_root: &Path,
    enabled_agent_ids: &[String],
    resolutions: &[Resolution],
) -> Result<SyncReport, AppError> {
    let ledger = project_scan::load_ledger(project_root)?;
    let enabled_keys = project_scan::enabled_dir_keys(enabled_agent_ids);
    let enabled_names: Vec<String> = enabled_keys
        .iter()
        .map(|key| key.as_str().to_string())
        .collect();
    let scan = project_scan::scan(project_root, &enabled_keys)?;

    let mut unavailable: Vec<String> = Vec::new();
    for entry in ledger.skills.values() {
        for target in &entry.targets {
            if !enabled_names.contains(target) && !unavailable.contains(target) {
                unavailable.push(target.clone());
            }
        }
    }

    let plan = plan_sync(&ledger, &scan, &unavailable);
    let mut report = SyncReport {
        outcomes: Vec::new(),
        conflicts: plan.conflicts.clone(),
        warnings: plan.warnings.clone(),
    };
    report.warnings.extend(scan.warnings.clone());

    for resolution in resolutions {
        let known = report
            .conflicts
            .iter()
            .any(|conflict| conflict.skill_id == resolution.skill_id
                && conflict.dirs.contains(&resolution.dir));
        if !known {
            return Err(AppError::Project(format!(
                "冲突处理无效：{} / {}",
                resolution.skill_id, resolution.dir
            )));
        }
        if !matches!(resolution.action.as_str(), "use" | "keepBoth") {
            return Err(AppError::Project(format!(
                "未知的冲突处理方式：{}",
                resolution.action
            )));
        }
    }

    let mut steps: Vec<Step> = Vec::new();
    let mut applied_skills: BTreeSet<String> = BTreeSet::new();

    // 1. 人工裁决的冲突：按胜出目录当作唯一变化传播。
    let mut resolved: BTreeSet<String> = BTreeSet::new();
    for conflict in &report.conflicts.clone() {
        let Some(resolution) = resolutions
            .iter()
            .find(|item| item.skill_id == conflict.skill_id)
        else {
            continue;
        };
        let Some(entry) = ledger.skills.get(&conflict.skill_id) else {
            continue;
        };
        let changes = actionable_changes(entry, &current_dirs(&scan, &conflict.skill_id), &unavailable);
        let Some(winner) = changes.iter().find(|change| change.dir == resolution.dir) else {
            continue;
        };
        for target in &entry.targets {
            if target == &resolution.dir || unavailable.contains(target) {
                continue;
            }
            if resolution.action == "keepBoth" {
                steps.push(Step::Preserve {
                    skill_id: conflict.skill_id.clone(),
                    dir: target.clone(),
                });
            }
            steps.push(Step::Apply(SyncAction {
                skill_id: conflict.skill_id.clone(),
                action: if winner.kind == ChangeKind::Delete {
                    "delete".into()
                } else {
                    "copy".into()
                },
                dir: target.clone(),
                source_dir: if winner.kind == ChangeKind::Delete {
                    None
                } else {
                    Some(resolution.dir.clone())
                },
            }));
        }
        applied_skills.insert(conflict.skill_id.clone());
        resolved.insert(conflict.skill_id.clone());
    }
    report
        .conflicts
        .retain(|conflict| !resolved.contains(&conflict.skill_id));

    // 2. 无歧义的自动传播。
    for action in &plan.actions {
        steps.push(Step::Apply(action.clone()));
        applied_skills.insert(action.skill_id.clone());
    }

    // 3. 新勾选目标：目标缺失而已有 target 有内容时按首次同步复制；内容不同则冲突。
    for (skill_id, entry) in &ledger.skills {
        if !applied_skills.contains(skill_id) {
            let current = current_dirs(&scan, skill_id);
            let changes = actionable_changes(entry, &current, &unavailable);
            if !changes.is_empty() {
                continue;
            }
            let tracked: Vec<&String> = entry
                .targets
                .iter()
                .filter(|target| !unavailable.contains(target))
                .collect();
            let present: Vec<&String> = tracked
                .iter()
                .copied()
                .filter(|target| current.get(*target).is_some_and(|state| state.present))
                .collect();
            if present.is_empty() || present.len() == tracked.len() {
                continue;
            }
            let hashes: BTreeSet<Option<String>> = present
                .iter()
                .map(|target| current.get(*target).and_then(|state| state.tree_hash.clone()))
                .collect();
            if hashes.len() > 1 {
                let recommended = present
                    .iter()
                    .copied()
                    .max_by_key(|target| current.get(*target).map_or(0, |state| state.mtime))
                    .map(|target| target.to_string());
                report.conflicts.push(SyncConflict {
                    skill_id: skill_id.clone(),
                    dirs: present.iter().map(|target| target.to_string()).collect(),
                    recommended,
                });
                continue;
            }
            // 源优先取基线里已存在且内容未变的目录。
            let source = present
                .iter()
                .copied()
                .find(|target| {
                    entry
                        .dirs
                        .get(*target)
                        .is_some_and(|state| state.present && same_state(state, &current[*target]))
                })
                .or_else(|| present.first().copied())
                .expect("present 非空");
            for target in tracked.iter().copied() {
                if present.contains(&target) {
                    continue;
                }
                steps.push(Step::Apply(SyncAction {
                    skill_id: skill_id.clone(),
                    action: "copy".into(),
                    dir: target.clone(),
                    source_dir: Some(source.to_string()),
                }));
            }
            applied_skills.insert(skill_id.clone());
        }
    }

    let refresh_only: Vec<String> = ledger
        .skills
        .iter()
        .filter(|(skill_id, entry)| {
            !applied_skills.contains(*skill_id)
                && identical_operation(&actionable_changes(
                    entry,
                    &current_dirs(&scan, skill_id),
                    &unavailable,
                ))
        })
        .map(|(skill_id, _)| skill_id.clone())
        .collect();

    let timestamp = chrono::Utc::now().timestamp_millis();
    let mut applied: Vec<Applied> = Vec::new();
    let mut last_ops: BTreeMap<String, (String, String)> = BTreeMap::new();
    for step in &steps {
        let result = match step {
            Step::Preserve { skill_id, dir } => apply_preserve(project_root, skill_id, dir),
            Step::Apply(action) => {
                let prepared = if action.action == "delete" {
                    apply_delete(store, project_id, timestamp, project_root, action)
                } else {
                    apply_copy(store, project_id, timestamp, project_root, action)
                };
                prepared
            }
        };
        match result {
            Ok((outcome, record)) => {
                if let Step::Apply(action) = step {
                    last_ops
                        .entry(action.skill_id.clone())
                        .or_insert_with(|| {
                            (
                                action
                                    .source_dir
                                    .clone()
                                    .unwrap_or_else(|| action.dir.clone()),
                                action.action.clone(),
                            )
                        });
                }
                report.outcomes.push(outcome);
                applied.push(record);
            }
            Err(error) => {
                let mut rollback_errors: Vec<String> = Vec::new();
                for record in applied.iter().rev() {
                    if let Err(e) = rollback(record) {
                        rollback_errors.push(format!("{e}"));
                    }
                }
                if rollback_errors.is_empty() {
                    return Err(error);
                }
                return Err(AppError::Project(format!(
                    "同步失败，回滚出现问题：{}",
                    rollback_errors.join("；")
                )));
            }
        }
    }

    let now = chrono::Utc::now().to_rfc3339();
    let mut ledger = ledger;
    let mut removed: Vec<String> = Vec::new();
    for skill_id in applied_skills.iter().chain(refresh_only.iter()) {
        let Some(entry) = ledger.skills.get_mut(skill_id) else {
            continue;
        };
        for target in entry.targets.clone() {
            if unavailable.contains(&target) {
                continue;
            }
            let path = skill_dir(project_root, &target, skill_id)?;
            let state = project_scan::tree_state(&path)?;
            entry.dirs.insert(target, state);
        }
        if let Some((dir, action)) = last_ops.get(skill_id) {
            entry.last_op = Some(ProjectSkillOp {
                dir: dir.clone(),
                action: action.clone(),
                at: now.clone(),
            });
        }
        let has_unavailable = entry.targets.iter().any(|target| unavailable.contains(target));
        if !has_unavailable
            && !entry.targets.is_empty()
            && applied_skills.contains(skill_id)
            && entry
                .targets
                .iter()
                .all(|target| entry.dirs.get(target).is_none_or(|state| !state.present))
        {
            removed.push(skill_id.clone());
        }
    }
    for skill_id in &removed {
        ledger.skills.remove(skill_id);
    }
    ledger.updated_at = now;
    project_scan::save_ledger(project_root, &ledger)?;

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::project::{ProjectDirState, ProjectLedger, ProjectSkillEntry};
    use crate::services::project_registry;
    use crate::services::project_scan;
    use crate::services::project_scan::{ProjectScan, ProjectSkillSnapshot};
    use crate::store::Store;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    struct Fixture {
        _temp: tempfile::TempDir,
        store: Store,
        home: PathBuf,
        root: PathBuf,
    }

    fn project_fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::init(temp.path().join("store")).unwrap();
        let home = temp.path().join("home");
        let root = home.join("work").join("app");
        std::fs::create_dir_all(&root).unwrap();
        Fixture {
            _temp: temp,
            store,
            home,
            root,
        }
    }

    /// 保持 `tempdir` 存活，同时借用 store / home / root。
    impl Fixture {
        fn add_project(&self, agents: &[String]) -> project_registry::ProjectView {
            project_registry::add_project(&self.store, &self.home, &self.root, agents).unwrap()
        }

        fn set_targets(&self, skill_id: &str, dirs: &[&str]) {
            let mut ledger = project_scan::load_ledger(&self.root).unwrap();
            let entry = ledger.skills.entry(skill_id.to_string()).or_default();
            entry.targets = dirs.iter().map(|dir| dir.to_string()).collect();
            project_scan::save_ledger(&self.root, &ledger).unwrap();
        }

        fn sync(&self, project_id: &str, resolutions: &[Resolution]) -> Result<SyncReport, AppError> {
            sync_project(
                &self.store,
                project_id,
                &self.root,
                &["claude".to_string(), "qoder".to_string()],
                resolutions,
            )
        }
    }

    fn backup_root(store: &Store) -> PathBuf {
        store.root().join("backups")
    }

    fn seed_skill(root: &Path, id: &str) {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: demo\ndescription: 演示\n---\n",
        )
        .unwrap();
    }

    fn state(hash: &str) -> ProjectDirState {
        ProjectDirState {
            present: true,
            tree_hash: Some(format!("sha256:{hash}")),
            ..Default::default()
        }
    }

    fn absent() -> ProjectDirState {
        ProjectDirState::default()
    }

    fn states(pairs: &[(&str, &str)]) -> BTreeMap<String, ProjectDirState> {
        pairs
            .iter()
            .map(|(dir, hash)| (dir.to_string(), state(hash)))
            .collect()
    }

    fn entry_with_targets(targets: &[&str]) -> ProjectSkillEntry {
        ProjectSkillEntry {
            targets: targets.iter().map(|t| t.to_string()).collect(),
            dirs: targets
                .iter()
                .map(|t| (t.to_string(), state("h1")))
                .collect(),
            ..Default::default()
        }
    }

    fn ledger_with(id: &str, entry: ProjectSkillEntry) -> ProjectLedger {
        ProjectLedger {
            version: 1,
            updated_at: String::new(),
            skills: [(id.to_string(), entry)].into_iter().collect(),
        }
    }

    fn scan_with(id: &str, current: BTreeMap<String, ProjectDirState>) -> ProjectScan {
        ProjectScan {
            skills: [(
                id.to_string(),
                ProjectSkillSnapshot {
                    id: id.to_string(),
                    dirs: current,
                },
            )]
            .into_iter()
            .collect(),
            dirs: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn one_changed_target_propagates_to_other_targets() {
        let entry = entry_with_targets(&["claude", "qoder"]);
        let current = states(&[("claude", "h2"), ("qoder", "h1")]);
        let changes = diff_entry(&entry, &current);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].dir, "claude");
    }

    #[test]
    fn two_changed_targets_are_a_conflict_and_produce_no_actions() {
        let entry = entry_with_targets(&["claude", "qoder"]);
        let current = states(&[("claude", "h2"), ("qoder", "h3")]);
        let ledger = ledger_with("a", entry);
        let scan = scan_with("a", current);
        let plan = plan_sync(&ledger, &scan, &[]);
        assert!(plan.actions.is_empty());
        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflicts[0].skill_id, "a");
    }

    #[test]
    fn delete_plus_edit_is_always_a_conflict() {
        let entry = entry_with_targets(&["claude", "qoder"]);
        let mut current = states(&[("claude", "h2")]);
        current.insert("qoder".into(), absent());
        let plan = plan_sync(&ledger_with("a", entry), &scan_with("a", current), &[]);
        assert!(plan.actions.is_empty());
        assert_eq!(plan.conflicts.len(), 1);
    }

    #[test]
    fn unavailable_targets_are_skipped_not_deleted() {
        let entry = entry_with_targets(&["claude", "qoder"]);
        let current = states(&[("claude", "h2")]);
        let plan = plan_sync(&ledger_with("a", entry), &scan_with("a", current), &["qoder".into()]);
        assert!(plan.actions.is_empty());
        assert!(plan
            .warnings
            .iter()
            .any(|warning| warning.contains("qoder")));
    }

    #[test]
    fn sync_copies_missing_target_and_backs_up_before_delete() {
        let fixture = project_fixture();
        seed_skill(&fixture.root.join(".claude/skills"), "a");
        let view = fixture.add_project(&["claude".into(), "qoder".into()]);
        fixture.set_targets("a", &["claude", "qoder"]);

        let report = fixture.sync(&view.id, &[]).unwrap();
        assert!(report.conflicts.is_empty());
        assert!(fixture.root.join(".qoder/skills/a/SKILL.md").is_file());

        std::fs::remove_dir_all(fixture.root.join(".qoder/skills/a")).unwrap();
        let report = fixture.sync(&view.id, &[]).unwrap();
        assert!(report.conflicts.is_empty());
        assert!(!fixture.root.join(".claude/skills/a").exists());
        assert!(backup_root(&fixture.store).join("projects").is_dir());
    }

    #[test]
    fn sync_conflict_writes_nothing() {
        let fixture = project_fixture();
        seed_skill(&fixture.root.join(".claude/skills"), "a");
        seed_skill(&fixture.root.join(".qoder/skills"), "a");
        let view = fixture.add_project(&["claude".into(), "qoder".into()]);
        fixture.set_targets("a", &["claude", "qoder"]);
        std::fs::write(fixture.root.join(".claude/skills/a/SKILL.md"), "claude v2").unwrap();
        std::fs::write(fixture.root.join(".qoder/skills/a/SKILL.md"), "qoder v2").unwrap();

        let before_claude =
            std::fs::read(fixture.root.join(".claude/skills/a/SKILL.md")).unwrap();
        let report = fixture.sync(&view.id, &[]).unwrap();
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(
            std::fs::read(fixture.root.join(".claude/skills/a/SKILL.md")).unwrap(),
            before_claude
        );
        assert!(report.outcomes.is_empty());
    }
}
