use crate::adapters::subagents;
use crate::adapters::{Deployed, Removed};
use crate::commands::agents::{descriptor_for, override_base};
use crate::domain::{AssetKind, AssetType};
use crate::error::AppError;
use crate::import::update_matrix;
use crate::services::agents::ensure_agent_detected;
use crate::services::drift;
use crate::services::scan::{self as scan_service, ScanReport};
use crate::state::AppState;
use tauri::State;

/// 返回实测标签（`written` / `removed` / `absent`）；外来内容报 `InvalidInput`，
/// 与 rules、skills 的口径一致：UI 看到的必须是「到底动没动」。
#[tauri::command]
pub fn set_subagent_enabled(
    state: State<'_, AppState>,
    id: String,
    agent_id: String,
    enabled: bool,
) -> Result<String, AppError> {
    let descriptor = descriptor_for(&agent_id, AssetKind::Subagent)?;
    let base = override_base(&state, &agent_id)?;
    let label = {
        let mut index = state
            .index
            .lock()
            .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
        // 有未解决的漂移时不许直接覆盖；取消落盘不拦（remove 自己会拒绝动不认识的内容）
        if enabled {
            ensure_agent_detected(&state.store, &state.home, &agent_id)?;
            drift::ensure_no_open_drift(&index, AssetKind::Subagent, &id, &agent_id)?;
            match subagents::deploy(
                &state.store,
                &mut index,
                &descriptor,
                &id,
                &state.home,
                base.as_deref(),
            )? {
                Deployed::Written(_) => "written".to_string(),
                Deployed::Foreign => {
                    return Err(AppError::InvalidInput(format!(
                        "{agent_id} 的 agents/{id} 是内置的或你自己写的文件，未做任何改动"
                    )))
                }
            }
        } else {
            match subagents::remove(
                &state.store,
                &mut index,
                &descriptor,
                &id,
                &state.home,
                base.as_deref(),
            )? {
                Removed::Ours => "removed".to_string(),
                Removed::Absent => "absent".to_string(),
                Removed::Foreign => {
                    return Err(AppError::InvalidInput(format!(
                        "{agent_id} 的 agents/{id} 不是我们落的文件，拒绝删除"
                    )))
                }
            }
        }
    };
    update_matrix(&state.store, &agent_id, |matrix| {
        if enabled {
            matrix.enable(AssetType::Subagent, &id);
        } else {
            matrix.disable(AssetType::Subagent, &id);
        }
    })?;
    Ok(label)
}

#[tauri::command]
pub fn scan_subagents(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<ScanReport, AppError> {
    let descriptor = descriptor_for(&agent_id, AssetKind::Subagent)?;
    let base = override_base(&state, &agent_id)?;
    let mut index = state
        .index
        .lock()
        .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
    scan_service::subagents(
        &state.store,
        &mut index,
        &descriptor,
        &state.home,
        base.as_deref(),
    )
}
