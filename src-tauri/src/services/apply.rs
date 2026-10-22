use crate::domain::{AssetState, AssetType};
use crate::error::AppError;
use crate::index::{AgentStateRow, Index};
use crate::store::{atomic, Store};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Enable,
    Disable,
    Refresh,
}

/// 一条已经渲染好的写入意图。`contents` 由适配器算出（含键级合并的结果），
/// 流水线只管「备份 → 写 → 回读 → 记录 → 失败回滚」，不掺渲染逻辑。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanItem {
    pub agent_id: String,
    pub asset_type: AssetType,
    pub asset_id: String,
    pub action: Action,
    pub target_path: PathBuf,
    #[serde(skip)]
    pub contents: Vec<u8>,
    pub diff: Vec<String>,
    pub needs_restart: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyReport {
    pub written: usize,
    pub skipped: usize,
    pub ok: bool,
    pub backup_dir: PathBuf,
    pub error: Option<String>,
}

/// 回滚一项所需的信息：目标文件、写入前的备份（None 表示当时目标不存在，回滚即删除）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RollbackEntry {
    target: PathBuf,
    backup: Option<PathBuf>,
    agent_id: String,
    asset_type: AssetType,
    asset_id: String,
}

pub struct ApplyService;

impl ApplyService {
    pub fn apply(
        store: &Store,
        index: &mut Index,
        plan: &[PlanItem],
    ) -> Result<ApplyReport, AppError> {
        let backup_dir = store
            .root()
            .join("backups")
            .join(chrono::Utc::now().timestamp_millis().to_string());

        // 1. 先备份全部已存在的目标：这一步失败时还没有任何写入，直接报错返回
        let mut work: Vec<(PlanItem, RollbackEntry)> = Vec::with_capacity(plan.len());
        for item in plan {
            let backup = Self::back_up(&backup_dir, item)?;
            work.push((
                item.clone(),
                RollbackEntry {
                    target: item.target_path.clone(),
                    backup,
                    agent_id: item.agent_id.clone(),
                    asset_type: item.asset_type,
                    asset_id: item.asset_id.clone(),
                },
            ));
        }

        // 2. 逐项写入并回读确认；任一项失败就把已写项全部还原
        let mut touched: Vec<usize> = Vec::new();
        let mut written = 0usize;
        let mut skipped = 0usize;
        for slot in 0..work.len() {
            let item = &work[slot].0;
            if std::fs::read(&item.target_path)
                .map(|current| current == item.contents)
                .unwrap_or(false)
            {
                skipped += 1;
                continue;
            }
            if let Err(error) = atomic::write_atomic(&item.target_path, &item.contents) {
                let failure = format!("写入 {} 失败：{error}", item.target_path.display());
                Self::restore(index, &work, &touched)?;
                return Err(AppError::InvalidInput(failure));
            }
            let read_back = std::fs::read(&item.target_path)?;
            if read_back != item.contents {
                let failure = format!("写入 {} 后回读不一致", item.target_path.display());
                Self::restore(index, &work, &touched)?;
                return Err(AppError::InvalidInput(failure));
            }
            touched.push(slot);
            written += 1;
        }

        // 3. 记录四态与执行历史
        for (item, _) in &work {
            let state = match item.action {
                Action::Disable => AssetState::Off,
                _ if item.needs_restart => AssetState::Written,
                _ => AssetState::Effective,
            };
            index.set_agent_state(&AgentStateRow {
                agent_id: item.agent_id.clone(),
                asset_type: item.asset_type,
                asset_id: item.asset_id.clone(),
                state,
                last_error: None,
                applied_at: chrono::Utc::now().timestamp_millis(),
            })?;
        }
        let entries: Vec<RollbackEntry> = work.iter().map(|(_, entry)| entry.clone()).collect();
        index.record_run(
            false,
            &serde_json::to_string(plan)?,
            &serde_json::to_string(&entries)?,
            &backup_dir,
        )?;

        Ok(ApplyReport {
            written,
            skipped,
            ok: true,
            backup_dir,
            error: None,
        })
    }

