//! 开机自启：系统登录项才是唯一真源，app 不额外保存一份状态。
//!
//! Windows 写 HKCU 的 Run 键，macOS 写登录项，Linux 写 XDG autostart，
//! 都不需要管理员权限。

use crate::error::AppError;
use auto_launch::{AutoLaunch, AutoLaunchBuilder};
#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};

const APP_NAME: &str = "CC Switch X";

/// 登录项追加的启动参数：开机自启时只驻留托盘，不弹主窗口。
pub const STARTUP_FLAG: &str = "--autostart";

/// macOS 的登录项需要 .app bundle 路径；直接指向 Contents/MacOS 下的可执行文件会多开一个终端窗口。
#[cfg(target_os = "macos")]
fn macos_bundle_path(exe_path: &Path) -> Option<PathBuf> {
    let path = exe_path.to_string_lossy();
    let position = path.find(".app/Contents/MacOS/")?;
    Some(PathBuf::from(&path[..position + 4]))
}

fn has_startup_flag(args: impl IntoIterator<Item = String>) -> bool {
    args.into_iter().any(|arg| arg == STARTUP_FLAG)
}

/// 本次进程是否由登录项拉起；只认完整参数，避免把 `--autostart=true` 之类误判。
pub fn launched_at_login() -> bool {
    has_startup_flag(std::env::args())
}

fn launch() -> Result<AutoLaunch, AppError> {
    let exe_path = std::env::current_exe()
        .map_err(|error| AppError::Autostart(format!("无法获取应用路径：{error}")))?;
    #[cfg(target_os = "macos")]
    let app_path = macos_bundle_path(&exe_path).unwrap_or(exe_path);
    #[cfg(not(target_os = "macos"))]
    let app_path = exe_path;
    let app_path = app_path.to_string_lossy().into_owned();

    // Windows 的登录项值被系统按命令行解析，安装路径含空格（"CC Switch X"）时必须自带引号，
    // 否则追加参数后可能找不到可执行文件。
    #[cfg(target_os = "windows")]
    let app_path = format!("\"{app_path}\"");

    AutoLaunchBuilder::new()
        .set_app_name(APP_NAME)
        .set_app_path(&app_path)
        .set_args(&[STARTUP_FLAG])
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

/// 命令行是否恰好只有可执行文件本身（没有参数）：旧写法不加引号，整串就是路径。
#[cfg(target_os = "windows")]
fn command_is_bare_path_to(command: &str, exe: &str) -> bool {
    let trimmed = command.trim();
    match trimmed.strip_prefix('"') {
        Some(rest) => rest
            .split_once('"')
            .map(|(path, tail)| path == exe && tail.trim().is_empty())
            .unwrap_or(false),
        None => trimmed == exe,
    }
}

/// 旧版本写入的登录项没有启动参数；只有它当前就指向本程序时才补写一次，升级后不用手动关开。
/// 未启用、已带引号或指向别处（开发构建、便携副本）都不动，系统登录项仍是唯一真源。
#[cfg(target_os = "windows")]
pub fn refresh_if_enabled() -> Result<bool, AppError> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;

    let exe_path = std::env::current_exe()
        .map_err(|error| AppError::Autostart(format!("无法获取应用路径：{error}")))?
        .to_string_lossy()
        .into_owned();
    let launch = launch()?;
    let enabled = launch
        .is_enabled()
        .map_err(|error| AppError::Autostart(format!("读取开机自启状态失败：{error}")))?;
    if !enabled {
        return Ok(false);
    }
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(key) = hkcu.open_subkey_with_flags(
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run",
        KEY_READ,
    ) else {
        return Ok(false);
    };
    let Ok(command) = key.get_value::<String, _>(APP_NAME) else {
        return Ok(false);
    };
    if !command_is_bare_path_to(&command, &exe_path) {
        return Ok(false);
    }
    launch
        .enable()
        .map_err(|error| AppError::Autostart(format!("更新开机自启登录项失败：{error}")))?;
    Ok(true)
}

/// 登录项不支持启动参数的平台保持原行为，不做迁移。
#[cfg(not(target_os = "windows"))]
pub fn refresh_if_enabled() -> Result<bool, AppError> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    #[test]
    fn startup_flag_requires_exact_match() {
        let args = |list: &[&str]| {
            super::has_startup_flag(list.iter().map(|arg| arg.to_string()))
        };
        assert!(args(&["cc-switch-x.exe", "--autostart"]));
        assert!(!args(&["cc-switch-x.exe", "--autostart=true"]));
        assert!(!args(&["cc-switch-x.exe"]));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn only_bare_old_entries_get_upgraded() {
        let exe = r"C:\Users\me\AppData\Local\CC Switch X\cc-switch-x.exe";
        let bare = |command: &str| super::command_is_bare_path_to(command, exe);
        assert!(bare(&format!("{exe} ")));
        assert!(bare(&format!("\"{exe}\"")));
        // 已带参数（无论新旧写法）都算已迁移，不重复写注册表
        assert!(!bare(&format!("\"{exe}\" --autostart")));
        // 指向别处（开发构建、便携副本）不动
        assert!(!bare(r"C:\dev\target\debug\cc-switch-x.exe"));
        assert!(!bare(&format!("\"{exe}\" --other")));
    }
}
