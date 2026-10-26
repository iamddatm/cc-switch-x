//! v1「对等副本」台账 → v2「项目内真源」的显式迁移。
//!
//! 三条纪律决定了这个模块的写法：
//! 1. **只新建文件**：迁移把某个目录的现有字节复制进 `.ccswitchx/skills/<id>/`，
//!    绝不改动任何 agent 目录 —— 那些是用户的 git 内容。
//! 2. **读路径不写盘**：计划由 `plan_migration` 纯计算得出，落盘只发生在
//!    `apply_migration`，且必须由用户在界面上确认过 `pending_writes`。
//! 3. **不确定就不动**：多个来源内容不一致、或真源已存在又不等于拟采纳内容时，
//!    一律列为冲突要求人工裁决；未裁决整体拒绝，不做部分写入。

use crate::domain::project::{
    canonical_root, canonical_skill_dir, ProjectDirState, ProjectLedger, PROJECT_LEDGER_VERSION,
    PROJECT_SKILL_DIRS,
};
use crate::error::AppError;
use crate::services::project_scan::{self, ProjectScan};
use crate::store::{copy_dir, Store};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 会把字节新建进真源的采纳来源；`files` 已经是相对项目根的完整路径。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationItem {
    pub skill_id: String,
    /// 采纳来源的目录 key；来源内容一致时取字典序最小，保证可复现。
    pub from_dir: String,
    pub files: Vec<String>,
}

/// 必须人工裁决的一条。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationConflict {
    pub skill_id: String,
    pub dirs: Vec<String>,
    /// mtime 最新的一侧，仅作推荐，不代表它更新。
    pub recommended: Option<String>,
    /// `targetsDiffer` | `unresolvable`
    pub kind: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMigrationPlan {
    /// targets 里就有内容且各 target 一致：采纳是确定的，确认后会执行。
    pub adopts: Vec<MigrationItem>,
    /// 从没勾选过目标、但目录里有副本：是否收进真源由用户决定。
    /// 多份副本内容完全一致时默认采纳（怎么都不会丢东西）。
    pub proposals: Vec<MigrationProposal>,
    pub conflicts: Vec<MigrationConflict>,
    /// 台账里有、磁盘上什么都没有：只去条目，不写任何文件。
    pub drops: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationProposal {
    #[serde(flatten)]
    pub item: MigrationItem,
    pub candidates: Vec<String>,
    pub default_adopt: bool,
    pub reason: String,
}

impl ProjectMigrationPlan {
    pub fn is_empty(&self) -> bool {
        self.adopts.is_empty()
            && self.proposals.is_empty()
            && self.conflicts.is_empty()
            && self.drops.is_empty()
    }

    /// 确认之前必须给用户看的清单：会在你仓库里新建/重写哪些文件。
    pub fn pending_writes(&self) -> Vec<String> {
        if self.is_empty() {
            return Vec::new();
        }
        let mut writes: Vec<String> = self.adopts.iter().flat_map(|a| a.files.clone()).collect();
        for proposal in self.proposals.iter().filter(|item| item.default_adopt) {
            writes.extend(proposal.item.files.clone());
        }
        writes.push(project_scan::LEDGER_RELATIVE.to_string());
        writes.sort();
        writes
    }
}

/// 用户对一条冲突 / 一个候选的裁决。`dir` 为空串表示这条跳过不动。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationDecision {
    pub skill_id: String,
    pub dir: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMigrationReport {
    pub adopted: Vec<String>,
    pub skipped: Vec<String>,
    pub dropped: Vec<String>,
    pub warnings: Vec<String>,
    pub ledger_backup: Option<PathBuf>,
    pub version: u32,
}

fn dir_keys() -> Vec<String> {
    PROJECT_SKILL_DIRS
        .iter()
        .map(|spec| spec.key.as_str().to_string())
        .collect()
}

fn relative_of(key: &str) -> Option<&'static str> {
    PROJECT_SKILL_DIRS
        .iter()
        .find(|spec| spec.key.as_str() == key)
        .map(|spec| spec.relative_path)
}