    /// 回滚最近一次未被回滚的执行。回滚后这些项的期望与磁盘不再一致，标成有漂移。
    pub fn revert_last(index: &mut Index) -> Result<ApplyReport, AppError> {
        let run = index
            .last_unreverted_run()?
            .ok_or_else(|| AppError::NotFound("没有可回滚的执行记录".into()))?;
        let entries: Vec<RollbackEntry> = serde_json::from_str(&run.result)?;
        let mut restored = 0usize;
        for entry in &entries {
            match &entry.backup {
                Some(backup) if backup.exists() => {
                    std::fs::copy(backup, &entry.target)?;
                    restored += 1;
                }
                None if entry.target.exists() => {
                    std::fs::remove_file(&entry.target)?;
                    restored += 1;
                }
                _ => {}
            }
            index.set_agent_state(&AgentStateRow {
                agent_id: entry.agent_id.clone(),
                asset_type: entry.asset_type,
                asset_id: entry.asset_id.clone(),
                state: AssetState::Drifted,
                last_error: Some("已回滚到本次执行前的状态".into()),
                applied_at: chrono::Utc::now().timestamp_millis(),
            })?;
        }
        index.mark_run_reverted(run.id)?;
        Ok(ApplyReport {
            written: restored,
            skipped: 0,
            ok: true,
            backup_dir: run.backup_dir,
            error: None,
        })
    }

