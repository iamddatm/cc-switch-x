use crate::domain::project::{ProjectDirState, ProjectLedger, ProjectSkillEntry};
use crate::services::project_scan::ProjectScan;
use std::collections::BTreeMap;

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

#[derive(Debug, Clone, PartialEq, Eq)]
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

/// 把「最后操作为准」的规则换算成可执行计划：单变化传播，多变化要么视为同一次操作、要么冲突。
pub fn plan_sync(ledger: &ProjectLedger, scan: &ProjectScan, unavailable: &[String]) -> SyncPlan {
    let mut plan = SyncPlan::default();

    for (skill_id, entry) in &ledger.skills {
        let current = scan
            .skills
            .get(skill_id)
            .map(|snapshot| snapshot.dirs.clone())
            .unwrap_or_default();
        let changes: Vec<DirChange> = diff_entry(entry, &current)
            .into_iter()
            .filter(|change| !unavailable.contains(&change.dir))
            .collect();
        if changes.is_empty() {
            continue;
        }

        if changes.len() == 1 {
            let change = &changes[0];
            for target in &entry.targets {
                if target == &change.dir || unavailable.contains(target) {
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
            for target in &entry.targets {
                if unavailable.contains(target) {
                    plan.warnings.push(format!(
                        "{skill_id}：目标 {target} 当前不可用，同步已跳过"
                    ));
                }
            }
            continue;
        }

        // 多个目录变成同一份内容（或一起消失）＝同一次操作，只需刷新基线。
        if changes.windows(2).all(|pair| {
            same_state(&pair[0].state, &pair[1].state)
                && (pair[0].kind == ChangeKind::Delete) == (pair[1].kind == ChangeKind::Delete)
        }) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::project::{ProjectDirState, ProjectLedger, ProjectSkillEntry};
    use crate::services::project_scan::{ProjectScan, ProjectSkillSnapshot};
    use std::collections::BTreeMap;

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
}
