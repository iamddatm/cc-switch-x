use crate::error::AppError;
use crate::services::settings as settings_service;
use crate::settings::{AppSettings, Theme};
use crate::state::AppState;
use tauri::State;

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, AppError> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    Ok(settings.clone())
}

#[tauri::command]
pub fn set_theme(state: State<'_, AppState>, theme: Theme) -> Result<AppSettings, AppError> {
    let updated = settings_service::set_theme(theme)?;
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    *settings = updated.clone();
    Ok(updated)
}

/// 设置全局启用的 agent 范围；只影响 UI 与分发入口，不触碰已部署文件。
#[tauri::command]
pub fn set_enabled_agents(
    state: State<'_, AppState>,
    agent_ids: Vec<String>,
) -> Result<AppSettings, AppError> {
    let updated = settings_service::set_enabled_agents(agent_ids)?;
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    *settings = updated.clone();
    Ok(updated)
}

/// 返回 true 表示设置已写入文件；store root 生效需要重启应用。
#[tauri::command]
pub fn set_store_root(
    state: State<'_, AppState>,
    store_root: std::path::PathBuf,
) -> Result<bool, AppError> {
    let updated = settings_service::set_store_root(store_root)?;
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    *settings = updated;
    Ok(true)
}
