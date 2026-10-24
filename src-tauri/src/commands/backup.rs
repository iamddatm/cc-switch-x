//! 快照备份与占用空间命令。恢复/清理都需要与写操作互斥，锁在命令边界获取。

use crate::error::AppError;
use crate::services::backup::{self, BackupUsage, RestoreBackupReport, SnapshotInfo, SnapshotKind};
use crate::services::settings as settings_service;
use crate::settings::{self as settings_file, AppSettings, BackupSettings};
use crate::state::AppState;
use std::path::Path;
use std::process::Command;
use tauri::{AppHandle, Manager, State};

async fn run_blocking<T, F>(task: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| AppError::Backup(format!("备份任务执行失败：{error}")))?
}

fn backup_policy(state: &AppState) -> Result<(u32, u32), AppError> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    Ok((
        settings.backup.interval_hours,
        settings.backup.retain_count,
    ))
}

#[tauri::command]
pub fn list_backup_snapshots(state: State<'_, AppState>) -> Result<Vec<SnapshotInfo>, AppError> {
    backup::list(&state.store)
}

#[tauri::command]
pub async fn create_backup_snapshot(
    app: AppHandle,
    name: Option<String>,
) -> Result<SnapshotInfo, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let (_, retain) = backup_policy(&state)?;
        let _guard = state.enter_maintenance()?;
        let info = backup::create(&state.store, name.as_deref(), SnapshotKind::Manual)?;
        let _ = backup::prune(&state.store, retain.max(1) as usize);
        Ok(info)
    })
    .await
}

#[tauri::command]
pub fn rename_backup_snapshot(
    state: State<'_, AppState>,
    old_name: String,
    new_name: String,
) -> Result<SnapshotInfo, AppError> {
    let _guard = state.enter_write()?;
    backup::rename(&state.store, &old_name, &new_name)
}

#[tauri::command]
pub fn delete_backup_snapshot(state: State<'_, AppState>, name: String) -> Result<(), AppError> {
    let _guard = state.enter_write()?;
    backup::delete(&state.store, &name)
}

#[tauri::command]
pub fn get_backup_usage(state: State<'_, AppState>) -> Result<BackupUsage, AppError> {
    backup::usage(&state.store)
}

#[tauri::command]
pub fn get_backup_settings(state: State<'_, AppState>) -> Result<BackupSettings, AppError> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    Ok(settings.backup.clone())
}

#[tauri::command]
pub fn set_backup_settings(
    state: State<'_, AppState>,
    settings: BackupSettings,
) -> Result<AppSettings, AppError> {
    let updated = settings_service::set_backup_settings_at(
        &settings_file::settings_path()?,
        settings,
    )?;
    let mut current = state
        .settings
        .lock()
        .map_err(|_| AppError::InvalidInput("settings 锁失败".into()))?;
    *current = updated.clone();
    Ok(updated)
}

#[tauri::command]
pub async fn restore_backup_snapshot(
    app: AppHandle,
    name: String,
) -> Result<RestoreBackupReport, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let (_, retain) = backup_policy(&state)?;
        let _guard = state.enter_maintenance()?;
        let mut index = state
            .index
            .lock()
            .map_err(|_| AppError::InvalidInput("index 锁失败".into()))?;
        let report = backup::restore(&state.store, &mut index, &name)?;
        drop(index);
        let _ = backup::prune(&state.store, retain.max(1) as usize);
        state.notify_git(format!("restore: {name}"));
        Ok(report)
    })
    .await
}

#[tauri::command]
pub async fn delete_backup_category(app: AppHandle, id: String) -> Result<u64, AppError> {
    run_blocking(move || {
        let state = app.state::<AppState>();
        let _guard = state.enter_maintenance()?;
        backup::delete_category(&state.store, &id)
    })
    .await
}

/// 只接受分区 id，路径由后端按 store root 计算；不接受前端传任意路径。
#[tauri::command]
pub fn reveal_backup_location(state: State<'_, AppState>, id: String) -> Result<(), AppError> {
    let path = backup::reveal_location(&state.store, &id)?;
    open_directory(&path)
}

fn open_directory(path: &Path) -> Result<(), AppError> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("explorer");
        command.arg(path);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(path);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(path);
        command
    };
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| AppError::Backup(format!("无法打开目录：{error}")))
}
