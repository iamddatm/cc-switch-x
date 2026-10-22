use crate::domain::{AssetSummary, AssetType};
use crate::error::AppError;
use crate::services::assets::AssetService;
use crate::state::AppState;
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
    let mut index = state
        .index
        .lock()
        .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
    AssetService::write(&state.store, &mut index, asset_type, &id, &content)
}

#[tauri::command]
pub fn delete_asset(
    state: State<'_, AppState>,
    asset_type: AssetType,
    id: String,
) -> Result<bool, AppError> {
    let mut index = state
        .index
        .lock()
        .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
    AssetService::delete(&state.store, &mut index, asset_type, &id)
}
