//! 自定义斜杠命令的落盘形态。canonical 真源是**逐字保留**的 markdown
//! （`assets/commands/<id>.md`），这里不做任何渲染：Claude Code 直接读
//! `~/.claude/commands/<id>.md`，frontmatter（description / argument-hint /
//! allowed-tools …）与正文都属于用户内容，重排会制造无意义的 diff。
//!
//! 归属判定与 rules / subagents 共用 `ledger::bytes_are_ours`：字节相等即认。

use crate::adapters::ledger;
use crate::adapters::paths::expand_root;
use crate::domain::agent::AgentDescriptor;
use crate::domain::{AssetKind, AssetType};
use crate::error::AppError;
use crate::index::{sha256_hex, Index};
use crate::services::apply::{Action, ApplyReport, ApplyService, PlanItem};
use crate::store::Store;
use std::path::{Path, PathBuf};

/// 重新加载语义没有实测依据，按「要重启」处理，不把未知报成安全。
const NEEDS_RESTART: bool = true;

/// 命令格式没有派生产物，渲染器版本恒为 1。
const RENDERER_VERSION: u32 = 1;

use super::{Deployed, Removed};

fn declared_commands_dir(agent: &AgentDescriptor) -> Result<&str, AppError> {
    agent
        .paths
        .commands_dir
        .as_deref()
        .ok_or_else(|| AppError::Unsupported(format!("{} 没有命令落点", agent.id)))
}

