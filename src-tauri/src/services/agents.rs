use crate::adapters::paths::expand_root;
use crate::domain::agent::{builtin_agents, AgentDescriptor};
use crate::domain::enablement::AgentEnablement;
use crate::domain::{AssetKind, AssetType, CapabilityStatus};
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
/// 顺带把该 agent 已勾选的资产一起返回，矩阵一次取全，省掉每个资产问一次的往返。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub id: String,
    pub display_name: String,
    pub detected: bool,
    /// PATH 与常见安装目录中是否找得到 CLI；只用于展示，不参与写入门禁。
    pub cli_available: bool,
    /// 探测用的命令名。
    pub cli_program: String,
    /// 每类资产的 supported/verified 一档一档列出来；不再有「整个 agent 已验证」这种粗话。
    pub capabilities: Vec<CapabilityStatus>,
    pub mcp_needs_restart: Option<bool>,
    pub mcp_path: PathBuf,
    /// 提示词整文件落点；提示词页用它显示「目标文件」。
    pub prompt_path: PathBuf,
    /// skills 目录落点；Agents 页展示用。
    pub skills_dir: PathBuf,
    /// 是否在全局启用范围内。false 时面板不展示这一列，但已部署文件不动。
    pub enabled: bool,
    pub enabled_mcp: Vec<String>,
    pub enabled_skills: Vec<String>,
    pub enabled_rules: Vec<String>,
    pub enabled_subagents: Vec<String>,
    pub enabled_hooks: Vec<String>,
    pub active_prompt: Option<String>,
}

impl AgentStatus {
    pub fn switchable(&self, kind: AssetKind) -> bool {
        self.capabilities.iter().any(|capability| {
            capability.kind == kind && capability.supported && capability.verified
        })
    }

    pub fn supported(&self, kind: AssetKind) -> bool {
        self.capabilities
            .iter()
            .any(|capability| capability.kind == kind && capability.supported)
    }
}