    fn back_up(backup_dir: &Path, item: &PlanItem) -> Result<Option<PathBuf>, AppError> {
        if !item.target_path.exists() {
            return Ok(None);
        }
        let target = backup_dir
            .join(&item.agent_id)
            .join(item.asset_type.dir_name())
            .join(&item.asset_id)
            .join(
                item.target_path
                    .file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new("target")),
            );
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&item.target_path, &target)?;
        Ok(Some(target))
    }

    /// 把 `touched` 里的项按倒序还原；`touched` 之外的项还没写过，不动。
    fn restore(
        index: &mut Index,
        work: &[(PlanItem, RollbackEntry)],
        touched: &[usize],
    ) -> Result<(), AppError> {
        for slot in touched.iter().rev() {
            let (_, entry) = &work[*slot];
            match &entry.backup {
                Some(backup) if backup.exists() => {
                    std::fs::copy(backup, &entry.target)?;
                }
                None => {
                    if entry.target.exists() {
                        std::fs::remove_file(&entry.target)?;
                    }
                }
                Some(_) => {}
            }
            index.set_agent_state(&AgentStateRow {
                agent_id: entry.agent_id.clone(),
                asset_type: entry.asset_type,
                asset_id: entry.asset_id.clone(),
                state: AssetState::Off,
                last_error: Some("整批写入失败，已回滚".into()),
                applied_at: chrono::Utc::now().timestamp_millis(),
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AssetType;
    use crate::index::Index;
    use crate::store::Store;
    use tempfile::tempdir;

    fn fixture() -> (tempfile::TempDir, Store, Index) {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let index = Index::open(&dir.path().join("index.db")).unwrap();
        (dir, store, index)
    }

    fn item(target: &Path, contents: &[u8]) -> PlanItem {
        PlanItem {
            agent_id: "claude".into(),
            asset_type: AssetType::Mcp,
            asset_id: "pw".into(),
            action: Action::Enable,
            target_path: target.to_path_buf(),
            contents: contents.to_vec(),
            diff: vec![format!(
                "mcpServers.pw => {}",
                String::from_utf8_lossy(contents)
            )],
            needs_restart: false,
        }
    }

    #[test]
    fn apply_writes_targets_and_records_state_per_item() {
        let (dir, store, mut index) = fixture();
        let target = dir.path().join("home/.claude.json");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"{\"permissions\":{}}").unwrap();

        let report =
            ApplyService::apply(&store, &mut index, &[item(&target, b"{\"a\":1}")]).unwrap();
        assert_eq!(report.written, 1);
        assert!(report.ok);
        assert_eq!(std::fs::read(&target).unwrap(), b"{\"a\":1}");

        let states = index.agent_states().unwrap();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].state, AssetState::Effective);
        assert_eq!(states[0].asset_id, "pw");

        // 写入前的原件必须留在备份里
        let backed_up = read_all(&report.backup_dir);
        assert_eq!(backed_up, vec![b"{\"permissions\":{}}".to_vec()]);
    }

    #[test]
    fn a_failing_item_aborts_the_batch_and_restores_earlier_ones() {
        let (dir, store, mut index) = fixture();
        let good = dir.path().join("home/good.json");
        std::fs::create_dir_all(good.parent().unwrap()).unwrap();
        std::fs::write(&good, b"ORIGINAL").unwrap();
        // 第二个目标的父路径是个已存在的文件，create_dir_all 必然失败
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"not a dir").unwrap();
        let bad = blocker.join("nested/bad.json");

        let failure =
            ApplyService::apply(&store, &mut index, &[item(&good, b"NEW"), item(&bad, b"X")])
                .expect_err("写失败必须报错，不能吞掉");
        assert!(failure.to_string().contains("bad.json"), "{failure}");
        assert_eq!(
            std::fs::read(&good).unwrap(),
            b"ORIGINAL",
            "整批中止必须把先写成功的项回滚"
        );
        let states = index.agent_states().unwrap();
        assert_eq!(states[0].state, AssetState::Off, "回滚后不能留下已生效状态");
        assert!(states[0].last_error.is_some());
    }

    #[test]
    fn rerunning_the_same_plan_writes_nothing() {
        let (dir, store, mut index) = fixture();
        let target = dir.path().join("home/t.json");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        let plan = [item(&target, b"{\"a\":1}")];

        let first = ApplyService::apply(&store, &mut index, &plan).unwrap();
        let second = ApplyService::apply(&store, &mut index, &plan).unwrap();
        assert_eq!(first.written, 1);
        assert_eq!(second.written, 0, "已生效项不得重复写");
        assert_eq!(second.skipped, 1);
    }

    #[test]
    fn items_needing_restart_are_recorded_as_written_not_effective() {
        let (dir, store, mut index) = fixture();
        let target = dir.path().join("home/q.json");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        let mut plan = item(&target, b"{\"a\":1}");
        plan.needs_restart = true;
        plan.agent_id = "qoder-cn".into();

        ApplyService::apply(&store, &mut index, &[plan]).unwrap();
        let states = index.agent_states().unwrap();
        assert_eq!(states[0].state, AssetState::Written);
    }

    #[test]
    fn disable_writes_the_merged_file_and_clears_the_state_row() {
        let (dir, store, mut index) = fixture();
        let target = dir.path().join("home/d.json");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        ApplyService::apply(&store, &mut index, &[item(&target, b"{\"a\":1}")]).unwrap();

        let mut off = item(&target, b"{}");
        off.action = Action::Disable;
        ApplyService::apply(&store, &mut index, &[off]).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"{}");
        let states = index.agent_states().unwrap();
        assert_eq!(states.len(), 1, "同一资产同一 agent 只应有一行状态");
        assert_eq!(states[0].state, AssetState::Off);
    }

    #[test]
    fn revert_last_restores_backed_up_files_and_removes_new_ones() {
        let (dir, store, mut index) = fixture();
        let existing = dir.path().join("home/existing.json");
        let created = dir.path().join("home/created.json");
        std::fs::create_dir_all(existing.parent().unwrap()).unwrap();
        std::fs::write(&existing, b"OLD").unwrap();

        ApplyService::apply(
            &store,
            &mut index,
            &[item(&existing, b"NEW"), item(&created, b"BRAND NEW")],
        )
        .unwrap();
        assert_eq!(std::fs::read(&created).unwrap(), b"BRAND NEW");

        let report = ApplyService::revert_last(&mut index).unwrap();
        assert_eq!(report.written, 2);
        assert_eq!(
            std::fs::read(&existing).unwrap(),
            b"OLD",
            "原有文件回滚到备份内容"
        );
        assert!(!created.exists(), "本次新建的文件回滚后必须消失");
        assert_eq!(index.agent_states().unwrap()[0].state, AssetState::Drifted);

        assert!(
            ApplyService::revert_last(&mut index).is_err(),
            "同一次执行不能被回滚两遍"
        );
    }

    fn read_all(root: &Path) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else {
                        out.push(std::fs::read(path).unwrap());
                    }
                }
            }
        }
        out.sort();
        out
    }
}
