//! 云备份设置与 WebDAV 操作命令。网络任务放阻塞线程，避免卡住 UI。

use crate::error::AppError;
use crate::services::cloud_backup::{self, CloudRestoreReport, CloudStatus};
use crate::services::settings::{cloud_view, set_cloud_settings_at, CloudSettingsView};
use crate::settings as settings_file;
use crate::state::AppState;
use crate::store::secrets::secrets_path;
use tauri::{AppHandle, Emitter, Manager, State};

/// 恢复下载进度事件名；载荷为 `{ received, total }`（字节，未知时 total 为 0）。
const RESTORE_PROGRESS_EVENT: &str = "cloud-restore-progress";

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
    enabled: bool,
    auto_upload: bool,
    auto_download: bool,
    upload_delay_hours: u32,
) -> Result<CloudSettingsView, AppError> {
    let (updated, secrets, connection_changed) = set_cloud_settings_at(
        &settings_file::settings_path()?,
        &secrets_path(state.store.root()),
        base_url,
        username,
        password,
        remote_dir,
        enabled,
        auto_upload,
        auto_download,
        upload_delay_hours,
    )?;
    // 换了远端或凭据，旧基线与观测缓存都不再可信。先递增连接代次，
    // 让仍在进行的旧任务在回写运行状态前看到代次已变，避免把旧基线写回来。
    if connection_changed {
        state.bump_cloud_connection_epoch();
        cloud_backup::reset_runtime_status(&state.store)?;
    }
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
    let view = cloud_view(&current, &current_secrets);
    // 唤醒自动同步工作线程，让它按新的开关与延迟重新计时。
    crate::change_signal::notify();
    Ok(view)
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

/// 只读本地设置、密钥与运行状态文件，立即返回上次观测到的云端状态。
#[tauri::command]
pub fn get_cloud_status_cached(state: State<'_, AppState>) -> Result<CloudStatus, AppError> {
    cloud_backup::status_cached(&state)
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
        // 大包下载可能持续几分钟；把进度推给前端，避免只有一个转圈的按钮。
        let emitter = app.clone();
        let progress = move |received: u64, total: u64| {
            if let Err(error) = emitter.emit(
                RESTORE_PROGRESS_EVENT,
                serde_json::json!({ "received": received, "total": total }),
            ) {
                eprintln!("发送云备份恢复进度失败：{error}");
            }
        };
        cloud_backup::restore_latest(&state, Some(&progress))
    })
    .await
}
