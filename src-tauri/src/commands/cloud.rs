//! 云备份设置与 WebDAV 操作命令。网络任务放阻塞线程，避免卡住 UI。

use crate::error::AppError;
use crate::services::cloud_backup::{self, CloudRestoreReport, CloudStatus};
use crate::services::settings::{cloud_view, set_cloud_settings_at, CloudSettingsView};
use crate::settings as settings_file;
use crate::state::AppState;
use crate::store::secrets::secrets_path;
use tauri::{AppHandle, Manager, State};

async fn run_blocking<T, F>(task: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| AppError::Cloud(format!("云备份任务执行失败：{error}")))?
}

#[tauri::command]
pub fn get_cloud_settings(state: State<'_, AppState>) -> Result<CloudSettingsView, AppError> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    let secrets = state
        .secrets
        .lock()
        .map_err(|_| AppError::InvalidInput("secrets 锁失败".into()))?;
    Ok(cloud_view(&settings, &secrets))
}

/// `password` 为 None 表示沿用已保存的密码；空串表示清除密码。
#[tauri::command]
pub fn set_cloud_settings(
    state: State<'_, AppState>,
    base_url: String,
    username: String,
    password: Option<String>,
    remote_dir: String,
    auto_upload: bool,
) -> Result<CloudSettingsView, AppError> {
    let (updated, secrets) = set_cloud_settings_at(
        &settings_file::settings_path()?,
        &secrets_path(state.store.root()),
        base_url,
        username,
        password,
        remote_dir,
        auto_upload,
    )?;
    let mut current = state
        .settings
        .lock()
        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    *current = updated;
    let mut current_secrets = state
        .secrets
        .lock()
        .map_err(|_| AppError::InvalidInput("secrets 锁失败".into()))?;
    *current_secrets = secrets;
    Ok(cloud_view(&current, &current_secrets))
}

#[tauri::command]
pub async fn test_cloud_connection(
    app: AppHandle,
    base_url: String,
    username: String,
    password: Option<String>,
    remote_dir: String,
) -> Result<u64, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        cloud_backup::test_connection(
            &state,
            &base_url,
            &username,
            password.as_deref(),
            &remote_dir,
        )
    })
    .await
}

#[tauri::command]
pub async fn get_cloud_status(app: AppHandle) -> Result<CloudStatus, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        cloud_backup::status(&state)
    })
    .await
}

#[tauri::command]
pub async fn cloud_upload_now(app: AppHandle) -> Result<CloudStatus, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        cloud_backup::upload_now(&state)
    })
    .await
}

#[tauri::command]
pub async fn cloud_restore_latest(app: AppHandle) -> Result<CloudRestoreReport, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        cloud_backup::restore_latest(&state)
    })
    .await
}