pub fn commands_dir(
    agent: &AgentDescriptor,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<PathBuf, AppError> {
    Ok(expand_root(
        declared_commands_dir(agent)?,
        base_override,
        home,
    ))
}

pub fn target_path(
    agent: &AgentDescriptor,
    command_id: &str,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<PathBuf, AppError> {
    Ok(commands_dir(agent, home, base_override)?.join(format!("{command_id}.md")))
}

fn plan_item(
    agent: &AgentDescriptor,
    command_id: &str,
    target_path: PathBuf,
    contents: Vec<u8>,
    action: Action,
) -> PlanItem {
    let marker = match action {
        Action::Remove => '-',
        _ => '+',
    };
    PlanItem {
        agent_id: agent.id.clone(),
        asset_type: AssetType::Command,
        asset_id: command_id.to_string(),
        action,
        target_path,
        contents,
        diff: vec![format!(
            "{marker} {}/{}",
            declared_commands_dir(agent).unwrap_or("commands"),
            command_id
        )],
        needs_restart: NEEDS_RESTART,
    }
}

fn read_if_exists(path: &Path) -> Result<Option<Vec<u8>>, AppError> {
    if path.exists() {
        Ok(Some(std::fs::read(path)?))
    } else {
        Ok(None)
    }
}

/// 真源可能已被删除；那种情况下归属只认台账（`bytes_are_ours` 的空 candidates 分支）。
fn read_canonical(store: &Store, command_id: &str) -> Option<String> {
    let bytes = store.read(AssetType::Command, command_id).ok()?;
    String::from_utf8(bytes).ok()
}

fn is_ours(
    store: &Store,
    agent: &AgentDescriptor,
    command_id: &str,
    path: &Path,
    current: &[u8],
) -> Result<bool, AppError> {
    let candidates = read_canonical(store, command_id).map(Ok).into_iter();
    ledger::bytes_are_ours(
        store,
        &agent.id,
        AssetKind::Command,
        command_id,
        path,
        current,
        candidates,
    )
}

/// `force=false` 时，目标上是不认识的内容就返回 `None`（一个字节不动）。
fn deploy_inner(
    store: &Store,
    index: &mut Index,
    agent: &AgentDescriptor,
    command_id: &str,
    home: &Path,
    base_override: Option<&Path>,
    force: bool,
) -> Result<Option<ApplyReport>, AppError> {
    let canonical = store.read(AssetType::Command, command_id)?;
    String::from_utf8(canonical.clone())
        .map_err(|_| AppError::InvalidInput("真源里的命令不是 UTF-8".into()))?;
    let path = target_path(agent, command_id, home, base_override)?;

    if !force {
        if let Some(current) = read_if_exists(&path)? {
            if !is_ours(store, agent, command_id, &path, &current)? {
                return Ok(None);
            }
        }
    }
    let report = ApplyService::apply(
        store,
        index,
        &[plan_item(
            agent,
            command_id,
            path.clone(),
            canonical.clone(),
            Action::Enable,
        )],
    )?;
    ledger::record(
        store,
        &agent.id,
        AssetKind::Command,
        command_id,
        &path,
        &sha256_hex(&canonical),
        RENDERER_VERSION,
    )?;
    Ok(Some(report))
}

pub fn deploy(
    store: &Store,
    index: &mut Index,
    agent: &AgentDescriptor,
    command_id: &str,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<Deployed, AppError> {
    Ok(
        match deploy_inner(store, index, agent, command_id, home, base_override, false)? {
            Some(report) => Deployed::Written(report),
            None => Deployed::Foreign,
        },
    )
}

/// 「重刷真源」：用户已经看过漂移并选了这一项，所以不再被 Foreign 拦住。
/// 写入仍走同一条流水线（备份、回读、失败回滚都在）。
pub fn repush(
    store: &Store,
    index: &mut Index,
    agent: &AgentDescriptor,
    command_id: &str,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<ApplyReport, AppError> {
    deploy_inner(store, index, agent, command_id, home, base_override, true)?.ok_or_else(|| {
        AppError::InvalidInput(format!(
            "{command_id} 的落盘目标无法处理，未做任何改动"
        ))
    })
}

/// 取消落盘。只删得出「是我们落的」文件；用户手改过的自家文件也算 Foreign，
/// 那是漂移该报告的东西，不该在这里被静默删掉。
pub fn remove(
    store: &Store,
    index: &mut Index,
    agent: &AgentDescriptor,
    command_id: &str,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<Removed, AppError> {
    let path = target_path(agent, command_id, home, base_override)?;
    let Some(current) = read_if_exists(&path)? else {
        ledger::forget(store, &agent.id, AssetKind::Command, command_id)?;
        return Ok(Removed::Absent);
    };
    if !is_ours(store, agent, command_id, &path, &current)? {
        return Ok(Removed::Foreign);
    }
    ApplyService::apply(
        store,
        index,
        &[plan_item(agent, command_id, path, Vec::new(), Action::Remove)],
    )?;
    ledger::forget(store, &agent.id, AssetKind::Command, command_id)?;
    Ok(Removed::Ours)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::builtin_agents;
    use tempfile::tempdir;

    struct Fixture {
        _dir: tempfile::TempDir,
        store: Store,
        index: Index,
        home: PathBuf,
        agent: AgentDescriptor,
    }

    fn fixture() -> Fixture {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let index = Index::open(&dir.path().join("index.db")).unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        Fixture {
            _dir: dir,
            store,
            index,
            home,
            agent: builtin_agents()
                .into_iter()
                .find(|a| a.id == "claude")
                .unwrap(),
        }
    }

    impl Fixture {
        fn commands_dir(&self) -> PathBuf {
            self.home.join(".claude/commands")
        }

        fn target(&self) -> PathBuf {
            target_path(&self.agent, "review", &self.home, None).unwrap()
        }

        fn seed(&self, text: &str) {
            self.store
                .write(AssetType::Command, "review", text.as_bytes())
                .unwrap();
        }

        fn deploy(&mut self) -> Deployed {
            deploy(
                &self.store,
                &mut self.index,
                &self.agent,
                "review",
                &self.home,
                None,
            )
            .unwrap()
        }
    }

    #[test]
    fn deploy_copies_the_canonical_text_verbatim() {
        let mut fx = fixture();
        let text = "---\ndescription: 审查并合并\nallowed-tools: Bash\n---\n\n按步骤审查。\n";
        fx.seed(text);
        assert!(matches!(fx.deploy(), Deployed::Written(_)));
        // 逐字落盘：frontmatter 与正文都不重排
        assert_eq!(std::fs::read_to_string(fx.target()).unwrap(), text);
    }

    #[test]
    fn a_foreign_file_with_the_same_name_is_never_overwritten() {
        let mut fx = fixture();
        fx.seed("我们的命令\n");
        std::fs::create_dir_all(fx.commands_dir()).unwrap();
        std::fs::write(fx.target(), "用户自己的 /review\n").unwrap();

        assert_eq!(fx.deploy(), Deployed::Foreign);
        assert_eq!(
            std::fs::read_to_string(fx.target()).unwrap(),
            "用户自己的 /review\n"
        );
        assert_eq!(
            remove(
                &fx.store,
                &mut fx.index,
                &fx.agent,
                "review",
                &fx.home,
                None
            )
            .unwrap(),
            Removed::Foreign
        );
        assert!(fx.target().is_file());
    }

    #[test]
    fn only_our_own_file_is_removed_and_the_ledger_is_cleared() {
        let mut fx = fixture();
        fx.seed("我们的命令\n");
        fx.deploy();
        std::fs::create_dir_all(fx.commands_dir()).unwrap();
        std::fs::write(fx.commands_dir().join("another.md"), "别人的\n").unwrap();

        assert_eq!(
            remove(
                &fx.store,
                &mut fx.index,
                &fx.agent,
                "review",
                &fx.home,
                None
            )
            .unwrap(),
            Removed::Ours
        );
        assert!(!fx.target().exists());
        assert!(fx.commands_dir().join("another.md").is_file());
        assert!(ledger::load(&fx.store, "claude")
            .unwrap()
            .get(AssetKind::Command, "review")
            .is_none());
    }

    #[test]
    fn a_hand_edited_copy_is_drift_not_silently_replaced() {
        let mut fx = fixture();
        fx.seed("我们的命令\n");
        fx.deploy();
        std::fs::write(fx.target(), "我们改过的命令\n").unwrap();

        assert_eq!(fx.deploy(), Deployed::Foreign);
        assert_eq!(
            std::fs::read_to_string(fx.target()).unwrap(),
            "我们改过的命令\n"
        );
    }

    #[test]
    fn an_agent_without_a_commands_target_is_refused() {
        let mut fx = fixture();
        fx.agent = builtin_agents()
            .into_iter()
            .find(|a| a.id == "codex")
            .unwrap();
        fx.seed("命令\n");
        assert!(
            deploy(
                &fx.store,
                &mut fx.index,
                &fx.agent,
                "review",
                &fx.home,
                None
            )
            .is_err(),
            "Codex 注册表里没有命令落点，不能往里写"
        );
    }
}
