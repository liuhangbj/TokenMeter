//! 设置持久化（Core 层，不依赖 Tauri）。
//!
//! 用户 2026-08-02 原则：除 App 本体外不产生未知临时文件。
//! 设置写入数据目录下的 settings.json（与凭证同目录，卸载随 App 走），
//! 0600 权限。开机启动的系统级副作用由 platform/commands 层负责。

use crate::core::store::{data_dir, write_private};
use serde::{Deserialize, Serialize};

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

fn settings_path() -> anyhow::Result<std::path::PathBuf> {
    Ok(data_dir()?.join("settings.json"))
}

/// 读取设置（不存在或损坏则返回默认）。
pub fn load() -> Settings {
    let Ok(p) = settings_path() else {
        return Settings::default();
    };
    std::fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// 保存设置（0600 权限写文件）。
pub fn save(settings: &Settings) -> anyhow::Result<()> {
    let p = settings_path()?;
    let json = serde_json::to_string_pretty(settings)?;
    write_private(&p, json.as_bytes())
}
