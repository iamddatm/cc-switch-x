use crate::commands::agents::override_base;
use crate::error::AppError;
use crate::services::agents::ensure_agent_detected;
use crate::services::drift::{self, DriftRecord, ResolveAction, ResolveOutcome};
use crate::state::AppState;
use tauri::State;

/// 重新检查落盘结果（启动后一次 + 面板上的「重新检查」）。**只报告，不动磁盘。**
#[tauri::command]
pub fn rescan_drift(state: State<'_, AppState>) -> Result<Vec<DriftRecord>, AppError> {
    let mut index = state
        .index
        .lock()
        .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
    drift::rescan(&state.store, &mut index)
}

/// 用户对一条漂移点「采纳磁盘」或「重刷真源」。
/// `base_override` 从本机状态取：重刷必须落在同一处落点上，否则会写回真实配置根。
#[tauri::command]
pub fn resolve_drift(
    state: State<'_, AppState>,
    id: i64,
    action: ResolveAction,
    agent_id: String,
) -> Result<ResolveOutcome, AppError> {
    let _guard = state.enter_write()?;
    let base = override_base(&state, &agent_id)?;
    // 重刷会把文件重新写回磁盘；配置根已不在时不再替用户创建。
    // 采纳磁盘只改真源，不受检测门禁影响。
    if action == ResolveAction::RepushSource {
        ensure_agent_detected(&state.store, &state.home, &agent_id)?;
    }
    let mut index = state
        .index
        .lock()
        .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
    let outcome = drift::resolve(
        &state.store,
        &mut index,
        &state.home,
        base.as_deref(),
        id,
        action,
    )?;
    drop(index);
    state.notify_git(format!("drift: {action:?} {agent_id}"));
    Ok(outcome)
}

/// 当前挂着的所有待解决漂移（不重新检查磁盘）。
#[tauri::command]
pub fn list_drifts(state: State<'_, AppState>) -> Result<Vec<DriftRecord>, AppError> {
    let index = state
        .index
        .lock()
        .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
    drift::open_records(&state.store, &index)
}
