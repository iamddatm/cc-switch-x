use crate::domain::project::{
    ProjectDirState, ProjectLedger, ProjectSkillEntry, ProjectSkillOp, PROJECT_SKILL_DIRS,
};
use crate::error::AppError;
use crate::services::project_scan::{self, ProjectScan};
use crate::store::{copy_dir, is_safe_id, Store};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// 渲染来源：项目真源，或（收进真源时）项目内某个目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncSourceRef {
    Canonical,
    Dir(String),
}

/// 一次同步里的动作。v2 起真源是唯一来源，「对等副本互相覆盖」的判定已经移除。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncKind {
    /// 真源 → 目标目录。
    Render,
    /// 删除目标副本（真源仍在）。
    Delete,
    /// 真源已消失且用户裁决 purge：删除全部目标副本。
    Purge,
    /// 某个目录的字节 → 真源。
    Adopt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncAction {
    pub skill_id: String,
    pub kind: SyncKind,
    /// Render/Delete/Purge 时是目标目录 key；Adopt 时是来源目录 key。
    pub dir: String,
    /// Render / Adopt 的来源；Delete 与 Purge 没有来源。
    pub source: Option<SyncSourceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncConflict {
    pub skill_id: String,
    pub dirs: Vec<String>,
    /// mtime 最新的一侧，仅作推荐；有删除参与时不推荐。
    pub recommended: Option<String>,
    /// targetVsCanonical：目录被外部改过；canonicalMissing：账本记着有真源，磁盘上却没有。
    #[serde(default)]
    pub kind: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncPlan {
    pub actions: Vec<SyncAction>,
    pub conflicts: Vec<SyncConflict>,
    pub warnings: Vec<String>,
    /// 只需刷新基线、不需要写盘的技能（内容与真源一致，基线旧了）。
    pub refresh: Vec<String>,
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

fn same_state(left: &ProjectDirState, right: &ProjectDirState) -> bool {
    left.present == right.present && left.tree_hash == right.tree_hash
}

/// 该 skill 在这一轮里不能判定的目录：agent 停用/账本里的陌生 key，以及被链接跳过、状态未知的格子。
/// 这些目录既不驱动 add/delete，也不写入、不刷基线。
fn blocked_dirs(
    entry: &ProjectSkillEntry,
    scan: &ProjectScan,
    skill_id: &str,
    unavailable: &[String],
) -> Vec<String> {
    let mut blocked: Vec<String> = entry
        .targets
        .iter()
        .filter(|dir| unavailable.contains(dir) || scan.blocks(dir, skill_id))
        .cloned()
        .collect();
    blocked.dedup();
    blocked
}

/// v2 判定：真源是唯一来源，不再有「谁最后被改谁赢」的推断。
///
/// - 真源还没落地 → 整条不动（新项目初始状态，等用户「收进真源」）。
/// - 账本记着有真源、磁盘上却没有 → 只报 `canonicalMissing`，删除全部副本要用户显式 purge。
/// - 有 target 被外部改成与真源不同的内容 → 整条等待裁决，**不半应用**（真源赢了又怎样，
///   用户的手改还是丢了）。
/// - 其余情况：每个「干净」的 target 只要与真源不一致就渲染。
pub fn plan_sync(
    ledger: &ProjectLedger,
    scan: &ProjectScan,
    unavailable: &[String],
    allowed_dirs: &[String],
) -> SyncPlan {
    let mut plan = SyncPlan::default();

    for (skill_id, entry) in &ledger.skills {
        if scan.blocks_canonical(skill_id) {
            plan.warnings
                .push(format!("{skill_id}：真源状态取不到（链接等），本轮不渲染"));
            continue;
        }

        // 三种「这一轮不能碰这个目标」的原因各不相同，警告必须说清楚是哪种：
        // 计划层统一负责，调用方不再自己补一遍。
        let mut blocked: Vec<String> = Vec::new();
        for dir in &entry.targets {
            let reason = if !allowed_dirs.contains(dir) {
                Some("未实测或未启用，不能作为渲染目标")
            } else if unavailable.contains(dir) {
                Some("对应的 agent 已停用")
            } else if scan.blocks(dir, skill_id) {
                Some("状态取不到（符号链接等）")
            } else {
                None
            };
            if let Some(reason) = reason {
                blocked.push(dir.clone());
                plan.warnings
                    .push(format!("{skill_id}：目标 {dir} {reason}，本轮跳过"));
            }
        }
        blocked.dedup();

        let current = current_dirs(scan, skill_id);
        let canonical_now = scan.canonical.get(skill_id).cloned().unwrap_or_default();

        if entry.canonical.present && !canonical_now.present {
            let present: Vec<String> = entry
                .targets
                .iter()
                .filter(|dir| !blocked.contains(dir))
                .filter(|dir| current.get(*dir).is_some_and(|state| state.present))
                .cloned()
                .collect();
            if !present.is_empty() {
                plan.conflicts.push(SyncConflict {
                    skill_id: skill_id.clone(),
                    dirs: present,
                    recommended: None,
                    kind: "canonicalMissing".into(),
                });
            }
            continue;
        }
        if !canonical_now.present {
            continue;
        }

        let state_of = |dir: &str| -> ProjectDirState {
            current.get(dir).cloned().unwrap_or_default()
        };
        let baseline_of = |dir: &str| -> ProjectDirState {
            entry.dirs.get(dir).cloned().unwrap_or_default()
        };
        let mut edited: Vec<String> = Vec::new();
        let mut already_in_sync: Vec<String> = Vec::new();
        let mut render: Vec<String> = Vec::new();
        for dir in entry.targets.iter().filter(|dir| !blocked.contains(dir)) {
            let now = state_of(dir);
            if now.present && !same_state(&now, &baseline_of(dir)) {
                if same_state(&now, &canonical_now) {
                    // 内容与真源一致，只是基线旧了：刷新基线即可，不该报冲突也不该重拷。
                    already_in_sync.push(dir.clone());
                } else {
                    edited.push(dir.clone());
                }
                continue;
            }
            if !same_state(&now, &canonical_now) {
                render.push(dir.clone());
            }
        }

        if !edited.is_empty() {
            let recommended = edited
                .iter()
                .max_by_key(|dir| state_of(dir).mtime)
                .cloned();
            plan.conflicts.push(SyncConflict {
                skill_id: skill_id.clone(),
                dirs: edited,
                recommended,
                kind: "targetVsCanonical".into(),
            });
            continue;
        }
        for dir in render {
            plan.actions.push(SyncAction {
                skill_id: skill_id.clone(),
                kind: SyncKind::Render,
                dir,
                source: Some(SyncSourceRef::Canonical),
            });
        }
        if plan
            .actions
            .iter()
            .any(|action| action.skill_id == *skill_id)
        {
            // 有写入的条目由 applied_skills 负责刷基线，避免重复处理。
            continue;
        }
        if !already_in_sync.is_empty() {
            plan.refresh.push(skill_id.clone());
        }
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

impl SyncAction {
    /// 本动作落到的目录：Adopt 写的是真源，其余写的是渲染目标。
    pub fn target_path(&self, project_root: &Path) -> Result<PathBuf, AppError> {
        match self.kind {
            SyncKind::Adopt => crate::domain::project::canonical_skill_dir(project_root, &self.skill_id),
            _ => skill_dir(project_root, &self.dir, &self.skill_id),
        }
    }

    pub fn source_path(&self, project_root: &Path) -> Result<PathBuf, AppError> {
        match self.source.as_ref().ok_or_else(|| {
            AppError::Project(format!("{} 缺少渲染来源", self.skill_id))
        })? {
            SyncSourceRef::Canonical => {
                crate::domain::project::canonical_skill_dir(project_root, &self.skill_id)
            }
            SyncSourceRef::Dir(dir) => skill_dir(project_root, dir, &self.skill_id),
        }
    }

    /// 动作对应的 SyncOutcome.action 字符串（前端与账本用它区分动作）。
    pub fn outcome_action(&self) -> &'static str {
        match self.kind {
            SyncKind::Render => "render",
            SyncKind::Delete | SyncKind::Purge => "delete",
            SyncKind::Adopt => "adopt",
        }
    }
}

/// 能力闸的写入侧校验：`targets` 可能来自旧版本、git 合并或手改的 JSON，
/// 只在勾选时校验挡不住这些来源。
fn assert_writable_dir(dir: &str, allowed_dirs: &[String]) -> Result<(), AppError> {
    if !allowed_dirs.iter().any(|allowed| allowed == dir) {
        return Err(AppError::Project(format!(
            "该目录尚未实测，不能作为渲染目标：{dir}"
        )));
    }
    Ok(())
}

/// 渲染后回读校验：目标与来源指纹必须一致，否则整批回滚（AGENTS.md 写入流水线）。
/// 顺带兜住「真源里有链接」——`tree_state` 遇到链接直接报错。
fn verify_render(source: &Path, target: &Path) -> Result<(), AppError> {
    let from = project_scan::tree_state(source)?;
    let to = project_scan::tree_state(target)?;
    if from.tree_hash != to.tree_hash {
        return Err(AppError::Project(format!(
            "回读校验失败：{} 与 {} 内容不一致",
            source.display(),
            target.display()
        )));
    }
    Ok(())
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

/// 整目录替换：来源可以是真源（Render）也可以是某个渲染目标（Adopt 写回真源）。
/// 备份 → 同父目录临时名 → rename → **回读校验**；失败当场还原。
fn apply_render(
    store: &Store,
    project_id: &str,
    timestamp: i64,
    project_root: &Path,
    action: &SyncAction,
    allowed_dirs: &[String],
) -> Result<(SyncOutcome, Applied), AppError> {
    if action.kind == SyncKind::Render {
        assert_writable_dir(&action.dir, allowed_dirs)?;
    }
    let source = action.source_path(project_root)?;
    reject_link(&source, "渲染来源")?;
    if !source.is_dir() {
        return Err(AppError::Project(format!(
            "渲染来源不存在：{}",
            source.display()
        )));
    }

    let target = action.target_path(project_root)?;
    reject_link(&target, "渲染目标")?;
    let parent = target
        .parent()
        .ok_or_else(|| AppError::Project("目标路径没有父目录".into()))?;
    std::fs::create_dir_all(parent)?;

    // 真源备份放在 canonical/ 下，与渲染目标的目录 key 区分开。
    let slot = match action.kind {
        SyncKind::Adopt => "canonical".to_string(),
        _ => action.dir.clone(),
    };
    let existed = target.is_dir();
    let backup = if existed {
        Some(backup_target(
            store,
            project_id,
            timestamp,
            &slot,
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
        verify_render(&source, &target)
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
            action: action.outcome_action().into(),
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
/// 删除只针对基线确认属于本项目的目录；能力闸不挡删除 —— 清理我们自己写过的副本
/// 是唯一能把未实测目录恢复原样的途径，挡下它反而留下删不掉的东西。
fn apply_delete(
    store: &Store,
    project_id: &str,
    timestamp: i64,
    project_root: &Path,
    action: &SyncAction,
) -> Result<(SyncOutcome, Applied), AppError> {
    let target = action.target_path(project_root)?;
    reject_link(&target, "删除目标")?;
    if !target.is_dir() {
        return Ok((
            SyncOutcome {
                skill_id: action.skill_id.clone(),
                dir: action.dir.clone(),
                action: action.outcome_action().into(),
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
            action: action.outcome_action().into(),
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

/// 手动/自动同步的唯一入口：按真源算渲染计划 → 落盘（先备份、写后回读）→ 刷新基线。
/// `allowed_dirs` 是能力闸允许的渲染目标（`selectable_dir_keys`），写入侧再校验一次，
/// 因为 `targets` 可能来自旧版本、git 合并或手改的 JSON。
pub fn sync_project(
    store: &Store,
    project_id: &str,
    project_root: &Path,
    enabled_agent_ids: &[String],
    allowed_dirs: &[String],
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

    let plan = plan_sync(&ledger, &scan, &unavailable, allowed_dirs);
    let mut report = SyncReport {
        outcomes: Vec::new(),
        conflicts: plan.conflicts.clone(),
        warnings: plan.warnings.clone(),
    };
    report.warnings.extend(scan.warnings.clone());

    // 裁决必须落在真实存在的冲突上：use / keepBoth 指向冲突里的某个目录，
    // purge 只允许用于「账本有真源、磁盘没有真源」——否则等于凭空删用户的副本。
    for resolution in resolutions {
        let Some(conflict) = report
            .conflicts
            .iter()
            .find(|conflict| conflict.skill_id == resolution.skill_id)
        else {
            return Err(AppError::Project(format!(
                "没有需要处理的冲突：{}",
                resolution.skill_id
            )));
        };
        match resolution.action.as_str() {
            "use" | "keepBoth" => {
                if !conflict.dirs.contains(&resolution.dir) {
                    return Err(AppError::Project(format!(
                        "冲突处理无效：{} / {}",
                        resolution.skill_id, resolution.dir
                    )));
                }
            }
            "purge" => {
                if conflict.kind != "canonicalMissing" {
                    return Err(AppError::Project(format!(
                        "{} 不是真源消失，不能删除全部副本",
                        resolution.skill_id
                    )));
                }
            }
            other => {
                return Err(AppError::Project(format!("未知的冲突处理方式：{other}")));
            }
        }
    }

    let mut steps: Vec<Step> = Vec::new();
    let mut applied_skills: BTreeSet<String> = BTreeSet::new();

    // 1. 人工裁决：先把胜出的字节落进真源（Adopt），再按真源渲染其余目标。
    //    顺序很重要 —— 真源没落地之前，渲染来源是不存在的。
    let mut resolved: BTreeSet<String> = BTreeSet::new();
    for conflict in report.conflicts.clone() {
        let Some(resolution) = resolutions
            .iter()
            .find(|item| item.skill_id == conflict.skill_id)
        else {
            continue;
        };
        let Some(entry) = ledger.skills.get(&conflict.skill_id) else {
            continue;
        };
        let blocked = blocked_dirs(entry, &scan, &conflict.skill_id, &unavailable);
        let current = current_dirs(&scan, &conflict.skill_id);
        let renderable: Vec<String> = entry
            .targets
            .iter()
            .filter(|dir| !blocked.contains(dir))
            .cloned()
            .collect();
        let render_step =
            |dir: String| Step::Apply(SyncAction {
                skill_id: conflict.skill_id.clone(),
                kind: SyncKind::Render,
                dir,
                source: Some(SyncSourceRef::Canonical),
            });

        if resolution.action == "purge" {
            for dir in renderable
                .iter()
                .filter(|dir| current.get(*dir).is_some_and(|state| state.present))
            {
                steps.push(Step::Apply(SyncAction {
                    skill_id: conflict.skill_id.clone(),
                    kind: SyncKind::Purge,
                    dir: dir.clone(),
                    source: None,
                }));
            }
        } else if resolution.action == "keepBoth" {
            for dir in &conflict.dirs {
                steps.push(Step::Preserve {
                    skill_id: conflict.skill_id.clone(),
                    dir: dir.clone(),
                });
            }
            steps.extend(renderable.into_iter().map(render_step));
        } else {
            let winner = resolution.dir.clone();
            steps.push(Step::Apply(SyncAction {
                skill_id: conflict.skill_id.clone(),
                kind: SyncKind::Adopt,
                dir: winner.clone(),
                source: Some(SyncSourceRef::Dir(winner.clone())),
            }));
            steps.extend(
                renderable
                    .into_iter()
                    .filter(|dir| *dir != winner)
                    .map(render_step),
            );
        }
        applied_skills.insert(conflict.skill_id.clone());
        resolved.insert(conflict.skill_id.clone());
    }
    report
        .conflicts
        .retain(|conflict| !resolved.contains(&conflict.skill_id));

    // 2. 无歧义的渲染：真源与目标不一致，且没有任何目标被外部改过。
    for action in &plan.actions {
        steps.push(Step::Apply(action.clone()));
        applied_skills.insert(action.skill_id.clone());
    }

    let timestamp = chrono::Utc::now().timestamp_millis();
    let mut applied: Vec<Applied> = Vec::new();
    let mut last_ops: BTreeMap<String, (String, String)> = BTreeMap::new();
    let rollback_all = |done: &[Applied]| -> Vec<String> {
        let mut errors = Vec::new();
        for record in done.iter().rev() {
            if let Err(error) = rollback(record) {
                errors.push(format!("{error}"));
            }
        }
        errors
    };

    for step in &steps {
        let result = match step {
            Step::Preserve { skill_id, dir } => apply_preserve(project_root, skill_id, dir),
            Step::Apply(action) => match action.kind {
                SyncKind::Render | SyncKind::Adopt => {
                    apply_render(store, project_id, timestamp, project_root, action, allowed_dirs)
                }
                SyncKind::Delete | SyncKind::Purge => {
                    apply_delete(store, project_id, timestamp, project_root, action)
                }
            },
        };
        match result {
            Ok((outcome, record)) => {
                if let Step::Apply(action) = step {
                    last_ops
                        .entry(action.skill_id.clone())
                        .or_insert_with(|| {
                            (
                                action.dir.clone(),
                                action.outcome_action().to_string(),
                            )
                        });
                }
                report.outcomes.push(outcome);
                applied.push(record);
            }
            Err(error) => return Err(sync_error(error, rollback_all(&applied))),
        }
    }

    // 账本刷新与写入也在同一个回滚保护圈内：目录改了而账本没跟上，下一轮会重复判定甚至误删。
    let now = chrono::Utc::now().to_rfc3339();
    let mut updated = ledger.clone();
    if let Err(error) = refresh_ledger(
        &mut updated,
        &scan,
        &unavailable,
        &applied_skills,
        &plan.refresh,
        &last_ops,
        project_root,
        &now,
    ) {
        return Err(sync_error(error, rollback_all(&applied)));
    }
    // 空跑（自动同步每次进项目都会来一轮）不该改仓库里要提交的文件。
    if updated != ledger {
        if let Err(error) = project_scan::save_ledger(project_root, &updated) {
            return Err(sync_error(error, rollback_all(&applied)));
        }
    }

    Ok(report)
}

fn sync_error(error: AppError, rollback_errors: Vec<String>) -> AppError {
    if rollback_errors.is_empty() {
        return error;
    }
    AppError::Project(format!(
        "{}；且回滚出现问题：{}",
        error,
        rollback_errors.join("；")
    ))
}

/// 把本轮真正处理过的 skill 的基线刷成磁盘现状（含真源），并记录最近一次操作。
#[allow(clippy::too_many_arguments)]
fn refresh_ledger(
    ledger: &mut ProjectLedger,
    scan: &ProjectScan,
    unavailable: &[String],
    applied_skills: &BTreeSet<String>,
    refresh: &[String],
    last_ops: &BTreeMap<String, (String, String)>,
    project_root: &Path,
    now: &str,
) -> Result<(), AppError> {
    let mut removed: Vec<String> = Vec::new();
    for skill_id in applied_skills.iter().chain(refresh.iter()) {
        let Some(entry) = ledger.skills.get_mut(skill_id) else {
            continue;
        };
        entry.canonical =
            project_scan::tree_state(&crate::domain::project::canonical_skill_dir(
                project_root, skill_id,
            )?)?;
        let blocked = blocked_dirs(entry, scan, skill_id, unavailable);
        for target in entry.targets.clone() {
            if blocked.contains(&target) {
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
                at: now.to_string(),
            });
        }
        ledger.updated_at = now.to_string();
        // v2 的移除条件必须同时看真源：全部副本取消部署是正常状态，
        // 真源还在就必须留条目，否则真源成了再也删不掉的孤儿。
        let has_blocked = entry.targets.iter().any(|target| blocked.contains(target));
        let all_targets_gone = entry
            .targets
            .iter()
            .all(|target| entry.dirs.get(target).is_none_or(|state| !state.present));
        if !has_blocked && applied_skills.contains(skill_id) && all_targets_gone {
            removed.push(skill_id.clone());
        }
    }
    for skill_id in &removed {
        ledger.skills.remove(skill_id);
    }
    Ok(())
}

/// 勾选 / 取消渲染目标：只记账本，不写盘 —— 渲染由下一次同步完成（设计 5.3 的两步流程）。
///
/// 新勾选的目录**不**按磁盘现状写基线。基线的含义是「我们上次渲染到那里的字节」，
/// 把用户自己的内容写成基线，下一轮就会认为它没被改过而直接用真源覆盖；留空基线，
/// `plan_sync` 才会把它判成需要人工裁决而不是可以安全渲染。
pub fn set_targets(
    project_root: &Path,
    skill_id: &str,
    targets: &[String],
    allowed_dirs: &[String],
) -> Result<(), AppError> {
    let mut wanted: Vec<String> = Vec::new();
    for dir in targets {
        if spec_relative_path(dir).is_none() {
            return Err(AppError::Project(format!("未知的项目目录：{dir}")));
        }
        if !allowed_dirs.contains(dir) {
            return Err(AppError::Project(format!(
                "该目录尚未实测，不能作为渲染目标：{dir}"
            )));
        }
        if !wanted.contains(dir) {
            wanted.push(dir.clone());
        }
    }

    let mut ledger = project_scan::load_ledger(project_root)?;
    let entry = ledger
        .skills
        .entry(skill_id.to_string())
        .or_default();
    entry.targets = wanted.clone();
    // 取消勾选不动基线（设计 7.5）：那份基线同时是「这个副本是我们写进去的」的
    // 唯一证明，删掉它就没法再区分我们的渲染产物与用户自己的同名目录。
    // 安全性来自另一头：`ensure_baseline` 与迁移都只为真正渲染过的目标产生基线。
    entry.last_op = Some(ProjectSkillOp {
        dir: wanted.first().cloned().unwrap_or_default(),
        action: "targets".into(),
        at: chrono::Utc::now().to_rfc3339(),
    });
    ledger.updated_at = chrono::Utc::now().to_rfc3339();
    project_scan::save_ledger(project_root, &ledger)
}

/// 移除单个副本：先备份再删，从 targets 移除并把基线置为不存在。
/// 真源还在就必须保留条目 —— 取消部署一个目录不等于卸载这个 skill。
pub fn remove_copy(
    store: &Store,
    project_id: &str,
    project_root: &Path,
    skill_id: &str,
    dir: &str,
) -> Result<SyncReport, AppError> {
    let mut ledger = project_scan::load_ledger(project_root)?;
    let entry = ledger
        .skills
        .get_mut(skill_id)
        .ok_or_else(|| AppError::NotFound(format!("项目里没有跟踪该 skill：{skill_id}")))?;
    if !entry.dirs.get(dir).is_some_and(|state| state.present) {
        return Err(AppError::Project(format!(
            "该副本不是本项目写入的，不能删除：{dir}"
        )));
    }
    entry.targets.retain(|target| target != dir);

    let timestamp = chrono::Utc::now().timestamp_millis();
    let action = SyncAction {
        skill_id: skill_id.to_string(),
        kind: SyncKind::Delete,
        dir: dir.to_string(),
        source: None,
    };
    let (outcome, _record) = apply_delete(store, project_id, timestamp, project_root, &action)?;

    let path = skill_dir(project_root, dir, skill_id)?;
    let entry = ledger
        .skills
        .get_mut(skill_id)
        .expect("条目存在，删除未成功前不会移除");
    entry.dirs.insert(dir.to_string(), project_scan::tree_state(&path)?);
    entry.last_op = Some(ProjectSkillOp {
        dir: dir.to_string(),
        action: "delete".into(),
        at: chrono::Utc::now().to_rfc3339(),
    });
    ledger.updated_at = chrono::Utc::now().to_rfc3339();
    project_scan::save_ledger(project_root, &ledger)?;

    Ok(SyncReport {
        outcomes: vec![outcome],
        conflicts: Vec::new(),
        warnings: Vec::new(),
    })
}

/// 卸载：从所有已跟踪副本删除（先备份），并移除账本条目。
pub fn uninstall(
    store: &Store,
    project_id: &str,
    project_root: &Path,
    skill_id: &str,
) -> Result<SyncReport, AppError> {
    let mut ledger = project_scan::load_ledger(project_root)?;
    let entry = ledger
        .skills
        .get(skill_id)
        .ok_or_else(|| AppError::NotFound(format!("项目里没有跟踪该 skill：{skill_id}")))?;
    let targets = entry.targets.clone();

    let timestamp = chrono::Utc::now().timestamp_millis();
    let mut outcomes: Vec<SyncOutcome> = Vec::new();
    let mut applied: Vec<Applied> = Vec::new();
    let rollback_all = |done: &[Applied]| -> Vec<String> {
        let mut errors = Vec::new();
        for record in done.iter().rev() {
            if let Err(error) = rollback(record) {
                errors.push(format!("{error}"));
            }
        }
        errors
    };
    for dir in &targets {
        if !entry.dirs.get(dir).is_some_and(|state| state.present) {
            continue;
        }
        let action = SyncAction {
            skill_id: skill_id.to_string(),
            kind: SyncKind::Delete,
            dir: dir.clone(),
            source: None,
        };
        match apply_delete(store, project_id, timestamp, project_root, &action) {
            Ok((outcome, record)) => {
                outcomes.push(outcome);
                applied.push(record);
            }
            Err(error) => return Err(sync_error(error, rollback_all(&applied))),
        }
    }
    // 真源与副本在同一批次、同一回滚圈里删除：只删副本会留下没人认领的真源。
    match apply_delete_canonical(store, project_id, timestamp, project_root, skill_id) {
        Ok(Some((outcome, record))) => {
            outcomes.push(outcome);
            applied.push(record);
        }
        Ok(None) => {}
        Err(error) => return Err(sync_error(error, rollback_all(&applied))),
    }

    ledger.skills.remove(skill_id);
    ledger.updated_at = chrono::Utc::now().to_rfc3339();
    if let Err(error) = project_scan::save_ledger(project_root, &ledger) {
        return Err(sync_error(error, rollback_all(&applied)));
    }

    Ok(SyncReport {
        outcomes,
        conflicts: Vec::new(),
        warnings: Vec::new(),
    })
}

/// 删除项目真源目录（先备份）。真源不存在时返回 None，不算失败。
fn apply_delete_canonical(
    store: &Store,
    project_id: &str,
    timestamp: i64,
    project_root: &Path,
    skill_id: &str,
) -> Result<Option<(SyncOutcome, Applied)>, AppError> {
    let target = crate::domain::project::canonical_skill_dir(project_root, skill_id)?;
    if !target.is_dir() {
        return Ok(None);
    }
    reject_link(&target, "删除真源")?;
    let backup = backup_target(store, project_id, timestamp, "canonical", skill_id, &target)?;
    std::fs::remove_dir_all(&target)?;
    Ok(Some((
        SyncOutcome {
            skill_id: skill_id.to_string(),
            dir: "canonical".to_string(),
            action: "delete".into(),
            status: "ok".into(),
            backup_path: Some(backup.clone()),
            error: None,
        },
        Applied {
            created: None,
            restored: Some((target, backup)),
        },
    )))
}

/// 收进真源：把某个渲染目标当前的字节写回真源（先备份旧真源），并刷新真源基线。
/// 用于「项目里已有一份手工 skill，想让它受管」的场景；targets 保持不变。
pub fn adopt(
    store: &Store,
    project_id: &str,
    project_root: &Path,
    skill_id: &str,
    from_dir: &str,
) -> Result<SyncReport, AppError> {
    let source = skill_dir(project_root, from_dir, skill_id)?;
    if !source.join("SKILL.md").is_file() {
        return Err(AppError::Project(format!(
            "{from_dir} 里没有 SKILL.md，不能收进真源"
        )));
    }

    let timestamp = chrono::Utc::now().timestamp_millis();
    let action = SyncAction {
        skill_id: skill_id.to_string(),
        kind: SyncKind::Adopt,
        dir: from_dir.to_string(),
        source: Some(SyncSourceRef::Dir(from_dir.to_string())),
    };
    let (outcome, record) = apply_render(store, project_id, timestamp, project_root, &action, &[])?;

    let written = crate::domain::project::canonical_skill_dir(project_root, skill_id)?;
    let ledger_result = (|| -> Result<(), AppError> {
        let mut ledger = project_scan::load_ledger(project_root)?;
        let now = chrono::Utc::now().to_rfc3339();
        let entry = ledger.skills.entry(skill_id.to_string()).or_default();
        entry.canonical = project_scan::tree_state(&written)?;
        entry.last_op = Some(ProjectSkillOp {
            dir: from_dir.to_string(),
            action: "adopt".into(),
            at: now.clone(),
        });
        ledger.updated_at = now;
        project_scan::save_ledger(project_root, &ledger)
    })();
    if let Err(error) = ledger_result {
        // 真源写了而账本没跟上，下一轮会把这份内容当成外来副本，必须回滚。
        if let Err(rollback_error) = rollback(&record) {
            return Err(AppError::Project(format!(
                "{error}；回滚真源也失败了：{rollback_error}"
            )));
        }
        return Err(AppError::Project(format!("{error}；已回滚真源写入")));
    }

    Ok(SyncReport {
        outcomes: vec![outcome],
        ..Default::default()
    })
}

/// 读项目真源的正文（应用内编辑用）。
pub fn read_body(project_root: &Path, skill_id: &str) -> Result<String, AppError> {
    let path = crate::domain::project::canonical_skill_dir(project_root, skill_id)?.join("SKILL.md");
    if !path.is_file() {
        return Err(AppError::NotFound(format!(
            "项目真源里还没有该 skill：{skill_id}"
        )));
    }
    Ok(String::from_utf8(std::fs::read(&path)?)
        .map_err(|_| AppError::InvalidInput("SKILL.md 不是 UTF-8 文本".into()))?)
}

/// 写项目真源的正文：只替换 `SKILL.md`，同目录附件不动（与中央库同一策略）。
/// 备份 → 原子写 → 回读字节一致；渲染留给下一次同步。
pub fn write_body(
    store: &Store,
    project_id: &str,
    project_root: &Path,
    skill_id: &str,
    content: &str,
) -> Result<SyncReport, AppError> {
    let dir = crate::domain::project::canonical_skill_dir(project_root, skill_id)?;
    let path = dir.join("SKILL.md");
    let bytes = content.as_bytes();
    let timestamp = chrono::Utc::now().timestamp_millis();

    if dir.is_dir() {
        reject_link(&dir, "真源目录")?;
        if path.is_file() {
            backup_target(store, project_id, timestamp, "canonical", skill_id, &dir)?;
        }
    } else {
        std::fs::create_dir_all(&dir)?;
    }
    crate::store::atomic::write_atomic(&path, bytes)
        .map_err(|error| AppError::Project(format!("写入真源失败：{error}")))?;
    if std::fs::read(&path)? != bytes {
        return Err(AppError::Project("真源回读与写入内容不一致".into()));
    }

    let mut ledger = project_scan::load_ledger(project_root)?;
    let entry = ledger.skills.entry(skill_id.to_string()).or_default();
    entry.canonical = project_scan::tree_state(&dir)?;
    entry.last_op = Some(ProjectSkillOp {
        dir: "canonical".to_string(),
        action: "update".into(),
        at: chrono::Utc::now().to_rfc3339(),
    });
    ledger.updated_at = chrono::Utc::now().to_rfc3339();
    project_scan::save_ledger(project_root, &ledger)?;

    Ok(SyncReport {
        outcomes: vec![SyncOutcome {
            skill_id: skill_id.to_string(),
            dir: "canonical".to_string(),
            action: "update".into(),
            status: "ok".into(),
            backup_path: None,
            error: None,
        }],
        conflicts: Vec::new(),
        warnings: Vec::new(),
    })
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

        /// 直接改账本，只用于铺测试数据；走正式入口请用 `set_targets`。
        fn mark_targets(&self, skill_id: &str, dirs: &[&str]) {
            let mut ledger = project_scan::load_ledger(&self.root).unwrap();
            let entry = ledger.skills.entry(skill_id.to_string()).or_default();
            entry.targets = dirs.iter().map(|dir| dir.to_string()).collect();
            project_scan::save_ledger(&self.root, &ledger).unwrap();
        }

        fn set_targets(&self, project_id: &str, skill_id: &str, dirs: &[&str]) {
            self.set_targets_allowed(project_id, skill_id, dirs, &["claude", "qoder"])
                .unwrap();
        }

        fn set_targets_allowed(
            &self,
            _project_id: &str,
            skill_id: &str,
            dirs: &[&str],
            allowed: &[&str],
        ) -> Result<(), AppError> {
            set_targets(
                &self.root,
                skill_id,
                &dirs.iter().map(|dir| dir.to_string()).collect::<Vec<_>>(),
                &allowed.iter().map(|dir| dir.to_string()).collect::<Vec<_>>(),
            )
        }

        fn remove_copy(&self, project_id: &str, skill_id: &str, dir: &str) {
            remove_copy(&self.store, project_id, &self.root, skill_id, dir).unwrap();
        }

        fn uninstall(&self, project_id: &str, skill_id: &str) {
            uninstall(&self.store, project_id, &self.root, skill_id).unwrap();
        }

        fn sync(&self, project_id: &str, resolutions: &[Resolution]) -> Result<SyncReport, AppError> {
            self.sync_allowed(project_id, &["claude", "qoder"], resolutions)
        }

        fn sync_allowed(
            &self,
            project_id: &str,
            allowed: &[&str],
            resolutions: &[Resolution],
        ) -> Result<SyncReport, AppError> {
            sync_project(
                &self.store,
                project_id,
                &self.root,
                &["claude".to_string(), "qoder".to_string()],
                &allowed.iter().map(|dir| dir.to_string()).collect::<Vec<_>>(),
                resolutions,
            )
        }

        /// 把真源写成指定内容（模拟用户在项目真源里编辑），并刷新台账里的真源基线。
        fn write_canonical(&self, skill_id: &str, body: &str) {
            let dir = crate::domain::project::canonical_skill_dir(&self.root, skill_id).unwrap();
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("SKILL.md"), body).unwrap();
        }

        fn adopt(&self, project_id: &str, skill_id: &str, dir: &str) -> Result<SyncReport, AppError> {
            adopt(&self.store, project_id, &self.root, skill_id, dir)
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
            canonical: BTreeMap::new(),
            dirs: Vec::new(),
            warnings: Vec::new(),
            unresolved_dirs: Vec::new(),
            unresolved_skills: Vec::new(),
            unresolved_canonical: Vec::new(),
        }
    }

    /// v2 条目：真源已落地（hash 为 `canonical`），targets 的基线默认与真源一致。
    fn entry_with_canonical(targets: &[&str], canonical: &str) -> ProjectSkillEntry {
        ProjectSkillEntry {
            targets: targets.iter().map(|dir| dir.to_string()).collect(),
            canonical: state(canonical),
            dirs: targets
                .iter()
                .map(|dir| (dir.to_string(), state(canonical)))
                .collect(),
            ..Default::default()
        }
    }

    fn scan_of(
        id: &str,
        dirs: BTreeMap<String, ProjectDirState>,
        canonical: Option<&str>,
    ) -> ProjectScan {
        let mut scan = scan_with(id, dirs);
        if let Some(hash) = canonical {
            scan.canonical
                .insert(id.to_string(), state(hash));
        }
        scan
    }

    fn plan_of(entry: ProjectSkillEntry, scan: ProjectScan, allowed: &[&str]) -> SyncPlan {
        let allowed: Vec<String> = allowed.iter().map(|dir| dir.to_string()).collect();
        plan_sync(
            &ledger_with("a", entry),
            &scan,
            &[],
            &allowed,
        )
    }

    #[test]
    fn only_targets_that_differ_from_the_canonical_are_rendered() {
        let entry = entry_with_canonical(&["claude", "qoder"], "h1");
        let current = states(&[("claude", "h1")]);

        let plan = plan_of(entry, scan_of("a", current, Some("h1")), &["claude", "qoder"]);
        assert_eq!(plan.actions.len(), 1, "claude 已与真源一致，不该重拷");
        assert_eq!(plan.actions[0].dir, "qoder");
        assert_eq!(plan.actions[0].kind, SyncKind::Render);
        assert_eq!(plan.actions[0].source, Some(SyncSourceRef::Canonical));
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn a_changed_canonical_renders_into_every_clean_target() {
        // 基线还是旧真源 h1，磁盘上两个目标也还是 h1，新真源是 h2。
        let entry = entry_with_canonical(&["claude", "qoder"], "h1");
        let current = states(&[("claude", "h1"), ("qoder", "h1")]);

        let plan = plan_of(entry, scan_of("a", current, Some("h2")), &["claude", "qoder"]);
        assert_eq!(plan.actions.len(), 2);
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn an_externally_edited_target_waits_for_a_decision_instead_of_being_overwritten() {
        // 真源上次是 h1，现在变成 h2；claude 仍是 h1（干净、只是落后），qoder 被外部改成 h9。
        let entry = entry_with_canonical(&["claude", "qoder"], "h1");
        let current = states(&[("claude", "h1"), ("qoder", "h9")]);

        let plan = plan_of(entry, scan_of("a", current, Some("h2")), &["claude", "qoder"]);
        assert!(
            plan.actions.is_empty(),
            "有任何被改目标就整条等待，不做半应用"
        );
        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflicts[0].kind, "targetVsCanonical");
        assert_eq!(plan.conflicts[0].dirs, vec!["qoder".to_string()]);
    }

    #[test]
    fn a_target_already_matching_the_canonical_only_needs_a_baseline_refresh() {
        // 基线还是 h1，真源已经是 h2，而两个目标的内容都恰好等于 h2
        // （例如用户手工拷过去了）：不该报冲突，也不该重拷，只需刷基线。
        let entry = entry_with_canonical(&["claude", "qoder"], "h1");
        let current = states(&[("claude", "h2"), ("qoder", "h2")]);

        let plan = plan_of(entry, scan_of("a", current, Some("h2")), &["claude", "qoder"]);
        assert!(plan.actions.is_empty());
        assert!(plan.conflicts.is_empty());
        assert_eq!(plan.refresh, vec!["a".to_string()]);
    }

    #[test]
    fn unverified_targets_are_skipped_at_plan_time_with_an_explained_warning() {
        let entry = entry_with_canonical(&["claude", "qoder"], "h1");
        let current = states(&[("claude", "h1")]);

        // 只有 claude 过了能力闸：qoder 一个文件都收不到，并且要说明原因。
        let plan = plan_of(entry, scan_of("a", current, Some("h2")), &["claude"]);
        assert_eq!(plan.actions.len(), 1);
        assert_eq!(plan.actions[0].dir, "claude");
        assert!(plan
            .warnings
            .iter()
            .any(|warning| warning.contains("qoder") && warning.contains("未实测")));
    }

    #[test]
    fn an_entry_without_a_canonical_does_nothing() {
        // 新项目：磁盘上有副本，但还没人「收进真源」。
        let entry = entry_with_targets(&["claude"]);
        let current = states(&[("claude", "h1")]);

        let plan = plan_of(entry, scan_of("a", current, None), &["claude"]);
        assert!(plan.actions.is_empty());
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn a_missing_canonical_reports_a_conflict_instead_of_deleting_copies() {
        let entry = entry_with_canonical(&["claude", "qoder"], "h1");
        let current = states(&[("claude", "h1"), ("qoder", "h1")]);

        let plan = plan_of(entry, scan_of("a", current, None), &["claude", "qoder"]);
        assert!(plan.actions.is_empty());
        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflicts[0].kind, "canonicalMissing");
        assert_eq!(plan.conflicts[0].dirs.len(), 2);
    }

    #[test]
    fn a_blocked_target_never_drives_a_render() {
        let entry = entry_with_canonical(&["claude", "qoder"], "h1");
        let current = states(&[("qoder", "h1")]);
        let mut scan = scan_of("a", current, Some("h2"));
        scan.unresolved_skills = vec![("claude".to_string(), "a".to_string())];

        let plan = plan_sync(
            &ledger_with("a", entry),
            &scan,
            &[],
            &["claude".to_string(), "qoder".to_string()],
        );
        assert_eq!(plan.actions.len(), 1);
        assert_eq!(plan.actions[0].dir, "qoder");
        assert!(plan.warnings.iter().any(|w| w.contains("claude")));
    }

    #[test]
    fn a_blocked_canonical_skips_the_entry_entirely() {
        let entry = entry_with_canonical(&["claude"], "h1");
        let current = states(&[("claude", "h1")]);
        let mut scan = scan_of("a", current, Some("h2"));
        scan.unresolved_canonical = vec!["a".to_string()];

        let plan = plan_sync(
            &ledger_with("a", entry),
            &scan,
            &[],
            &["claude".to_string()],
        );
        assert!(plan.actions.is_empty(), "真源状态取不到时不能按旧内容渲染");
        assert!(plan.warnings.iter().any(|w| w.contains("真源")));
    }

    /// 真源落地后，同步的唯一职责就是把真源渲染到勾选的目标上。
    #[test]
    fn sync_renders_the_canonical_into_a_target_that_lacks_it() {
        let fixture = project_fixture();
        let view = fixture.add_project(&["claude".into(), "qoder".into()]);
        fixture.write_canonical("a", "---\nname: a\n---\n真源内容\n");
        fixture.mark_targets("a", &["claude"]);

        let report = fixture.sync(&view.id, &[]).unwrap();
        assert_eq!(report.outcomes.len(), 1);
        assert_eq!(report.outcomes[0].action, "render");
        assert_eq!(report.outcomes[0].dir, "claude");
        let rendered = fixture.root.join(".claude/skills/a/SKILL.md");
        assert_eq!(std::fs::read_to_string(&rendered).unwrap(), "---\nname: a\n---\n真源内容\n");

        let ledger = project_scan::load_ledger(&fixture.root).unwrap();
        assert!(ledger.skills["a"].dirs["claude"].present);
        assert_eq!(
            ledger.skills["a"].dirs["claude"].tree_hash,
            ledger.skills["a"].canonical.tree_hash,
            "渲染成功后基线必须等于真源"
        );
    }

    #[test]
    fn sync_never_overwrites_an_externally_edited_target_then_adopts_the_winner() {
        let fixture = project_fixture();
        let view = fixture.add_project(&["claude".into(), "qoder".into()]);
        fixture.write_canonical("a", "真源 v1\n");
        fixture.mark_targets("a", &["claude", "qoder"]);
        fixture.sync(&view.id, &[]).unwrap();

        // 用户手工改了 claude 的副本，同时真源也变了：谁都不该被静默覆盖。
        std::fs::write(fixture.root.join(".claude/skills/a/SKILL.md"), "用户的手改\n").unwrap();
        fixture.write_canonical("a", "真源 v2\n");

        let report = fixture.sync(&view.id, &[]).unwrap();
        assert!(report.outcomes.is_empty(), "被改目标未裁决前不写盘");
        assert_eq!(report.conflicts[0].kind, "targetVsCanonical");
        assert_eq!(report.conflicts[0].dirs, vec!["claude".to_string()]);
        assert_eq!(
            std::fs::read_to_string(fixture.root.join(".claude/skills/a/SKILL.md")).unwrap(),
            "用户的手改\n"
        );

        // 裁决「以 claude 为真源」：手改进真源，其余目标按新真源渲染。
        let report = fixture
            .sync(
                &view.id,
                &[Resolution {
                    skill_id: "a".to_string(),
                    dir: "claude".to_string(),
                    action: "use".to_string(),
                }],
            )
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(fixture.root.join(".ccswitchx/skills/a/SKILL.md")).unwrap(),
            "用户的手改\n"
        );
        assert_eq!(
            std::fs::read_to_string(fixture.root.join(".qoder/skills/a/SKILL.md")).unwrap(),
            "用户的手改\n",
            "其余目标要跟上被采纳的内容"
        );
        assert!(report.outcomes.iter().any(|item| item.action == "adopt"));
    }

    #[test]
    fn keep_both_preserves_the_edited_copy_as_an_untracked_sibling() {
        let fixture = project_fixture();
        let view = fixture.add_project(&["claude".into(), "qoder".into()]);
        fixture.write_canonical("a", "真源 v1\n");
        fixture.mark_targets("a", &["claude", "qoder"]);
        fixture.sync(&view.id, &[]).unwrap();
        std::fs::write(fixture.root.join(".claude/skills/a/SKILL.md"), "用户的手改\n").unwrap();
        fixture.write_canonical("a", "真源 v2\n");

        fixture
            .sync(
                &view.id,
                &[Resolution {
                    skill_id: "a".to_string(),
                    dir: "claude".to_string(),
                    action: "keepBoth".to_string(),
                }],
            )
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(fixture.root.join(".claude/skills/a/SKILL.md")).unwrap(),
            "真源 v2\n"
        );
        assert_eq!(
            std::fs::read_to_string(fixture.root.join(".claude/skills/a-copy/SKILL.md")).unwrap(),
            "用户的手改\n",
            "落败的手改必须留在原地，只是不再受管"
        );
    }

    #[test]
    fn unchecking_every_target_keeps_the_entry_while_the_canonical_exists() {
        let fixture = project_fixture();
        let view = fixture.add_project(&["claude".into(), "qoder".into()]);
        fixture.write_canonical("a", "真源内容\n");
        fixture.mark_targets("a", &["claude"]);
        fixture.sync(&view.id, &[]).unwrap();

        // 取消所有渲染目标：副本被删，但真源和条目都还在，否则真源成了孤儿。
        fixture.set_targets(&view.id, "a", &[]);
        fixture.remove_copy(&view.id, "a", "claude");
        let ledger = project_scan::load_ledger(&fixture.root).unwrap();
        assert!(
            ledger.skills.contains_key("a"),
            "真源还在就不能移除条目"
        );
        assert!(ledger.skills["a"].canonical.present);
        assert!(fixture.root.join(".ccswitchx/skills/a/SKILL.md").is_file());
        assert!(!fixture.root.join(".claude/skills/a").exists());
    }

    #[test]
    fn uninstall_deletes_the_canonical_and_every_copy_in_one_batch() {
        let fixture = project_fixture();
        let view = fixture.add_project(&["claude".into(), "qoder".into()]);
        fixture.write_canonical("a", "真源内容\n");
        fixture.mark_targets("a", &["claude", "qoder"]);
        fixture.sync(&view.id, &[]).unwrap();

        fixture.uninstall(&view.id, "a");
        assert!(!fixture.root.join(".ccswitchx/skills/a").exists());
        assert!(!fixture.root.join(".claude/skills/a").exists());
        assert!(!fixture.root.join(".qoder/skills/a").exists());
        assert!(
            project_scan::load_ledger(&fixture.root)
                .unwrap()
                .skills
                .is_empty()
        );
        assert!(backup_root(&fixture.store).exists(), "删除前必须留备份");
    }

    #[test]
    fn adopt_writes_the_copy_back_into_the_canonical() {
        let fixture = project_fixture();
        seed_skill(&fixture.root.join(".claude/skills"), "mine");
        let view = fixture.add_project(&["claude".into(), "qoder".into()]);

        fixture.adopt(&view.id, "mine", "claude").unwrap();
        let canonical = fixture.root.join(".ccswitchx/skills/mine/SKILL.md");
        assert_eq!(
            std::fs::read_to_string(&canonical).unwrap(),
            std::fs::read_to_string(fixture.root.join(".claude/skills/mine/SKILL.md")).unwrap()
        );
        let ledger = project_scan::load_ledger(&fixture.root).unwrap();
        assert!(ledger.skills["mine"].canonical.present);
        assert!(ledger.skills["mine"].targets.is_empty(), "采纳不改变渲染目标");
    }

    #[test]
    fn purge_needs_an_explicit_resolution_and_then_removes_the_entry() {
        let fixture = project_fixture();
        let view = fixture.add_project(&["claude".into(), "qoder".into()]);
        fixture.write_canonical("a", "真源内容\n");
        fixture.mark_targets("a", &["claude", "qoder"]);
        fixture.sync(&view.id, &[]).unwrap();
        std::fs::remove_dir_all(fixture.root.join(".ccswitchx/skills/a")).unwrap();

        // 自动同步绝不代用户决定删副本。
        let report = fixture.sync(&view.id, &[]).unwrap();
        assert!(report.outcomes.is_empty());
        assert_eq!(report.conflicts[0].kind, "canonicalMissing");
        assert!(fixture.root.join(".claude/skills/a").is_dir());

        fixture
            .sync(
                &view.id,
                &[Resolution {
                    skill_id: "a".to_string(),
                    dir: String::new(),
                    action: "purge".to_string(),
                }],
            )
            .unwrap();
        assert!(!fixture.root.join(".claude/skills/a").exists());
        assert!(!fixture.root.join(".qoder/skills/a").exists());
        assert!(
            project_scan::load_ledger(&fixture.root)
                .unwrap()
                .skills
                .is_empty(),
            "真源与全部副本都没了，条目才该移除"
        );
        assert!(backup_root(&fixture.store).exists());
    }

    #[test]
    fn rendering_refuses_a_target_that_is_not_selectable() {
        let fixture = project_fixture();
        let view = fixture.add_project(&["claude".into(), "qoder".into()]);
        fixture.write_canonical("a", "真源内容\n");
        fixture.mark_targets("a", &["claude", "qoder"]);

        // 只允许 claude：qoder 是未实测/未启用的落点，一个文件都不能写进去。
        let report = fixture.sync_allowed(&view.id, &["claude"], &[]).unwrap();
        assert!(report.outcomes.iter().all(|item| item.dir == "claude"));
        assert!(!fixture.root.join(".qoder/skills/a").exists());
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("qoder")));
    }

    #[test]
    fn purge_is_refused_for_a_conflict_that_is_not_a_missing_canonical() {
        let fixture = project_fixture();
        let view = fixture.add_project(&["claude".into(), "qoder".into()]);
        fixture.write_canonical("a", "真源 v1\n");
        fixture.mark_targets("a", &["claude", "qoder"]);
        fixture.sync(&view.id, &[]).unwrap();
        std::fs::write(fixture.root.join(".claude/skills/a/SKILL.md"), "手改\n").unwrap();
        fixture.write_canonical("a", "真源 v2\n");

        let error = fixture
            .sync(
                &view.id,
                &[Resolution {
                    skill_id: "a".to_string(),
                    dir: "claude".to_string(),
                    action: "purge".to_string(),
                }],
            )
            .unwrap_err();
        assert!(error.to_string().contains("不能删除全部副本"), "{error}");
        assert!(fixture.root.join(".claude/skills/a/SKILL.md").is_file());
    }

    /// 渲染产物是可丢弃的：手工删掉一个副本，下一轮按真源重新渲染出来；
    /// 真正删除要走 remove_copy / uninstall（都会先备份）。
    #[test]
    fn deleting_a_rendered_copy_by_hand_just_renders_it_again_and_removal_backs_up() {
        let fixture = project_fixture();
        let view = fixture.add_project(&["claude".into(), "qoder".into()]);
        fixture.write_canonical("a", "真源内容\n");
        fixture.mark_targets("a", &["claude", "qoder"]);
        fixture.sync(&view.id, &[]).unwrap();
        assert!(fixture.root.join(".qoder/skills/a/SKILL.md").is_file());

        std::fs::remove_dir_all(fixture.root.join(".qoder/skills/a")).unwrap();
        let report = fixture.sync(&view.id, &[]).unwrap();
        assert!(report.conflicts.is_empty());
        assert!(
            fixture.root.join(".qoder/skills/a/SKILL.md").is_file(),
            "真源还在，副本就该被重新渲染出来"
        );

        fixture.remove_copy(&view.id, "a", "qoder");
        assert!(!fixture.root.join(".qoder/skills/a").exists());
        assert!(
            backup_root(&fixture.store).join("projects").is_dir(),
            "删除副本前必须留备份"
        );
    }

    #[test]
    fn two_externally_edited_targets_are_a_conflict_and_write_nothing() {
        let fixture = project_fixture();
        let view = fixture.add_project(&["claude".into(), "qoder".into()]);
        fixture.write_canonical("a", "真源 v1\n");
        fixture.mark_targets("a", &["claude", "qoder"]);
        fixture.sync(&view.id, &[]).unwrap();

        std::fs::write(fixture.root.join(".claude/skills/a/SKILL.md"), "claude 手改\n").unwrap();
        std::fs::write(fixture.root.join(".qoder/skills/a/SKILL.md"), "qoder 手改\n").unwrap();
        let before_claude = std::fs::read(fixture.root.join(".claude/skills/a/SKILL.md")).unwrap();

        let report = fixture.sync(&view.id, &[]).unwrap();
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.conflicts[0].kind, "targetVsCanonical");
        assert_eq!(report.conflicts[0].dirs.len(), 2);
        assert!(report.outcomes.is_empty());
        assert_eq!(
            std::fs::read(fixture.root.join(".claude/skills/a/SKILL.md")).unwrap(),
            before_claude
        );
        assert_eq!(
            std::fs::read_to_string(fixture.root.join(".ccswitchx/skills/a/SKILL.md")).unwrap(),
            "真源 v1\n",
            "冲突期间真源也不能被自动改写"
        );
    }

    #[test]
    fn checking_a_target_waits_for_sync_then_renders() {
        let fixture = project_fixture();
        let view = fixture.add_project(&["claude".into(), "qoder".into()]);
        fixture.write_canonical("a", "真源内容\n");
        fixture.set_targets(&view.id, "a", &["claude"]);
        fixture.sync(&view.id, &[]).unwrap();

        fixture.set_targets(&view.id, "a", &["claude", "qoder"]);
        assert!(
            !fixture.root.join(".qoder/skills/a").exists(),
            "勾选只写账本，不该替用户点同步"
        );

        fixture.sync(&view.id, &[]).unwrap();
        assert_eq!(
            std::fs::read_to_string(fixture.root.join(".qoder/skills/a/SKILL.md")).unwrap(),
            "真源内容\n"
        );
    }

    #[test]
    fn foreign_content_on_a_newly_checked_target_becomes_a_conflict_not_an_error() {
        let fixture = project_fixture();
        let qoder = fixture.root.join(".qoder/skills/a");
        std::fs::create_dir_all(&qoder).unwrap();
        std::fs::write(qoder.join("SKILL.md"), "用户自己写的同名 skill").unwrap();
        let view = fixture.add_project(&["claude".into(), "qoder".into()]);
        fixture.write_canonical("a", "真源内容\n");
        fixture.mark_targets("a", &["claude"]);
        fixture.sync(&view.id, &[]).unwrap();

        // 新勾选 qoder，但那里已有一份不是我们写进去的同名目录：只能问用户。
        fixture.set_targets(&view.id, "a", &["claude", "qoder"]);
        let report = fixture.sync(&view.id, &[]).unwrap();

        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.conflicts[0].dirs, vec!["qoder".to_string()]);
        assert!(report.outcomes.is_empty());
        assert_eq!(
            std::fs::read(qoder.join("SKILL.md")).unwrap(),
            "用户自己写的同名 skill".as_bytes(),
            "外来内容一个字节都不能被覆盖"
        );
    }

    #[test]
    fn untracking_keeps_files_but_removing_copy_deletes_them() {
        let fixture = project_fixture();
        let view = fixture.add_project(&["claude".into(), "qoder".into()]);
        fixture.write_canonical("a", "真源内容\n");
        fixture.set_targets(&view.id, "a", &["claude", "qoder"]);
        fixture.sync(&view.id, &[]).unwrap();

        fixture.set_targets(&view.id, "a", &["claude"]);
        assert!(
            fixture.root.join(".qoder/skills/a").is_dir(),
            "取消勾选保留文件，之后按未跟踪处理"
        );
        fixture.write_canonical("a", "真源 v2\n");
        let report = fixture.sync(&view.id, &[]).unwrap();
        assert!(
            report.outcomes.iter().all(|item| item.dir != "qoder"),
            "未跟踪的副本不再被渲染"
        );
        assert_eq!(
            std::fs::read_to_string(fixture.root.join(".qoder/skills/a/SKILL.md")).unwrap(),
            "真源内容\n"
        );

        fixture.remove_copy(&view.id, "a", "qoder");
        assert!(!fixture.root.join(".qoder/skills/a").exists());
        assert!(fixture.root.join(".claude/skills/a").is_dir());
    }

    /// 无事可做的空跑（自动同步每次进项目都会来一轮）不该改仓库里要提交的文件。
    #[test]
    fn sync_without_anything_to_do_leaves_the_ledger_file_untouched() {
        let fixture = project_fixture();
        let view = fixture.add_project(&["claude".into(), "qoder".into()]);
        fixture.write_canonical("a", "真源内容\n");
        fixture.mark_targets("a", &["claude", "qoder"]);
        fixture.sync(&view.id, &[]).unwrap();

        let path = fixture.root.join(".ccswitchx").join("skills.json");
        let before = std::fs::read(&path).unwrap();
        let report = fixture.sync(&view.id, &[]).unwrap();
        assert!(report.outcomes.is_empty());
        assert_eq!(std::fs::read(&path).unwrap(), before, "空跑不该改仓库里的账本");
    }

    /// 账本里出现无法解析的 target key（手改或 git 合并留下的），只能跳过并提示，
    /// 不能借它删掉健康副本，也不能拿它当渲染目标。
    #[test]
    fn unresolvable_target_key_is_skipped_and_reported() {
        let fixture = project_fixture();
        let view = fixture.add_project(&["claude".into(), "qoder".into()]);
        fixture.write_canonical("a", "真源内容\n");
        fixture.mark_targets("a", &["claude"]);
        fixture.sync(&view.id, &[]).unwrap();
        fixture.mark_targets("a", &["claude", "bogus"]);

        let report = fixture.sync(&view.id, &[]).unwrap();
        assert!(report.outcomes.is_empty());
        assert!(fixture.root.join(".claude/skills/a").is_dir());
        assert!(report.warnings.iter().any(|w| w.contains("bogus")));
    }

    #[test]
    fn set_targets_rejects_dirs_that_are_not_selectable() {
        let fixture = project_fixture();
        seed_skill(&fixture.root.join(".claude/skills"), "a");
        let view = fixture.add_project(&["claude".into(), "qoder".into()]);

        let error = fixture
            .set_targets_allowed(&view.id, "a", &["qoder"], &["claude"])
            .unwrap_err();
        assert!(matches!(error, AppError::Project(_)));
        assert!(!fixture.root.join(".qoder/skills/a").exists());
    }
}
