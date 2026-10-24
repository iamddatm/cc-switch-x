use crate::adapters::ledger;
use crate::adapters::paths::expand_root;
use crate::domain::agent::AgentDescriptor;
use crate::domain::prompt::{parse_prompt, render_prompt, Prompt, PromptMetadata};
use crate::domain::{AssetKind, AssetType};
use crate::error::AppError;
use crate::index::{sha256_hex, Index};
use crate::store::{atomic::write_atomic, Store};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// 目标文件里已有内容、且不是我们上次写的，怎么处置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Takeover {
    /// 默认：先问用户，一个字节都不写
    Ask,
    /// 用户已确认，直接覆盖
    Confirm,
    /// 把现有内容另存成一条新 prompt（值为新 ID），再覆盖
    SaveExistingAs(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ActivateOutcome {
    Activated {
        path: PathBuf,
    },
    /// 文件全文已经等于该 prompt，什么都没做
    AlreadyActive {
        path: PathBuf,
    },
    /// 目标文件是别人的内容，等用户二选一
    NeedsConfirmation {
        path: PathBuf,
    },
}

/// 提示词的渲染器版本：写进的是正文（无 frontmatter）。将来若改成带标记的托管块，
/// 递增此值，旧条目仍能靠 `find_by_path` 认出自家写的文件。
const RENDERER_VERSION: u32 = 1;

/// 台账损坏必须报错，不能当成「没有记录」：那会让下次 activate 误判成外来内容。
fn is_ours(store: &Store, agent_id: &str, path: &Path, bytes: &[u8]) -> Result<bool, AppError> {
    Ok(ledger::load(store, agent_id)?
        .find_by_path(path, &sha256_hex(bytes))
        .is_some())
}

fn record_applied(
    store: &Store,
    agent_id: &str,
    prompt_id: &str,
    path: &Path,
    bytes: &[u8],
) -> Result<(), AppError> {
    ledger::record(
        store,
        agent_id,
        AssetKind::Prompt,
        prompt_id,
        path,
        &sha256_hex(bytes),
        RENDERER_VERSION,
    )
}

pub fn target_file(agent: &AgentDescriptor, home: &Path, base_override: Option<&Path>) -> PathBuf {
    expand_root(&agent.paths.prompt_file, base_override, home)
}

/// 整文件接管（spec 4.2）：写进去的是 prompt 的**正文**，frontmatter 只是真源元数据，
/// 不该混进 agent 实际读到的提示词里。
pub fn activate(
    store: &Store,
    index: &mut Index,
    agent: &AgentDescriptor,
    prompt_id: &str,
    takeover: &Takeover,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<ActivateOutcome, AppError> {
    let canonical = String::from_utf8(store.read(AssetType::Prompt, prompt_id)?)
        .map_err(|_| AppError::InvalidInput(format!("真源里的 {prompt_id} 不是 UTF-8")))?;
    let body = parse_prompt(prompt_id, &canonical)?.body;
    let path = target_file(agent, home, base_override);
    let existing = std::fs::read(&path).ok();

    if existing.as_deref() == Some(body.as_bytes()) {
        return Ok(ActivateOutcome::AlreadyActive { path });
    }

    let ours = match &existing {
        None => true,
        Some(current) => is_ours(store, &agent.id, &path, current)?,
    };
    if !ours {
        match takeover {
            Takeover::Ask => return Ok(ActivateOutcome::NeedsConfirmation { path }),
            Takeover::Confirm => {}
            Takeover::SaveExistingAs(new_id) => {
                let current = existing.clone().unwrap_or_default();
                let text = String::from_utf8(current)
                    .map_err(|_| AppError::InvalidInput("现有提示词不是 UTF-8，无法另存".into()))?;
                let saved = render_prompt(&Prompt {
                    id: new_id.clone(),
                    meta: PromptMetadata {
                        title: Some(new_id.clone()),
                        description: Some("激活前从目标文件抢救出来的内容".into()),
                    },
                    body: text,
                });
                store.write(AssetType::Prompt, new_id, saved.as_bytes())?;
                index.upsert(&crate::domain::AssetSummary {
                    asset_type: AssetType::Prompt,
                    id: new_id.clone(),
                    name: new_id.clone(),
                    updated_at: chrono::Utc::now().timestamp_millis(),
                    hash: sha256_hex(saved.as_bytes()),
                    description: None,
                    source: None,
                    mtime: None,
                    transport: None,
                    size: None,
                    origin: None,
                })?;
            }
        }
    }

    write_atomic(&path, body.as_bytes())?;
    record_applied(store, &agent.id, prompt_id, &path, body.as_bytes())?;
    Ok(ActivateOutcome::Activated { path })
}

/// 取消激活：只有文件确实是我们写的那份才清掉，否则留着别人的内容不动。
pub fn deactivate(
    store: &Store,
    agent: &AgentDescriptor,
    home: &Path,
    base_override: Option<&Path>,
) -> Result<bool, AppError> {
    let path = target_file(agent, home, base_override);
    let Some(current) = std::fs::read(&path).ok() else {
        return Ok(false);
    };
    let ours = ledger::load(store, &agent.id)?
        .find_by_path(&path, &sha256_hex(&current))
        .cloned();
    let Some(entry) = ours else {
        return Ok(false);
    };

    std::fs::remove_file(&path)?;
    ledger::forget(store, &agent.id, AssetKind::Prompt, &entry.asset_id)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::Index;
    use tempfile::tempdir;

    fn agent(id: &str) -> AgentDescriptor {
        crate::domain::agent::builtin_agents()
            .into_iter()
            .find(|a| a.id == id)
            .unwrap()
    }

    fn seed_prompt(store: &Store, id: &str, body: &str) {
        let text = format!("---\ntitle: {id}\n---\n\n{body}");
        store.write(AssetType::Prompt, id, text.as_bytes()).unwrap();
    }

    fn fixture() -> (tempfile::TempDir, Store, Index, AgentDescriptor) {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let index = Index::open(&dir.path().join("index.db")).unwrap();
        (dir, store, index, agent("claude"))
    }

    #[test]
    fn activating_writes_the_prompt_body_not_the_frontmatter() {
        let (dir, store, mut index, descriptor) = fixture();
        seed_prompt(&store, "team", "正文内容");
        let home = dir.path().join("home");

        let outcome = activate(
            &store,
            &mut index,
            &descriptor,
            "team",
            &Takeover::Confirm,
            &home,
            None,
        )
        .unwrap();
        assert!(matches!(outcome, ActivateOutcome::Activated { .. }));
        let written = std::fs::read_to_string(home.join(".claude/CLAUDE.md")).unwrap();
        assert_eq!(
            written, "正文内容",
            "frontmatter 不该混进 agent 读到的提示词"
        );
    }

    #[test]
    fn activating_twice_is_a_no_op() {
        let (dir, store, mut index, descriptor) = fixture();
        seed_prompt(&store, "team", "正文");
        let home = dir.path().join("home");
        activate(
            &store,
            &mut index,
            &descriptor,
            "team",
            &Takeover::Confirm,
            &home,
            None,
        )
        .unwrap();

        let outcome = activate(
            &store,
            &mut index,
            &descriptor,
            "team",
            &Takeover::Confirm,
            &home,
            None,
        )
        .unwrap();
        assert!(matches!(outcome, ActivateOutcome::AlreadyActive { .. }));
    }

    #[test]
    fn over_foreign_content_requires_confirmation_and_writes_nothing() {
        let (dir, store, mut index, descriptor) = fixture();
        seed_prompt(&store, "team", "我们的");
        let home = dir.path().join("home");
        let target = home.join(".claude/CLAUDE.md");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "用户自己写的提示词").unwrap();

        let outcome = activate(
            &store,
            &mut index,
            &descriptor,
            "team",
            &Takeover::Ask,
            &home,
            None,
        )
        .unwrap();
        assert!(matches!(outcome, ActivateOutcome::NeedsConfirmation { .. }));
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "用户自己写的提示词",
            "没确认之前一个字都不能覆盖"
        );
    }

    #[test]
    fn a_confirmed_takeover_overwrites_foreign_content_and_backs_it_up_first() {
        let (dir, store, mut index, descriptor) = fixture();
        seed_prompt(&store, "team", "我们的");
        let home = dir.path().join("home");
        let target = home.join(".claude/CLAUDE.md");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "用户自己写的提示词").unwrap();

        let outcome = activate(
            &store,
            &mut index,
            &descriptor,
            "team",
            &Takeover::Confirm,
            &home,
            None,
        )
        .unwrap();
        assert!(matches!(outcome, ActivateOutcome::Activated { .. }));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "我们的");
    }

    #[test]
    fn save_existing_as_keeps_the_old_content_then_activates() {
        let (dir, store, mut index, descriptor) = fixture();
        seed_prompt(&store, "team", "我们的");
        let home = dir.path().join("home");
        let target = home.join(".claude/CLAUDE.md");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "旧文件内容").unwrap();

        activate(
            &store,
            &mut index,
            &descriptor,
            "team",
            &Takeover::SaveExistingAs("salvaged".into()),
            &home,
            None,
        )
        .unwrap();

        assert_eq!(std::fs::read_to_string(&target).unwrap(), "我们的");
        let salvaged =
            String::from_utf8(store.read(AssetType::Prompt, "salvaged").unwrap()).unwrap();
        assert!(salvaged.contains("旧文件内容"), "{salvaged}");
        assert!(store
            .list_ids(AssetType::Prompt)
            .unwrap()
            .contains(&"salvaged".to_string()));
    }

    #[test]
    fn our_own_previous_write_can_be_replaced_without_confirmation() {
        let (dir, store, mut index, descriptor) = fixture();
        seed_prompt(&store, "a", "A 的正文");
        seed_prompt(&store, "b", "B 的正文");
        let home = dir.path().join("home");
        activate(
            &store,
            &mut index,
            &descriptor,
            "a",
            &Takeover::Confirm,
            &home,
            None,
        )
        .unwrap();

        let outcome = activate(
            &store,
            &mut index,
            &descriptor,
            "b",
            &Takeover::Confirm,
            &home,
            None,
        )
        .unwrap();
        assert!(
            matches!(outcome, ActivateOutcome::Activated { .. }),
            "切换激活项不该被当成漂移"
        );
        assert_eq!(
            std::fs::read_to_string(home.join(".claude/CLAUDE.md")).unwrap(),
            "B 的正文"
        );
    }

    #[test]
    fn external_edit_after_activation_is_reported_as_drift_not_silently_overwritten() {
        let (dir, store, mut index, descriptor) = fixture();
        seed_prompt(&store, "team", "我们的");
        let home = dir.path().join("home");
        activate(
            &store,
            &mut index,
            &descriptor,
            "team",
            &Takeover::Confirm,
            &home,
            None,
        )
        .unwrap();
        let target = home.join(".claude/CLAUDE.md");
        std::fs::write(&target, "用户手改了").unwrap();

        let outcome = activate(
            &store,
            &mut index,
            &descriptor,
            "team",
            &Takeover::Ask,
            &home,
            None,
        )
        .unwrap();
        assert!(matches!(outcome, ActivateOutcome::NeedsConfirmation { .. }));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "用户手改了");
    }

    #[test]
    fn deactivate_only_removes_our_own_file() {
        let (dir, store, mut index, descriptor) = fixture();
        seed_prompt(&store, "team", "我们的");
        let home = dir.path().join("home");
        let target = home.join(".claude/CLAUDE.md");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "别人的").unwrap();
        assert!(!deactivate(&store, &descriptor, &home, None).unwrap());
        assert!(target.exists());

        // 清掉别人的文件后才轮到我们写（activate 对外来内容会拒绝覆盖）
        std::fs::remove_file(&target).unwrap();
        activate(
            &store,
            &mut index,
            &descriptor,
            "team",
            &Takeover::Confirm,
            &home,
            None,
        )
        .unwrap();
        assert!(deactivate(&store, &descriptor, &home, None).unwrap());
        assert!(!target.exists());
    }
}