/// 采纳后将在真源里出现的文件（相对项目根）。
fn canonical_files_of(id: &str, state: &ProjectDirState) -> Vec<String> {
    state
        .files
        .keys()
        .map(|file| format!("{}/{id}/{file}", project_scan::CANONICAL_RELATIVE, id = id))
        .collect()
}

/// 纯计算：不写任何文件。真源已落地的条目直接跳过，所以迁移过一次就不会再出计划。
pub fn plan_migration(ledger: &ProjectLedger, scan: &ProjectScan) -> ProjectMigrationPlan {
    let mut plan = ProjectMigrationPlan::default();

    for (id, entry) in &ledger.skills {
        if scan.blocks_canonical(id) {
            plan.conflicts.push(MigrationConflict {
                skill_id: id.clone(),
                dirs: dir_keys(),
                recommended: None,
                kind: "unresolvable".into(),
            });
            continue;
        }
        if scan.canonical.get(id).is_some_and(|state| state.present) {
            continue;
        }

        let states: std::collections::BTreeMap<String, ProjectDirState> = scan
            .skills
            .get(id)
            .map(|snapshot| snapshot.dirs.clone())
            .unwrap_or_default();
        let present_in = |keys: &[String]| -> Vec<String> {
            keys.iter()
                .filter(|key| {
                    !scan.blocks(key, id) && states.get(*key).is_some_and(|state| state.present)
                })
                .cloned()
                .collect()
        };
        let present_all = present_in(&dir_keys());
        let present_targets = present_in(&entry.targets);
        let blocked_targets: Vec<String> = entry
            .targets
            .iter()
            .filter(|key| scan.blocks(key, id))
            .cloned()
            .collect();

        if present_all.is_empty() {
            if blocked_targets.is_empty() {
                plan.drops.push(id.clone());
            } else {
                // 状态取不到的格子不能当「不存在」，否则会把健康副本判成已删除。
                plan.conflicts.push(MigrationConflict {
                    skill_id: id.clone(),
                    dirs: blocked_targets,
                    recommended: None,
                    kind: "unresolvable".into(),
                });
            }
            continue;
        }

        let chosen = if present_targets.is_empty() {
            present_all.clone()
        } else {
            present_targets.clone()
        };
        let hashes: BTreeSet<Option<String>> = chosen
            .iter()
            .map(|key| states.get(key).and_then(|state| state.tree_hash.clone()))
            .collect();
        if hashes.len() > 1 {
            let recommended = chosen
                .iter()
                .max_by_key(|key| states.get(*key).map_or(0, |state| state.mtime))
                .cloned();
            plan.conflicts.push(MigrationConflict {
                skill_id: id.clone(),
                dirs: chosen,
                recommended,
                kind: "targetsDiffer".into(),
            });
            continue;
        }

        let from_dir = chosen.iter().min().cloned().expect("chosen 非空");
        let item = MigrationItem {
            files: states
                .get(&from_dir)
                .map(|state| canonical_files_of(id, state))
                .unwrap_or_default(),
            skill_id: id.clone(),
            from_dir,
        };
        if present_targets.is_empty() {
            plan.proposals.push(MigrationProposal {
                candidates: chosen,
                default_adopt: present_all.len() > 1,
                reason: if present_all.len() > 1 {
                    "各目录副本内容一致，收进真源不会丢内容".into()
                } else {
                    "该 skill 还没被勾选为渲染目标，收进真源后才会参与同步".into()
                },
                item,
            });
        } else {
            plan.adopts.push(item);
        }
    }

    plan.warnings.extend(scan.warnings.clone());
    plan
}

fn backup_ledger(
    store: &Store,
    project_id: &str,
    timestamp: i64,
    project_root: &Path,
) -> Result<Option<PathBuf>, AppError> {
    let source = project_scan::ledger_path(project_root);
    if !source.is_file() {
        return Ok(None);
    }
    let target = store
        .root()
        .join("backups")
        .join("projects")
        .join(project_id)
        .join(timestamp.to_string())
        .join("ledger")
        .join("skills.json");
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(&source, &target)?;
    Ok(Some(target))
}

