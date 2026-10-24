use crate::domain::{AssetSummary, AssetType};
use crate::error::AppError;
use crate::services::assets::AssetService;
use crate::state::AppState;
use std::path::PathBuf;
use tauri::State;

#[tauri::command]
pub fn list_assets(
    state: State<'_, AppState>,
    asset_type: Option<AssetType>,
) -> Result<Vec<AssetSummary>, AppError> {
    let index = state
        .index
        .lock()
        .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
    AssetService::list(&state.store, &index, asset_type)
}

#[tauri::command]
pub fn read_asset(
    state: State<'_, AppState>,
    asset_type: AssetType,
    id: String,
) -> Result<String, AppError> {
    AssetService::read(&state.store, asset_type, &id)
}

#[tauri::command]
pub fn write_asset(
    state: State<'_, AppState>,
    asset_type: AssetType,
    id: String,
    content: String,
) -> Result<AssetSummary, AppError> {
    let _guard = state.enter_write()?;
    let mut index = state
        .index
        .lock()
        .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
    let summary = AssetService::write(&state.store, &mut index, asset_type, &id, &content)?;
    drop(index);
    state.notify_git(format!("write: {}/{id}", asset_type.dir_name()));
    Ok(summary)
}

#[tauri::command]
pub fn delete_asset(
    state: State<'_, AppState>,
    asset_type: AssetType,
    id: String,
) -> Result<bool, AppError> {
    let _guard = state.enter_write()?;
    let mut index = state
        .index
        .lock()
        .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
    let deleted = AssetService::delete(&state.store, &mut index, asset_type, &id)?;
    drop(index);
    if deleted {
        state.notify_git(format!("delete: {}/{id}", asset_type.dir_name()));
    }
    Ok(deleted)
}

/// 从本地目录导入一个 skill。`source` 是用户通过目录选择器挑的绝对路径。
#[tauri::command]
pub fn import_skill_dir(
    state: State<'_, AppState>,
    source: PathBuf,
) -> Result<AssetSummary, AppError> {
    let _guard = state.enter_write()?;
    let mut index = state
        .index
        .lock()
        .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
    let summary = AssetService::import_skill_dir(&state.store, &mut index, &source)?;
    drop(index);
    state.notify_git(format!("import: skills/{}", summary.id));
    Ok(summary)
}
