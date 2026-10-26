//! 主窗口摆放：按整块显示器的几何中心居中，而不是 Tauri 默认的工作区居中。

use tauri::{PhysicalPosition, WebviewWindow};

/// 把窗口的可见内容（客户区）摆到它当前所在显示器的正中间。
///
/// Tauri 的 `center`（含 `tauri.conf.json` 的 `"center": true`）用的是 `work_area()`，
/// 会扣掉任务栏，窗口因此整体上移半个任务栏高度；这里改用完整 `size()`。
/// 返回 Err 只表示「没能摆位」，不影响启动。
pub fn center_on_screen(window: &WebviewWindow) -> Result<(), String> {
    let monitor = window
        .current_monitor()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "找不到窗口所在的显示器".to_string())?;
    let outer = window.outer_position().map_err(|e| e.to_string())?;
    let inner = window.inner_position().map_err(|e| e.to_string())?;
    let inner_size = window.inner_size().map_err(|e| e.to_string())?;
    let monitor_pos = monitor.position();
    let monitor_size = monitor.size();

    // 无边框窗口的不可见阴影边框左右各有几像素、上下不等宽，直接按外框居中会让
    // 可见内容偏上，所以先算客户区的目标位置，再倒推回 set_position 要的外框坐标。
    let border = (inner.x - outer.x, inner.y - outer.y);
    let target = (
        monitor_pos.x + (monitor_size.width as i32 - inner_size.width as i32) / 2,
        monitor_pos.y + (monitor_size.height as i32 - inner_size.height as i32) / 2,
    );

    window
        .set_position(PhysicalPosition::new(target.0 - border.0, target.1 - border.1))
        .map_err(|e| e.to_string())
}