/// 读 `agents/<id>.json`；缺失按空矩阵处理，损坏则如实报错（不静默清空真源）。
pub fn read_matrix(store: &Store, agent_id: &str) -> Result<AgentEnablement, AppError> {
    let path = store.agent_config_path(agent_id);
    if !path.exists() {
        return Ok(AgentEnablement::default());
    }
    Ok(serde_json::from_slice(&std::fs::read(&path)?)?)
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

/// 判定检测用的两个落点：配置根，以及可能位于根外的用户级 MCP 文件。
fn detection_paths(
    descriptor: &AgentDescriptor,
    base_override: Option<&Path>,
    home: &Path,
) -> (PathBuf, PathBuf) {
    (
        expand_root(&descriptor.config_root, base_override, home),
        expand_root(descriptor.paths.mcp_file(), base_override, home),
    )
}

/// 配置根存在，或**根外**的 MCP 文件存在，都算装过。
/// Claude 的用户级 MCP 在 `~/.claude.json`，不在 `~/.claude/` 里；只看根会把它误判成未安装。
/// 其他 agent 的 MCP 文件都在根内部，根不存在时文件不可能存在，这条不会放宽判定。
fn is_detected(config_root: &Path, mcp_path: &Path) -> bool {
    config_root.is_dir() || (!mcp_path.starts_with(config_root) && mcp_path.is_file())
}

/// 写盘门禁：配置根不在就拒绝，避免替用户创建工具目录。
/// 只拦会写文件的动作；纯删除在目标缺失时本来就无事可做，继续放行以便清理。
pub fn ensure_agent_detected(store: &Store, home: &Path, agent_id: &str) -> Result<(), AppError> {
    let descriptor = builtin_agents()
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .ok_or_else(|| AppError::NotFound(format!("注册表里没有 {agent_id}")))?;
    let base = load_state(store, agent_id)?.base_override;
    let (config_root, mcp_path) = detection_paths(&descriptor, base.as_deref(), home);
    if is_detected(&config_root, &mcp_path) {
        return Ok(());
    }
    Err(AppError::NotDetected(format!(
        "{} 的配置目录 {} 不存在，未做任何改动",
        descriptor.display_name,
        config_root.display()
    )))
}

/// 检测只看配置根（或与根同级的 MCP 文件）在不在；覆盖生效与否完全由 `local/agents/<id>.json` 决定。
pub fn detect_all(store: &Store, home: &Path) -> Result<Vec<AgentStatus>, AppError> {
    let mut out = Vec::new();
    for descriptor in builtin_agents() {
        let override_base = load_state(store, &descriptor.id)?.base_override;
        let matrix = read_matrix(store, &descriptor.id)?;
        let (config_root, mcp_path) = detection_paths(&descriptor, override_base.as_deref(), home);
        let prompt_path = expand_root(
            &descriptor.paths.prompt_file,
            override_base.as_deref(),
            home,
        );
        let skills_dir = expand_root(&descriptor.paths.skills_dir, override_base.as_deref(), home);
        let cli_program = descriptor.cli_program.to_string();
        let cli_available = crate::adapters::cli::discover_cli(&cli_program, home).is_some();
        out.push(AgentStatus {
            detected: is_detected(&config_root, &mcp_path),
            cli_available,
            cli_program,
            capabilities: descriptor.capabilities(),
            mcp_needs_restart: descriptor.mcp_needs_restart,
            display_name: descriptor.display_name,
            id: descriptor.id,
            mcp_path,
            prompt_path,
            skills_dir,
            enabled: true,
            enabled_mcp: matrix.enabled_for(AssetType::Mcp),
            enabled_skills: matrix.enabled_for(AssetType::Skill),
            enabled_rules: matrix.enabled_for(AssetType::Rule),
            enabled_subagents: matrix.enabled_for(AssetType::Subagent),
            enabled_hooks: matrix.enabled_for(AssetType::Hook),
            active_prompt: matrix.active_prompt,
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
    fn claude_is_detected_by_its_sibling_mcp_file() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".claude.json"), b"{}").unwrap();

        let status = detect_all(&store, &home).unwrap();
        assert!(
            status.iter().find(|s| s.id == "claude").unwrap().detected,
            "~/.claude.json 存在也应算检测到"
        );
        assert!(!status.iter().find(|s| s.id == "gemini").unwrap().detected);
        assert!(ensure_agent_detected(&store, &home, "claude").is_ok());
    }

    #[test]
    fn the_write_gate_refuses_a_missing_config_root() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();

        let error = ensure_agent_detected(&store, &home, "gemini").unwrap_err();
        assert!(matches!(error, AppError::NotDetected(_)), "{error}");
        assert!(error.to_string().contains(".gemini"), "{error}");

        std::fs::create_dir_all(home.join(".gemini")).unwrap();
        assert!(ensure_agent_detected(&store, &home, "gemini").is_ok());
    }

    #[test]
    fn the_write_gate_follows_the_base_override() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let home = dir.path().join("home");
        let sandbox = dir.path().join("sandbox");
        std::fs::create_dir_all(sandbox.join(".gemini")).unwrap();
        save_override(&store, "gemini", Some(&sandbox)).unwrap();

        assert!(ensure_agent_detected(&store, &home, "gemini").is_ok());
    }

    #[test]
    fn each_agent_declares_the_cli_it_probes() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let status = detect_all(&store, &dir.path().join("home")).unwrap();
        let program = |id: &str| {
            status
                .iter()
                .find(|s| s.id == id)
                .unwrap()
                .cli_program
                .as_str()
        };
        assert_eq!(program("claude"), "claude");
        assert_eq!(program("codex"), "codex");
        assert_eq!(program("gemini"), "gemini");
        assert_eq!(program("qoder"), "qoder");
        assert_eq!(program("qoder-cn"), "qoderclicn");
        assert_eq!(program("pi"), "pi");
    }

    #[test]
    fn agent_status_json_keys_are_the_frontend_contract() {
        // 改字段必须同步改 src/types.ts 的 AgentStatus；这条断言就是那道闸
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let status = detect_all(&store, &dir.path().join("home")).unwrap();
        let mut keys: Vec<String> = serde_json::to_value(&status[0])
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                "activePrompt",
                "capabilities",
                "cliAvailable",
                "cliProgram",
                "detected",
                "displayName",
                "enabled",
                "enabledHooks",
                "enabledMcp",
                "enabledRules",
                "enabledSkills",
                "enabledSubagents",
                "id",
                "mcpNeedsRestart",
                "mcpPath",
                "promptPath",
                "skillsDir",
            ]
        );
    }

    #[test]
    fn status_carries_the_enablement_matrix_so_the_ui_needs_one_call() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        crate::import::update_matrix(&store, "claude", |matrix| {
            matrix.enable(AssetType::Mcp, "pw");
            matrix.active_prompt = Some("claude-team".into());
        })
        .unwrap();

        let status = detect_all(&store, &dir.path().join("home")).unwrap();
        let claude = status.iter().find(|s| s.id == "claude").unwrap();
        assert_eq!(claude.enabled_mcp, vec!["pw".to_string()]);
        assert_eq!(claude.active_prompt.as_deref(), Some("claude-team"));
        assert!(claude.enabled_skills.is_empty());
    }

    #[test]
    fn switches_are_offered_per_capability_not_per_agent() {
        let dir = tempdir().unwrap();
        let store = Store::init(dir.path().join("store")).unwrap();
        let status = detect_all(&store, &dir.path().join("home")).unwrap();
        let by_id = |id: &str| status.iter().find(|s| s.id == id).unwrap();

        // Qoder 国际版：A2b 没确证，连 MCP 都不给开关
        assert!(!by_id("qoder").switchable(AssetKind::Mcp));

        // Qoder CN：MCP 有依据；rules 是「有落点但没验证」，两者在 UI 上表现不同
        let qoder_cn = by_id("qoder-cn");
        assert!(qoder_cn.switchable(AssetKind::Mcp));
        assert!(!qoder_cn.switchable(AssetKind::Rule));
        assert!(
            qoder_cn.supported(AssetKind::Rule),
            "落点存在只是没验证，不能说成不支持"
        );

        // Codex 根本没有 markdown 规则这个概念
        assert!(!by_id("codex").supported(AssetKind::Rule));
    }
}
