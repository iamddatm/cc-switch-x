use crate::adapters::mcp as mcp_adapter;
use crate::adapters::prompt::{self, ActivateOutcome, Takeover};
use crate::adapters::skills::{self, Deployed, Removed};
use crate::domain::agent::{builtin_agents, AgentDescriptor};
use crate::domain::mcp::McpServer;
use crate::domain::{AssetKind, AssetType};
use crate::error::AppError;
use crate::import::update_matrix;
use crate::services::agents::{detect_all, load_state, AgentStatus};
use crate::services::apply::{ApplyReport, ApplyService};
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

    let plan = {
        let secrets = state
            .secrets
            .lock()
            .map_err(|_| AppError::InvalidInput("secrets 锁失败".into()))?;
        if enabled {
            mcp_adapter::plan_enable(&descriptor, &server, &secrets, &state.home, base.as_deref())
        } else {
            mcp_adapter::plan_disable(&descriptor, &id, &state.home, base.as_deref())
        }
    }?;

    let report = {
        let mut index = state
            .index
            .lock()
            .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
        ApplyService::apply(&state.store, &mut index, &[plan])?
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
