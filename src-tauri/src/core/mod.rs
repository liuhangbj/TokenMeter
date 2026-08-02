//! 平台无关核心层（三层架构的 Core）。
//!
//! 这里的代码不依赖 Tauri / 窗口 / 托盘：
//! - `providers`：8 个平台的额度抓取与 OAuth 凭证逻辑
//! - `store`：AES-256-GCM 凭证加密存储
//! - `scheduler`：定时/触发式刷新调度
//! - `settings`：设置文件持久化
//! - `oauth_codex` / `oauth_device`：OAuth 流程（浏览器打开由 platform 层注入）
//!
//! 平台差异（托盘、窗口事件、系统浏览器、Dock 策略）全部收敛在 `crate::platform`。

pub mod oauth_codex;
pub mod oauth_device;
pub mod providers;
pub mod scheduler;
pub mod scheduler_ctl;
pub mod settings;
pub mod store;
