use crate::adapters::cli::CliChannel;
use crate::adapters::mcp as mcp_adapter;
use crate::adapters::paths::WriteTarget;
use crate::adapters::prompt::{self, ActivateOutcome, Takeover};
use crate::adapters::skills::{self, Deployed, Removed};
use crate::domain::agent::{builtin_agents, AgentDescriptor};
use crate::domain::mcp::McpServer;
use crate::domain::{AssetKind, AssetType};
use crate::error::AppError;
use crate::import::update_matrix;
use crate::services::agents::{detect_all, load_state, AgentStatus};
use crate::services::apply::ApplyReport;
use crate::services::drift;
use crate::state::AppState;
use std::path::PathBuf;
use tauri::State;

/// 每个写入入口都要带上要动的资产类型，能力表按类型把关：
/// 未实测的落点连命令都进不来，而不是靠面板自觉。
pub fn descriptor_for(agent_id: &str, kind: AssetKind) -> Result<AgentDescriptor, AppError> {
    builtin_agents()
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .filter(|agent| agent.switchable(kind))
        .ok_or_else(|| {
            AppError::Unsupported(format!(
                "{agent_id} 的 {} 落点未收录或未实测，未做任何改动",
                kind.snake_name()
            ))
        })
}

pub(crate) fn override_base(state: &AppState, agent_id: &str) -> Result<Option<PathBuf>, AppError> {
    Ok(load_state(&state.store, agent_id)?.base_override)
}

#[tauri::command]
pub fn list_agents(state: State<'_, AppState>) -> Result<Vec<AgentStatus>, AppError> {
    detect_all(&state.store, &state.home)
}

#[tauri::command]
pub fn set_mcp_enabled(
    state: State<'_, AppState>,
    id: String,
    agent_id: String,
    enabled: bool,
) -> Result<ApplyReport, AppError> {
    let descriptor = descriptor_for(&agent_id, AssetKind::Mcp)?;
    let canonical = state.store.read(AssetType::Mcp, &id)?;
    let server: McpServer = serde_json::from_slice(&canonical)?;
    let base = override_base(&state, &agent_id)?;
    // CLI 通道：只有该 agent 有官方写命令时才去 PATH 里找（Claude / Codex 等直接是 NotApplicable）。
    let target = WriteTarget::new(&state.home, base.as_deref())
        .with_channel(CliChannel::for_agent(&agent_id));

    // 锁顺序固定为 index → secrets，与 commands::import 一致；反过来会构成死锁风险。
    let report = {
        let mut index = state
            .index
            .lock()
            .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
        let secrets = state
            .secrets
            .lock()
            .map_err(|_| AppError::InvalidInput("secrets 锁失败".into()))?;
        if enabled {
            mcp_adapter::write_enable(
                &state.store,
                &mut index,
                &descriptor,
                &server,
                &secrets,
                &target,
            )
        } else {
            mcp_adapter::write_disable(&state.store, &mut index, &descriptor, &id, &target)
        }?
    };
    update_matrix(&state.store, &agent_id, |matrix| {
        if enabled {
            matrix.enable(AssetType::Mcp, &id);
        } else {
            matrix.disable(AssetType::Mcp, &id);
        }
    })?;
    Ok(report)
}

/// 返回实测的落地方式（`symlink` / `junction` / `copy` / `already` / `removed`），
/// UI 显示的是它而不是配置意图。
#[tauri::command]
pub fn set_skill_enabled(
    state: State<'_, AppState>,
    id: String,
    agent_id: String,
    enabled: bool,
) -> Result<String, AppError> {
    let descriptor = descriptor_for(&agent_id, AssetKind::Skill)?;
    let base = override_base(&state, &agent_id)?;
    let label = if enabled {
        match skills::deploy(&state.store, &descriptor, &id, &state.home, base.as_deref())? {
            Deployed::Linked(kind) => format!("{kind:?}").to_lowercase(),
            Deployed::Already => "already".to_string(),
            Deployed::Foreign => {
                return Err(AppError::InvalidInput(format!(
                    "{agent_id} 的 skills/{id} 位置上是别人建的目录，未做任何改动"
                )))
            }
        }
    } else {
        match skills::remove(&state.store, &descriptor, &id, &state.home, base.as_deref())? {
            Removed::Ours(_) => "removed".to_string(),
            Removed::Absent => "absent".to_string(),
            Removed::Foreign => {
                return Err(AppError::InvalidInput(format!(
                    "{agent_id} 的 skills/{id} 不是我们部署的，拒绝删除"
                )))
            }
        }
    };
    update_matrix(&state.store, &agent_id, |matrix| {
        if enabled {
            matrix.enable(AssetType::Skill, &id);
        } else {
            matrix.disable(AssetType::Skill, &id);
        }
    })?;
    Ok(label)
}

/// `confirm=false` 时遇到外来内容只返回 `needsConfirmation`，不写盘；
/// `saveExistingAs` 给了新 ID 就先把旧内容抢救成一条 prompt 再覆盖。
#[tauri::command]
pub fn activate_prompt(
    state: State<'_, AppState>,
    prompt_id: String,
    agent_id: String,
    confirm: Option<bool>,
    save_existing_as: Option<String>,
) -> Result<ActivateOutcome, AppError> {
    let descriptor = descriptor_for(&agent_id, AssetKind::Prompt)?;
    let base = override_base(&state, &agent_id)?;
    let takeover = match save_existing_as {
        Some(new_id) => Takeover::SaveExistingAs(new_id),
        None if confirm.unwrap_or(false) => Takeover::Confirm,
        None => Takeover::Ask,
    };
    let outcome = {
        let mut index = state
            .index
            .lock()
            .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
        // 提示词是一个 agent 一份文件，所以按 agent + 类型判漂移：换激活哪一条
        // 同样会覆盖那份被手改过的文件。
        drift::ensure_no_open_drift_for_agent(&index, AssetKind::Prompt, &agent_id)?;
        prompt::activate(
            &state.store,
            &mut index,
            &descriptor,
            &prompt_id,
            &takeover,
            &state.home,
            base.as_deref(),
        )?
    };
    if matches!(outcome, ActivateOutcome::Activated { .. }) {
        let id = prompt_id.clone();
        update_matrix(&state.store, &agent_id, move |matrix| {
            matrix.active_prompt = Some(id);
        })?;
    }
    Ok(outcome)
}
