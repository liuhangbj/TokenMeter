//! 调度控制 —— 刷新间隔动态调整 + 面板触发立即刷新。
//!
//! 刷新逻辑（用户 2026-08-02 定）：
//!   - 点开面板 → 前端调 command 立即触发一次全量刷新
//!   - 否则按设置的后台间隔周期性刷新
//! 实现：
//!   - `watch<u64>` 承载当前间隔（秒），改设置即广播，scheduler 即时采用新间隔
//!   - `Notify` 承载"立即刷新"信号，前端点开面板时触发

use std::sync::Arc;
use tokio::sync::{watch, Notify};

/// 调度控制句柄：间隔广播 + 立即刷新信号
#[derive(Clone)]
pub struct SchedulerCtl {
    pub interval_tx: watch::Sender<u64>,
    pub refresh_now: Arc<Notify>,
}

pub fn new_ctl(initial_interval_secs: u64) -> SchedulerCtl {
    let (interval_tx, _) = watch::channel(initial_interval_secs);
    SchedulerCtl {
        interval_tx,
        refresh_now: Arc::new(Notify::new()),
    }
}

impl SchedulerCtl {
    /// 更新后台刷新间隔（秒），scheduler 立即采用。
    pub fn set_interval(&self, secs: u64) {
        let _ = self.interval_tx.send(secs);
    }

    /// 触发一次立即刷新（前端点开面板时调用）。
    pub fn trigger_refresh(&self) {
        self.refresh_now.notify_one();
    }
}
