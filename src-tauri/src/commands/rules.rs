use crate::adapters::rules::{self, Deployed, Removed};
use crate::commands::agents::{descriptor_for, override_base};
use crate::domain::{AssetKind, AssetType};
use crate::error::AppError;
use crate::import::update_matrix;
use crate::services::scan::{self as scan_service, ScanReport};
use crate::state::AppState;
use tauri::State;

/// 返回实测结果标签（`written` / `foreign` / `removed` / `absent`），
/// UI 显示的是它而不是「已成功」——外来内容挡住了要说清楚。
#[tauri::command]
pub fn set_rule_enabled(
    state: State<'_, AppState>,
    id: String,
    agent_id: String,
    enabled: bool,
) -> Result<String, AppError> {
    let descriptor = descriptor_for(&agent_id, AssetKind::Rule)?;
    let base = override_base(&state, &agent_id)?;
    let label = {
        let mut index = state
            .index
            .lock()
            .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
        if enabled {
            match rules::deploy(
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
                        "{agent_id} 的 rules/{id} 是不认识的内容，未做任何改动；要先采纳它请用扫描"
                    )))
                }
            }
        } else {
            match rules::remove(
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
                        "{agent_id} 的 rules/{id} 不是我们落的文件，拒绝删除"
                    )))
                }
            }
        }
    };
    update_matrix(&state.store, &agent_id, |matrix| {
        if enabled {
            matrix.enable(AssetType::Rule, &id);
        } else {
            matrix.disable(AssetType::Rule, &id);
        }
    })?;
    Ok(label)
}

/// 把该 agent 规则目录里已有的文件读成真源资产（spec 7.3 的「采纳磁盘」）。
#[tauri::command]
pub fn scan_rules(state: State<'_, AppState>, agent_id: String) -> Result<ScanReport, AppError> {
    let descriptor = descriptor_for(&agent_id, AssetKind::Rule)?;
    let base = override_base(&state, &agent_id)?;
    let mut index = state
        .index
        .lock()
        .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
    scan_service::rules(
        &state.store,
        &mut index,
        &descriptor,
        &state.home,
        base.as_deref(),
    )
}
