use crate::error::AppError;
use crate::import::mcp::ConflictPolicy;
use crate::import::ImportPreview;
use crate::services::apply::{ApplyReport, ApplyService};
use crate::services::import as import_service;
use crate::services::import::ImportSummary;
use crate::state::AppState;
use tauri::State;

fn ccswitch_home(state: &AppState) -> Result<std::path::PathBuf, AppError> {
    import_service::detect_home(&state.home)
        .ok_or_else(|| AppError::NotFound("没找到 ~/.cc-switch/cc-switch.db".into()))
}

#[tauri::command]
pub fn import_preview(state: State<'_, AppState>) -> Result<ImportPreview, AppError> {
    let home = ccswitch_home(&state)?;
    import_service::preview(&state.store, &home)
}

#[tauri::command]
pub fn import_apply(
    state: State<'_, AppState>,
    policy: ConflictPolicy,
) -> Result<ImportSummary, AppError> {
    let home = ccswitch_home(&state)?;
    let mut index = state
        .index
        .lock()
        .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
    let mut secrets = state
        .secrets
        .lock()
        .map_err(|_| AppError::InvalidInput("secrets 锁失败".into()))?;
    import_service::apply(&state.store, &mut index, &mut secrets, &home, policy)
}

/// 回滚最近一次写入。返回的条目数是被还原的文件数。
#[tauri::command]
pub fn revert_last(state: State<'_, AppState>) -> Result<ApplyReport, AppError> {
    let mut index = state
        .index
        .lock()
        .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
    ApplyService::revert_last(&mut index)
}
