//! 应用内更新：检查、下载、安装都由后端完成，前端只负责触发和展示进度。

use crate::error::AppError;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

pub const UPDATE_DOWNLOAD_PROGRESS_EVENT: &str = "update-download-progress";
pub const UPDATE_INSTALL_STARTED_EVENT: &str = "update-install-started";

const CHECK_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current_version: String,
    pub version: String,
    pub notes: Option<String>,
    pub pub_date: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

fn build_updater(app: &AppHandle) -> Result<tauri_plugin_updater::Updater, AppError> {
    app.updater_builder()
        .timeout(CHECK_TIMEOUT)
        .build()
        .map_err(|error| AppError::Update(format!("初始化更新器失败：{error}")))
}

fn to_info(update: &tauri_plugin_updater::Update) -> UpdateInfo {
    UpdateInfo {
        current_version: update.current_version.clone(),
        version: update.version.clone(),
        notes: update.body.clone(),
        pub_date: update.date.map(|date| date.to_string()),
    }
}

#[tauri::command]
pub async fn check_app_update(app: AppHandle) -> Result<Option<UpdateInfo>, AppError> {
    let updater = build_updater(&app)?;
    let update = updater
        .check()
        .await
        .map_err(|error| AppError::Update(format!("检查更新失败：{error}")))?;
    Ok(update.as_ref().map(to_info))
}

static INSTALLING: AtomicBool = AtomicBool::new(false);

struct InstallingGuard;

impl Drop for InstallingGuard {
    fn drop(&mut self) {
        INSTALLING.store(false, Ordering::SeqCst);
    }
}

#[tauri::command]
pub async fn install_app_update(app: AppHandle) -> Result<bool, AppError> {
    // 开发模式的产物不是安装版，装更新会覆盖开发用的可执行文件，直接拒绝。
    if cfg!(debug_assertions) {
        return Err(AppError::Unsupported(
            "开发模式下不支持应用内更新，请使用安装版验证".into(),
        ));
    }
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err(AppError::Update("已有更新正在安装".into()));
    }
    let _guard = InstallingGuard;

    let updater = build_updater(&app)?;
    let Some(mut update) = updater
        .check()
        .await
        .map_err(|error| AppError::Update(format!("检查更新失败：{error}")))?
    else {
        return Ok(false);
    };

    // 检查请求沿用 30s 超时；安装包下载单独放宽，避免大包被误杀。
    update.timeout = Some(DOWNLOAD_TIMEOUT);

    let progress_app = app.clone();
    let mut downloaded = 0u64;
    let mut last_emit: Option<Instant> = None;
    let bytes = update
        .download(
            move |chunk_len, content_len| {
                downloaded = downloaded.saturating_add(chunk_len as u64);
                let now = Instant::now();
                let finished = content_len.is_some_and(|total| downloaded >= total);
                let due = last_emit
                    .map(|last| now.duration_since(last) >= PROGRESS_INTERVAL)
                    .unwrap_or(true);
                if due || finished {
                    last_emit = Some(now);
                    let _ = progress_app.emit(
                        UPDATE_DOWNLOAD_PROGRESS_EVENT,
                        UpdateDownloadProgress {
                            downloaded,
                            total: content_len,
                        },
                    );
                }
            },
            || {},
        )
        .await
        .map_err(|error| AppError::Update(format!("下载更新失败：{error}")))?;

    let _ = app.emit(UPDATE_INSTALL_STARTED_EVENT, update.version.clone());

    // Windows 的 NSIS 安装器由插件以 /P /UPDATE /R 启动，并直接退出当前进程；
    // 其余平台 install 返回后再显式重启。
    update
        .install(bytes)
        .map_err(|error| AppError::Update(format!("安装更新失败：{error}")))?;
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_info_serializes_camel_case() {
        let info = UpdateInfo {
            current_version: "0.1.0".into(),
            version: "0.1.1".into(),
            notes: Some("修复更新问题".into()),
            pub_date: Some("2026-10-23T00:00:00Z".into()),
        };
        let value = serde_json::to_value(info).unwrap();
        assert_eq!(value["currentVersion"], "0.1.0");
        assert_eq!(value["version"], "0.1.1");
        assert_eq!(value["notes"], "修复更新问题");
        assert_eq!(value["pubDate"], "2026-10-23T00:00:00Z");
    }
}
