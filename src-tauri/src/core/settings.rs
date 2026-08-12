//! 设置持久化（Core 层，不依赖 Tauri）。
//!
//! 用户 2026-08-02 原则：除 App 本体外不产生未知临时文件。
//! 设置写入数据目录下的 settings.json（与凭证同目录，卸载随 App 走），
//! 0600 权限。开机启动的系统级副作用由 platform/commands 层负责。

use crate::core::store::{data_dir, ensure_private_permissions, write_private};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 默认后台刷新间隔：5 分钟
pub const DEFAULT_INTERVAL_SECS: u64 = 300;
/// 可选间隔档位（秒），供前端下拉
pub const INTERVAL_OPTIONS: &[u64] = &[60, 180, 300, 600, 900, 1800];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    Classic,
    Parchment,
    Cyberpunk,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub launch_at_login: bool,
    pub refresh_interval_secs: u64,
    /// 卡片排序（provider_id 数组）。空表示按默认（紧张度）排序。
    #[serde(default)]
    pub card_order: Vec<String>,
    /// 账号昵称：key 为账号实例 ID。空/缺失时，前端回退到平台返回的邮箱或 ID。
    #[serde(default)]
    pub account_nicknames: HashMap<String, String>,
    /// 显式视觉主题。经典主题延续原有 Polar Silver 并跟随系统明暗模式。
    #[serde(default)]
    pub theme: Theme,
    /// 主题的明暗外观独立于风格；system 会实时跟随操作系统。
    #[serde(default)]
    pub appearance: Appearance,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            launch_at_login: false,
            refresh_interval_secs: DEFAULT_INTERVAL_SECS,
            card_order: Vec::new(),
            account_nicknames: HashMap::new(),
            theme: Theme::default(),
            appearance: Appearance::default(),
        }
    }
}

fn settings_path() -> anyhow::Result<std::path::PathBuf> {
    Ok(data_dir()?.join("settings.json"))
}

/// 严格读取设置。修改设置前必须走这个入口，避免损坏文件被默认值覆盖。
pub fn load_strict() -> anyhow::Result<Settings> {
    let p = settings_path()?;
    if !p.exists() {
        return Ok(Settings::default());
    }
    ensure_private_permissions(&p)?;
    let json = std::fs::read_to_string(&p)?;
    serde_json::from_str(&json)
        .with_context(|| format!("设置文件 {} 已损坏，原文件保持不变", p.display()))
}

/// 启动读取设置。损坏时允许应用以默认值继续启动，但后续保存仍会由
/// `load_strict` 拦截，不会覆盖原文件。
pub fn load() -> Settings {
    load_strict().unwrap_or_else(|error| {
        log::error!("读取设置失败，将使用默认值启动: {error:#}");
        Settings::default()
    })
}

/// 保存设置（0600 权限写文件）。
pub fn save(settings: &Settings) -> anyhow::Result<()> {
    let p = settings_path()?;
    let json = serde_json::to_string_pretty(settings)?;
    write_private(&p, json.as_bytes())
}

/// 删除账号后同步清理排序与昵称引用，避免重新添加时复用陈旧设置。
pub fn remove_account_references(settings: &mut Settings, account_id: &str) {
    settings.card_order.retain(|id| id != account_id);
    settings.account_nicknames.remove(account_id);
}

#[cfg(test)]
mod tests {
    use super::{remove_account_references, Settings};

    #[test]
    fn legacy_settings_default_to_no_account_nicknames() {
        let settings: Settings = serde_json::from_str(
            r#"{
                "launch_at_login": false,
                "refresh_interval_secs": 300,
                "card_order": ["codex"]
            }"#,
        )
        .expect("legacy settings should deserialize");

        assert!(settings.account_nicknames.is_empty());
        assert_eq!(settings.theme, super::Theme::Classic);
        assert_eq!(settings.appearance, super::Appearance::System);
    }

    #[test]
    fn removing_account_cleans_order_and_nickname() {
        let mut settings = Settings {
            card_order: vec!["codex".into(), "moonshot".into()],
            ..Settings::default()
        };
        settings
            .account_nicknames
            .insert("moonshot".into(), "备用".into());

        remove_account_references(&mut settings, "moonshot");

        assert_eq!(settings.card_order, vec!["codex"]);
        assert!(!settings.account_nicknames.contains_key("moonshot"));
    }
}
