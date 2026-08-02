//! 设置存储 + 开机启动
//!
//! 用户 2026-08-02 原则：除 App 本体外不产生未知临时文件。
//! 设置走 `tauri-plugin-store`（JSON 存系统标准应用配置目录，卸载随 App 走，
//! 非散落临时文件）。开机启动走 `tauri-plugin-autostart`（系统登录项/注册表，
//! 不产生用户可见文件）。凭证走 AES-256-GCM 加密文件（随机主密钥，见 keychain.rs）。
//! 三者分工清晰。

use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_store::StoreExt as _;

const STORE_PATH: &str = "settings.json";
const KEY_LAUNCH_AT_LOGIN: &str = "launch_at_login";
const KEY_REFRESH_INTERVAL: &str = "refresh_interval_secs";
const KEY_CARD_ORDER: &str = "card_order";

/// 默认后台刷新间隔：5 分钟
pub const DEFAULT_INTERVAL_SECS: u64 = 300;
/// 可选间隔档位（秒），供前端下拉
pub const INTERVAL_OPTIONS: &[u64] = &[60, 180, 300, 600, 900, 1800];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub launch_at_login: bool,
    pub refresh_interval_secs: u64,
    /// 卡片排序（provider_id 数组）。空表示按默认（紧张度）排序。
    #[serde(default)]
    pub card_order: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            launch_at_login: false,
            refresh_interval_secs: DEFAULT_INTERVAL_SECS,
            card_order: Vec::new(),
        }
    }
}

/// 读取设置（不存在则返回默认）。
pub fn load(app: &AppHandle) -> Settings {
    let Ok(store) = app.store(STORE_PATH) else {
        return Settings::default();
    };
    let launch = store
        .get(KEY_LAUNCH_AT_LOGIN)
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let interval = store
        .get(KEY_REFRESH_INTERVAL)
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_INTERVAL_SECS);
    let card_order = store
        .get(KEY_CARD_ORDER)
        .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
        .unwrap_or_default();
    Settings {
        launch_at_login: launch,
        refresh_interval_secs: interval,
        card_order,
    }
}

/// 应用开机启动设置到系统（登录项）。
fn apply_autostart(app: &AppHandle, enable: bool) -> anyhow::Result<()> {
    let mgr = app.autolaunch();
    if enable {
        mgr.enable()?;
    } else {
        mgr.disable()?;
    }
    Ok(())
}

/// 保存设置并应用副作用（开机启动即时生效；刷新间隔由 scheduler 通过 watch 感知）。
pub fn save(app: &AppHandle, settings: &Settings) -> anyhow::Result<()> {
    let store = app.store(STORE_PATH)?;
    store.set(KEY_LAUNCH_AT_LOGIN, serde_json::json!(settings.launch_at_login));
    store.set(
        KEY_REFRESH_INTERVAL,
        serde_json::json!(settings.refresh_interval_secs),
    );
    store.set(KEY_CARD_ORDER, serde_json::json!(settings.card_order));
    store.save()?;
    apply_autostart(app, settings.launch_at_login)?;
    Ok(())
}
