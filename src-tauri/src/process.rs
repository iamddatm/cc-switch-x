//! 外部子进程的启动工具。

use std::ffi::OsStr;
use std::process::Command;

/// 构造一个不会弹出控制台窗口的外部命令。
///
/// 发布版是 `windows_subsystem = "windows"` 的无控制台 GUI 进程；直接 spawn 控制台
/// 程序（git、qoderclicn、cmd 等）时，Windows 会为子进程新建并显示控制台窗口。
/// 所有对外部 CLI 的调用都应走这里，避免界面上闪黑窗。
pub fn hidden_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Win32 CreateProcess 的 CREATE_NO_WINDOW 标志。
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}