//! 开机自启：系统登录项才是唯一真源，app 不额外保存一份状态。
//!
//! Windows 写 HKCU 的 Run 键，macOS 写登录项，Linux 写 XDG autostart，
//! 都不需要管理员权限。

use crate::error::AppError;
use auto_launch::{AutoLaunch, AutoLaunchBuilder};
#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};

const APP_NAME: &str = "CC Switch X";

/// macOS 的登录项需要 .app bundle 路径；直接指向 Contents/MacOS 下的可执行文件会多开一个终端窗口。
#[cfg(target_os = "macos")]
fn macos_bundle_path(exe_path: &Path) -> Option<PathBuf> {
    let path = exe_path.to_string_lossy();
    let position = path.find(".app/Contents/MacOS/")?;
    Some(PathBuf::from(&path[..position + 4]))
}

fn launch() -> Result<AutoLaunch, AppError> {
    let exe_path = std::env::current_exe()
        .map_err(|error| AppError::Autostart(format!("无法获取应用路径：{error}")))?;
    #[cfg(target_os = "macos")]
    let app_path = macos_bundle_path(&exe_path).unwrap_or(exe_path);
    #[cfg(not(target_os = "macos"))]
    let app_path = exe_path;

    AutoLaunchBuilder::new()
        .set_app_name(APP_NAME)
        .set_app_path(&app_path.to_string_lossy())
        .build()
        .map_err(|error| AppError::Autostart(format!("初始化登录项失败：{error}")))
}

pub fn enable() -> Result<(), AppError> {
    launch()?
        .enable()
        .map_err(|error| AppError::Autostart(format!("启用开机自启失败：{error}")))
}

pub fn disable() -> Result<(), AppError> {
    launch()?
        .disable()
        .map_err(|error| AppError::Autostart(format!("关闭开机自启失败：{error}")))
}

pub fn is_enabled() -> Result<bool, AppError> {
    launch()?
        .is_enabled()
        .map_err(|error| AppError::Autostart(format!("读取开机自启状态失败：{error}")))
}
