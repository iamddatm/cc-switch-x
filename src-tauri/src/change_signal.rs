//! 真源变更信号：`store` 落盘后打点，自动上传工作线程负责防抖与合并。
//!
//! 只有「是否发生过写入」这一位信息，通道容量 1：连续写入天然合并成一次唤醒，
//! 时间窗（1 秒防抖、10 秒上限）由工作线程自己计时，见 `services::backup`。
//! 放在叶子模块里，避免 `store` 反向依赖 `services`。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::OnceLock;

static TX: OnceLock<SyncSender<()>> = OnceLock::new();
static SUPPRESS_DEPTH: AtomicUsize = AtomicUsize::new(0);

/// 抑制窗口：恢复自备份、回滚等批量写入期间不该触发自动上传。
/// 用独立计数而不是布尔，嵌套调用也能正确恢复。
pub struct SuppressionGuard;

impl SuppressionGuard {
    pub fn new() -> Self {
        SUPPRESS_DEPTH.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Default for SuppressionGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for SuppressionGuard {
    fn drop(&mut self) {
        let _ = SUPPRESS_DEPTH.try_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
            Some(value.saturating_sub(1))
        });
    }
}

pub fn is_suppressed() -> bool {
    SUPPRESS_DEPTH.load(Ordering::SeqCst) > 0
}

/// 记录一次真源写入。没有工作线程（测试、CLI）时静默丢弃。
pub fn notify() {
    if is_suppressed() {
        return;
    }
    let Some(tx) = TX.get() else {
        return;
    };
    // 队列已满说明已经有一次待处理的唤醒，再丢一个信号正是想要的行为。
    let _ = tx.try_send(());
}

/// 安装信号通道并交给工作线程。重复调用返回 `None`，保证只有一个消费者。
pub(crate) fn install() -> Option<Receiver<()>> {
    let (tx, rx) = sync_channel::<()>(1);
    if TX.set(tx).is_err() {
        return None;
    }
    Some(rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suppression_guard_restores_state_after_drop() {
        let before = is_suppressed();
        {
            let _guard = SuppressionGuard::new();
            assert!(is_suppressed());
        }
        assert_eq!(is_suppressed(), before);
    }

    #[test]
    fn notify_without_worker_is_a_noop() {
        // 测试进程里没有装工作线程；这里只验证不 panic。
        notify();
    }
}