/// 删除本次新建的真源目录并从备份还原台账。agent 目录从头到尾没被碰过，无需还原。
fn rollback(created: &[PathBuf], backup: &Option<PathBuf>, project_root: &Path) {
    for path in created {
        if path.is_dir() {
            let _ = std::fs::remove_dir_all(path);
        }
    }
    if let Some(backup) = backup {
        if let Ok(bytes) = std::fs::read(backup) {
            let _ = crate::store::atomic::write_atomic(
                &project_scan::ledger_path(project_root),
                &bytes,
            );
        }
    }
}

/// 落盘迁移。调用方必须已经让用户看过 `pending_writes` 并确认。
pub fn apply_migration(
    store: &Store,
    project_id: &str,
    project_root: &Path,
    ledger: &ProjectLedger,
    scan: &ProjectScan,
    decisions: &[MigrationDecision],
) -> Result<ProjectMigrationReport, AppError> {
    if !project_scan::ledger_needs_migration(ledger) {
        return Err(AppError::Project("该项目台账已经是当前版本，无需迁移".into()));
    }
    let plan = plan_migration(ledger, scan);
    if plan.is_empty() {
        return Ok(ProjectMigrationReport {
            version: ledger.version,
            ..Default::default()
        });
    }
    let undecided: Vec<&str> = plan
        .conflicts
        .iter()
        .filter(|conflict| {
            !decisions
                .iter()
                .any(|item| item.skill_id == conflict.skill_id)
        })
        .map(|conflict| conflict.skill_id.as_str())
        .collect();
    if !undecided.is_empty() {
        return Err(AppError::Project(format!(
            "仍有 {} 个冲突未裁决（{}），未写入任何内容",
            undecided.len(),
            undecided.join("、")
        )));
    }

    let targets_of = |id: &str| -> Vec<String> {
        ledger
            .skills
            .get(id)
            .map(|entry| entry.targets.clone())
            .unwrap_or_default()
    };
    let mut wanted: Vec<(String, String, Vec<String>)> = plan
        .adopts
        .iter()
        .map(|adopt| (adopt.skill_id.clone(), adopt.from_dir.clone(), targets_of(&adopt.skill_id)))
        .collect();
    let mut skipped: Vec<String> = Vec::new();

    for proposal in &plan.proposals {
        let id = &proposal.item.skill_id;
        match decisions.iter().find(|item| item.skill_id == *id) {
            Some(decision) if decision.dir.is_empty() => skipped.push(id.clone()),
            Some(decision) => {
                if !proposal.candidates.contains(&decision.dir) {
                    return Err(AppError::Project(format!(
                        "候选 {} 的采纳目录无效：{}",
                        id, decision.dir
                    )));
                }
                wanted.push((id.clone(), decision.dir.clone(), targets_of(id)));
            }
            None if proposal.default_adopt => {
                wanted.push((id.clone(), proposal.item.from_dir.clone(), targets_of(id)));
            }
            None => skipped.push(id.clone()),
        }
    }
    for conflict in &plan.conflicts {
        let Some(decision) = decisions.iter().find(|item| item.skill_id == conflict.skill_id)
        else {
            continue;
        };
        if decision.dir.is_empty() {
            skipped.push(conflict.skill_id.clone());
            continue;
        }
        if conflict.kind == "targetsDiffer" && !conflict.dirs.contains(&decision.dir) {
            return Err(AppError::Project(format!(
                "冲突 {} 的裁决目录无效：{}",
                conflict.skill_id, decision.dir
            )));
        }
        wanted.push((conflict.skill_id.clone(), decision.dir.clone(), targets_of(&conflict.skill_id)));
    }

    let timestamp = chrono::Utc::now().timestamp_millis();
    let mut report = ProjectMigrationReport {
        version: PROJECT_LEDGER_VERSION,
        warnings: plan.warnings.clone(),
        ..Default::default()
    };
    let mut created: Vec<PathBuf> = Vec::new();

    let result = (|| -> Result<(), AppError> {
        report.ledger_backup = backup_ledger(store, project_id, timestamp, project_root)?;
        let mut next = ledger.clone();

        for (id, from_dir, targets) in &wanted {
            let relative = relative_of(from_dir)
                .ok_or_else(|| AppError::Project(format!("未知的项目目录：{from_dir}")))?;
            let source = project_root.join(relative).join(id);
            if project_scan::is_link(&source) {
                return Err(AppError::Project(format!(
                    "拒绝迁移符号链接目录：{}",
                    source.display()
                )));
            }
            if !source.join("SKILL.md").is_file() {
                return Err(AppError::Project(format!(
                    "迁移来源不是一个 skill 目录：{}",
                    source.display()
                )));
            }
            let target = canonical_skill_dir(project_root, id)?;
            let source_state = project_scan::tree_state(&source)?;
            if target.exists() {
                // 幂等靠这一支：内容相同当作已采纳；不同则拒绝，绝不覆盖已有真源。
                if project_scan::tree_state(&target)? == source_state {
                    skipped.push(id.clone());
                } else {
                    return Err(AppError::Project(format!(
                        "项目真源已存在且内容不同，拒绝覆盖：{}",
                        target.display()
                    )));
                }
            } else {
                let root = canonical_root(project_root);
                std::fs::create_dir_all(&root)?;
                let staging = root.join(format!(".staging-{}-{}", timestamp, std::process::id()));
                if staging.exists() {
                    std::fs::remove_dir_all(&staging)?;
                }
                copy_dir(&source, &staging)?;
                std::fs::rename(&staging, &target)?;
                created.push(target.clone());
                if project_scan::tree_state(&target)? != source_state {
                    return Err(AppError::Project(format!(
                        "{id} 的真源回读校验与来源不一致"
                    )));
                }
            }

            let canonical = project_scan::tree_state(&target)?;
            let entry = next
                .skills
                .get_mut(id)
                .ok_or_else(|| AppError::Project(format!("台账里没有该 skill：{id}")))?;
            entry.canonical = canonical.clone();
            entry.targets = targets.clone();
            // 渲染基线取真源：与真源不一致的目录会被判成 edited 要求人工裁决，
            // 而不是「落后了就覆盖」—— 迁移本身不该替用户决定谁更新。
            entry.dirs = PROJECT_SKILL_DIRS
                .iter()
                .map(|spec| {
                    let key = spec.key.as_str();
                    let state = if targets.iter().any(|target| target == key) {
                        canonical.clone()
                    } else {
                        scan.skills
                            .get(id)
                            .and_then(|snapshot| snapshot.dirs.get(key))
                            .cloned()
                            .unwrap_or_default()
                    };
                    (key.to_string(), state)
                })
                .collect();
            report.adopted.push(id.clone());
        }

        for id in &plan.drops {
            next.skills.remove(id);
            report.dropped.push(id.clone());
        }

        next.version = PROJECT_LEDGER_VERSION;
        next.updated_at = chrono::Utc::now().to_rfc3339();
        project_scan::save_ledger(project_root, &next)?;

        // 回读：台账写坏了就等于丢掉用户的归属信息，必须当场发现。
        // 按序列化后的形态比，`mtime` 之类不持久化的字段才不会造成假阴性。
        let read_back = project_scan::load_ledger(project_root)?;
        let same = serde_json::to_value(&read_back.skills)? == serde_json::to_value(&next.skills)?;
        if read_back.version != PROJECT_LEDGER_VERSION || !same {
            return Err(AppError::Project("迁移后的台账回读不一致".into()));
        }
        Ok(())
    })();

    if let Err(error) = result {
        rollback(&created, &report.ledger_backup, project_root);
        return Err(AppError::Project(format!(
            "{error}；迁移已回滚，未改动任何 agent 目录"
        )));
    }
    report.skipped.extend(skipped);
    report.skipped.sort();
    report.skipped.dedup();
    report.adopted.sort();
    report.dropped.sort();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::project::{ProjectSkillDirKey, ProjectSkillEntry};
    use crate::store::Store;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    struct Fixture {
        _temp: tempfile::TempDir,
        store: Store,
        root: PathBuf,
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::init(temp.path().join("store")).unwrap();
        let root = temp.path().join("home/work/app");
        std::fs::create_dir_all(&root).unwrap();
        Fixture {
            _temp: temp,
            store,
            root,
        }
    }

    /// 一个 skill 目录：SKILL.md + 一个附件，用来验证整目录复制。
    fn seed(dir: &Path, body: &str) {
        std::fs::create_dir_all(dir.join("references")).unwrap();
        std::fs::write(dir.join("SKILL.md"), body).unwrap();
        std::fs::write(dir.join("references/a.md"), "附件").unwrap();
    }

    fn claude(fx: &Fixture, id: &str, body: &str) -> PathBuf {
        let path = fx.root.join(".claude/skills").join(id);
        seed(&path, body);
        path
    }

    fn qoder(fx: &Fixture, id: &str, body: &str) -> PathBuf {
        let path = fx.root.join(".qoder/skills").join(id);
        seed(&path, body);
        path
    }

    /// 手工造 v1 台账：targets 与 dirs 基线按当前磁盘状态填。
    fn write_v1_ledger(fx: &Fixture, entries: &[(&str, &[&str])]) {
        let scan = project_scan::scan(&fx.root, &ProjectSkillDirKey::ALL).unwrap();
        let mut ledger = ProjectLedger {
            version: 1,
            updated_at: String::new(),
            skills: BTreeMap::new(),
        };
        for (id, targets) in entries {
            let dirs = scan
                .skills
                .get(*id)
                .map(|snapshot| snapshot.dirs.clone())
                .unwrap_or_default();
            ledger.skills.insert(
                id.to_string(),
                ProjectSkillEntry {
                    targets: targets.iter().map(|key| key.to_string()).collect(),
                    source: None,
                    dirs,
                    ..Default::default()
                },
            );
        }
        project_scan::save_ledger(&fx.root, &ledger).unwrap();
    }

    fn plan(fx: &Fixture) -> ProjectMigrationPlan {
        let ledger = project_scan::load_ledger(&fx.root).unwrap();
        let scan = project_scan::scan(&fx.root, &ProjectSkillDirKey::ALL).unwrap();
        plan_migration(&ledger, &scan)
    }

    fn apply(fx: &Fixture, decisions: &[(&str, &str)]) -> Result<ProjectMigrationReport, AppError> {
        let ledger = project_scan::load_ledger(&fx.root).unwrap();
        let scan = project_scan::scan(&fx.root, &ProjectSkillDirKey::ALL).unwrap();
        let decisions: Vec<MigrationDecision> = decisions
            .iter()
            .map(|(skill_id, dir)| MigrationDecision {
                skill_id: skill_id.to_string(),
                dir: dir.to_string(),
            })
            .collect();
        apply_migration(&fx.store, "app-1", &fx.root, &ledger, &scan, &decisions)
    }

    fn tree_hash(path: &Path) -> Option<String> {
        project_scan::tree_state(path).ok()?.tree_hash
    }

    #[test]
    fn plan_adopts_agreeing_targets_from_the_lexically_smallest_dir() {
        let fx = fixture();
        claude(&fx, "a", "同样的内容");
        qoder(&fx, "a", "同样的内容");
        write_v1_ledger(&fx, &[("a", &["claude", "qoder"])]);

        let plan = plan(&fx);
        assert_eq!(plan.adopts.len(), 1);
        assert_eq!(plan.conflicts, vec![]);
        assert_eq!(
            plan.adopts[0].from_dir, "claude",
            "来源必须确定，不能依赖 mtime"
        );
        assert!(plan.adopts[0]
            .files
            .contains(&".ccswitchx/skills/a/references/a.md".to_string()));
    }

    #[test]
    fn plan_conflicts_when_targets_differ() {
        let fx = fixture();
        claude(&fx, "a", "一边");
        qoder(&fx, "a", "另一边");
        write_v1_ledger(&fx, &[("a", &["claude", "qoder"])]);

        let plan = plan(&fx);
        assert!(plan.adopts.is_empty());
        assert_eq!(plan.conflicts.len(), 1);
        assert_eq!(plan.conflicts[0].kind, "targetsDiffer");
        assert_eq!(plan.conflicts[0].dirs.len(), 2);
    }

    #[test]
    fn apply_refuses_while_any_conflict_is_undecided_and_writes_nothing() {
        let fx = fixture();
        let source = claude(&fx, "a", "一边");
        qoder(&fx, "a", "另一边");
        write_v1_ledger(&fx, &[("a", &["claude", "qoder"])]);
        let before = tree_hash(&source);

        let error = apply(&fx, &[]).unwrap_err();
        assert!(error.to_string().contains("未裁决"), "{error}");
        assert!(!canonical_root(&fx.root).exists(), "整体拒绝时不能留下半个真源");
        assert_eq!(tree_hash(&source), before);
        assert_eq!(
            project_scan::load_ledger(&fx.root).unwrap().version,
            1,
            "台账不能被改动"
        );
    }

    #[test]
    fn apply_writes_only_the_canonical_and_leaves_agent_dirs_byte_identical() {
        let fx = fixture();
        let claude_dir = claude(&fx, "a", "内容");
        let qoder_dir = qoder(&fx, "a", "内容");
        write_v1_ledger(&fx, &[("a", &["claude", "qoder"])]);
        let before = (tree_hash(&claude_dir), tree_hash(&qoder_dir));

        let report = apply(&fx, &[]).unwrap();
        assert_eq!(report.adopted, vec!["a".to_string()]);
        assert_eq!(report.version, PROJECT_LEDGER_VERSION);

        let canonical = canonical_skill_dir(&fx.root, "a").unwrap();
        assert_eq!(tree_hash(&canonical), before.0, "真源必须等于采纳来源");
        assert_eq!(
            (tree_hash(&claude_dir), tree_hash(&qoder_dir)),
            before,
            "迁移只新建文件，agent 目录一个字节都不能动"
        );
        assert!(canonical.join("references/a.md").is_file(), "整目录复制，附件也要在");

        let ledger = project_scan::load_ledger(&fx.root).unwrap();
        assert_eq!(ledger.version, PROJECT_LEDGER_VERSION);
        assert!(ledger.skills["a"].canonical.present);
        // 基线取真源：不一致的目录会被判成 edited 要求人工裁决，而不是被覆盖。
        assert_eq!(
            ledger.skills["a"].dirs["claude"].tree_hash,
            ledger.skills["a"].canonical.tree_hash
        );
    }

    #[test]
    fn apply_is_idempotent_and_the_second_plan_is_empty() {
        let fx = fixture();
        claude(&fx, "a", "内容");
        write_v1_ledger(&fx, &[("a", &["claude"])]);

        apply(&fx, &[]).unwrap();
        assert!(plan(&fx).is_empty(), "迁完之后不该再出计划");
        let second = apply(&fx, &[]);
        // 台账已是 v2，第二次调用直接被版本闸挡下，不会重复写。
        assert!(second.is_err());
    }

    #[test]
    fn unchecked_copies_are_proposed_and_default_to_off_for_a_single_dir() {
        let fx = fixture();
        claude(&fx, "a", "只有一份");
        write_v1_ledger(&fx, &[("a", &[])]);

        let p = plan(&fx);
        assert!(p.adopts.is_empty());
        assert_eq!(p.proposals.len(), 1);
        assert!(!p.proposals[0].default_adopt, "只有一份副本时不该替用户决定");
        let created: Vec<String> = p
            .pending_writes()
            .into_iter()
            .filter(|write| write.starts_with(&format!("{}/", project_scan::CANONICAL_RELATIVE)))
            .collect();
        assert!(
            created.is_empty(),
            "默认不采纳就不该新建真源文件：{created:?}"
        );

        // 勾选采纳才落地。
        apply(&fx, &[("a", "claude")]).unwrap();
        assert!(canonical_skill_dir(&fx.root, "a").unwrap().is_dir());
    }

    #[test]
    fn identical_unchecked_copies_default_to_adopt() {
        let fx = fixture();
        claude(&fx, "a", "两边一样");
        qoder(&fx, "a", "两边一样");
        write_v1_ledger(&fx, &[("a", &[])]);

        let p = plan(&fx);
        assert!(p.proposals[0].default_adopt);
        assert!(p
            .pending_writes()
            .contains(&".ccswitchx/skills/a/SKILL.md".to_string()));
        assert!(p
            .pending_writes()
            .contains(&project_scan::LEDGER_RELATIVE.to_string()));

        apply(&fx, &[]).unwrap();
        assert!(canonical_skill_dir(&fx.root, "a").unwrap().is_dir());
    }

    #[test]
    fn entries_with_no_content_anywhere_are_dropped_without_writing_files() {
        let fx = fixture();
        write_v1_ledger(&fx, &[("gone", &["claude"])]);

        let p = plan(&fx);
        assert_eq!(p.drops, vec!["gone".to_string()]);
        let report = apply(&fx, &[]).unwrap();
        assert_eq!(report.dropped, vec!["gone".to_string()]);
        assert!(report.adopted.is_empty());
        assert!(
            !canonical_root(&fx.root).exists(),
            "无事可做时不该在仓库里建目录"
        );
        assert!(project_scan::load_ledger(&fx.root)
            .unwrap()
            .skills
            .is_empty());
    }

    #[test]
    fn apply_backs_up_the_old_ledger_and_can_restore_it_when_a_write_fails() {
        let fx = fixture();
        claude(&fx, "a", "内容");
        claude(&fx, "b", "也要迁");
        write_v1_ledger(&fx, &[("a", &["claude"]), ("b", &["claude"])]);
        // b 的真源位置被用户手建了一个没有 SKILL.md 的目录：扫描不算它存在，
        // 计划里会有 b，落盘时才发现会覆盖别人的内容 —— 必须拒绝并回滚 a。
        let blocked = canonical_skill_dir(&fx.root, "b").unwrap();
        std::fs::create_dir_all(&blocked).unwrap();
        std::fs::write(blocked.join("README.md"), "用户自己的东西").unwrap();

        let error = apply(&fx, &[]).unwrap_err();
        assert!(error.to_string().contains("回滚"), "{error}");
        assert!(error.to_string().contains("拒绝覆盖"), "{error}");
        assert_eq!(
            project_scan::load_ledger(&fx.root).unwrap().version,
            1,
            "失败必须把台账还原"
        );
        assert!(
            !canonical_skill_dir(&fx.root, "a").unwrap().exists(),
            "本次新建的真源目录必须被删掉"
        );
        assert!(
            blocked.join("README.md").is_file(),
            "回滚不能顺手删掉用户自己建的目录内容"
        );
        let backup = report_ledger_backup(&fx);
        let text = std::fs::read_to_string(backup).unwrap();
        assert!(
            text.contains(r#""version": 1"#),
            "备份里必须是迁移前的 v1 台账：{text}"
        );
    }

    /// 从备份目录里找出唯一一份台账备份。
    fn report_ledger_backup(fx: &Fixture) -> PathBuf {
        let base = fx.store.root().join("backups/projects/app-1");
        let mut found: Vec<PathBuf> = Vec::new();
        let mut stack = vec![base];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.file_name().is_some_and(|name| name == "skills.json") {
                    found.push(path);
                }
            }
        }
        assert_eq!(found.len(), 1, "应当且只应当留一份台账备份");
        found.remove(0)
    }

    #[test]
    fn plan_skips_entries_whose_canonical_already_exists() {
        let fx = fixture();
        claude(&fx, "a", "内容");
        seed(&canonical_skill_dir(&fx.root, "a").unwrap(), "内容");
        write_v1_ledger(&fx, &[("a", &["claude"])]);

        assert!(plan(&fx).is_empty());
    }

    #[test]
    fn blocked_canonical_requires_a_decision_instead_of_silently_passing() {
        let fx = fixture();
        claude(&fx, "a", "内容");
        write_v1_ledger(&fx, &[("a", &["claude"])]);
        let ledger = project_scan::load_ledger(&fx.root).unwrap();
        let mut scan = project_scan::scan(&fx.root, &ProjectSkillDirKey::ALL).unwrap();
        scan.unresolved_canonical.push("a".to_string());

        let p = plan_migration(&ledger, &scan);
        assert_eq!(p.conflicts[0].kind, "unresolvable");
        assert!(p.adopts.is_empty());
    }
}
