use crate::adapters::paths::expand_root;
use crate::domain::agent::builtin_agents;
use crate::error::AppError;
use crate::store::atomic::write_atomic;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// `local/agents/<agent-id>.json`：本机专属状态，永不参与同步。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LocalAgentState {
    /// 覆盖 `~` 锚点；真机验证与非常规安装位置用。
    pub base_override: Option<PathBuf>,
    /// 上次部署 skill **实测**用到的方式（不是配置意图）。
    pub link_kind: Option<crate::adapters::skills::LinkKind>,
}

/// 给前端的一份 agent 现状（列表 + 矩阵表头用）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub id: String,
    pub display_name: String,
    pub detected: bool,
    pub verified: bool,
    pub mcp_needs_restart: Option<bool>,
    pub mcp_path: PathBuf,
}

fn state_path(store: &Store, agent_id: &str) -> PathBuf {
    store
        .root()
        .join("local")
        .join("agents")
        .join(format!("{agent_id}.json"))
}

pub fn load_state(store: &Store, agent_id: &str) -> Result<LocalAgentState, AppError> {
    let path = state_path(store, agent_id);
    if !path.exists() {
        return Ok(LocalAgentState::default());
    }
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

fn write_state(store: &Store, agent_id: &str, state: &LocalAgentState) -> Result<(), AppError> {
    write_atomic(
        &state_path(store, agent_id),
        &serde_json::to_vec_pretty(state)?,
    )?;
    Ok(())
}

/// 只改 baseOverride，保留已经记下的 link 方式（反之亦然）。
pub fn save_override(store: &Store, agent_id: &str, base: Option<&Path>) -> Result<(), AppError> {
    let mut state = load_state(store, agent_id)?;
    state.base_override = base.map(Path::to_path_buf);
    write_state(store, agent_id, &state)
}

pub fn save_link_kind(
    store: &Store,
    agent_id: &str,
    kind: crate::adapters::skills::LinkKind,
) -> Result<(), AppError> {
    let mut state = load_state(store, agent_id)?;
    state.link_kind = Some(kind);
    write_state(store, agent_id, &state)
}

/// 检测只看配置根在不在；覆盖生效与否完全由 `local/agents/<id>.json` 决定。
pub fn detect_all(store: &Store, home: &Path) -> Result<Vec<AgentStatus>, AppError> {
    let mut out = Vec::new();
    for descriptor in builtin_agents() {
        let override_base = load_state(store, &descriptor.id)?.base_override;
        let config_root = expand_root(&descriptor.config_root, override_base.as_deref(), home);
        let mcp_path = expand_root(descriptor.paths.mcp_file(), override_base.as_deref(), home);
        out.push(AgentStatus {
            detected: config_root.is_dir(),
            verified: descriptor.verified,
            mcp_needs_restart: descriptor.mcp_needs_restart,
            display_name: descriptor.display_name,
            id: descriptor.id,
            mcp_path,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn detect_marks_agents_by_their_config_root() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".claude")).unwrap();

        let status = detect_all(&store, &home).unwrap();
        assert_eq!(status.len(), builtin_agents().len());
        assert!(status.iter().find(|s| s.id == "claude").unwrap().detected);
        assert!(!status.iter().find(|s| s.id == "codex").unwrap().detected);
    }

    #[test]
    fn base_override_relocates_every_target() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        let sandbox = dir.path().join("sandbox");
        std::fs::create_dir_all(sandbox.join(".claude")).unwrap();
        save_override(&store, "claude", Some(&sandbox)).unwrap();

        let status = detect_all(&store, &home).unwrap();
        let claude = status.iter().find(|s| s.id == "claude").unwrap();
        assert_eq!(claude.mcp_path, sandbox.join(".claude.json"));
        assert!(claude.detected, "覆盖目录里有配置根即视为可用");
    }

    #[test]
    fn clearing_the_override_falls_back_to_the_real_home() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        save_override(&store, "claude", Some(&dir.path().join("sandbox"))).unwrap();
        save_override(&store, "claude", None).unwrap();

        let status = detect_all(&store, &home).unwrap();
        let claude = status.iter().find(|s| s.id == "claude").unwrap();
        assert_eq!(claude.mcp_path, home.join(".claude.json"));
        assert!(!claude.detected);
    }

    #[test]
    fn unverified_agents_are_reported_so_the_ui_can_hide_their_switches() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let status = detect_all(&store, &dir.path().join("home")).unwrap();
        assert!(!status.iter().find(|s| s.id == "qoder").unwrap().verified);
        assert_eq!(status.iter().filter(|s| s.verified).count(), 4);
    }
}
