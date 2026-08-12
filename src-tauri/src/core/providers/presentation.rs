//! 供应商快照 → 标准账户卡片契约。
//!
//! Provider 只需要把原始 API 映射成 `ProviderSnapshot`，并通过 `CardConfig`
//! 声明少量展示语义（余额角色、货币窗口角色、套餐档位）。前端只渲染
//! `CardItem::Quota` 与 `CardItem::Balance`，不再感知具体供应商。

use super::{
    Balance, BillingMode, Brand, HealthStatus, ProviderSnapshot, QuotaUnit, QuotaWindow,
    WindowPeriod,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct BrandStyle {
    pub key: &'static str,
    pub accent_light: &'static str,
    pub accent_alt_light: &'static str,
    pub accent_dark: &'static str,
    pub accent_alt_dark: &'static str,
}

impl Brand {
    pub fn style(self) -> BrandStyle {
        match self {
            Brand::Anthropic => BrandStyle {
                key: "anthropic",
                accent_light: "#c15f3c",
                accent_alt_light: "#9f472b",
                accent_dark: "#e58b69",
                accent_alt_dark: "#c96a47",
            },
            Brand::OpenAI => BrandStyle {
                key: "openai",
                accent_light: "#10a37f",
                accent_alt_light: "#0c8a6b",
                accent_dark: "#10a37f",
                accent_alt_dark: "#0c8a6b",
            },
            Brand::OpenRouter => BrandStyle {
                key: "openrouter",
                accent_light: "#5b52d6",
                accent_alt_light: "#3f36b5",
                accent_dark: "#9992ff",
                accent_alt_dark: "#756de6",
            },
            Brand::Kimi => BrandStyle {
                key: "kimi",
                accent_light: "#303845",
                accent_alt_light: "#141a22",
                accent_dark: "#c3ccd7",
                accent_alt_dark: "#8f9aa8",
            },
            Brand::Moonshot => BrandStyle {
                key: "moonshot",
                accent_light: "#303845",
                accent_alt_light: "#141a22",
                accent_dark: "#c3ccd7",
                accent_alt_dark: "#8f9aa8",
            },
            Brand::DeepSeek => BrandStyle {
                key: "deepseek",
                accent_light: "#4d6bfe",
                accent_alt_light: "#3a52e6",
                accent_dark: "#6f8aff",
                accent_alt_dark: "#4d6bfe",
            },
            Brand::Glm => BrandStyle {
                key: "glm",
                accent_light: "#6b57e8",
                accent_alt_light: "#4d39c9",
                accent_dark: "#a79bff",
                accent_alt_dark: "#8172f2",
            },
            Brand::MiniMax => BrandStyle {
                key: "minimax",
                accent_light: "#2b52ff",
                accent_alt_light: "#1738d6",
                accent_dark: "#7891ff",
                accent_alt_dark: "#526eff",
            },
            Brand::Tencent => BrandStyle {
                key: "tencent",
                accent_light: "#0052d9",
                accent_alt_light: "#0046b8",
                accent_dark: "#5c92f2",
                accent_alt_dark: "#3979e6",
            },
            Brand::Gemini => BrandStyle {
                key: "gemini",
                accent_light: "#4666d5",
                accent_alt_light: "#9b58c7",
                accent_dark: "#8ca6ff",
                accent_alt_dark: "#d18cff",
            },
            Brand::SiliconFlow => BrandStyle {
                key: "siliconflow",
                accent_light: "#2f6dd5",
                accent_alt_light: "#704fc7",
                accent_dark: "#67a2ff",
                accent_alt_dark: "#a786ff",
            },
            Brand::Volcengine => BrandStyle {
                key: "volcengine",
                accent_light: "#1769d2",
                accent_alt_light: "#0aa3a5",
                accent_dark: "#62a4ff",
                accent_alt_dark: "#31d5cf",
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CardProvider {
    pub id: String,
    pub name: String,
    pub brand: BrandStyle,
    pub detail_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardPlan {
    pub name: String,
    /// 按大致月价统一后的 0–5 档；None 表示无法判断。
    pub tier: Option<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CardPrimary {
    pub label: String,
    pub value: Option<f64>,
    pub unit: Option<QuotaUnit>,
    /// 状态点与默认排序使用的“已使用比例”，与主值的剩余比例分开。
    pub health_used_percent: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CardItem {
    Quota {
        label: String,
        used_percent: Option<f64>,
        used: Option<f64>,
        limit: Option<f64>,
        unit: QuotaUnit,
        reset_at: Option<i64>,
    },
    Balance {
        label: String,
        value: Option<f64>,
        unit: QuotaUnit,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountCardModel {
    pub account_id: String,
    pub account_label: Option<String>,
    pub provider: CardProvider,
    pub plan: Option<CardPlan>,
    pub primary: CardPrimary,
    pub items: Vec<CardItem>,
    pub status: HealthStatus,
    pub fetched_at: i64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum BalanceRole {
    /// 按量账户：余额总额是主值，充值/赠送金额作为明细。
    Primary {
        topped_up_label: &'static str,
        granted_label: &'static str,
    },
    /// 订阅账户的补充余额，例如 Codex Credit。
    Supplemental {
        label: &'static str,
    },
    Hidden,
}

#[derive(Debug, Clone, Copy)]
pub enum CurrencyWindowRole {
    /// 有上限时按 quota；无上限时按 balance/统计值。
    Standard,
    /// 货币限额不是套餐主额度，以“限额 - 已用”的余额行展示。
    RemainingBalance { label_suffix: &'static str },
}

#[derive(Debug, Clone, Copy)]
pub enum PrimaryWindowSelection {
    HighestPeriod,
    HighestNonCurrency,
    MostUsed,
}

#[derive(Debug, Clone, Copy)]
pub struct CardConfig {
    pub detail_url: Option<&'static str>,
    pub balance_role: BalanceRole,
    pub currency_window_role: CurrencyWindowRole,
    pub primary_window: PrimaryWindowSelection,
}

impl CardConfig {
    pub fn for_billing(billing: BillingMode) -> Self {
        let balance_role = match billing {
            BillingMode::Subscription => BalanceRole::Hidden,
            BillingMode::PayAsYouGo => BalanceRole::Primary {
                topped_up_label: "充值余额",
                granted_label: "赠送余额",
            },
        };
        Self {
            detail_url: None,
            balance_role,
            currency_window_role: CurrencyWindowRole::Standard,
            primary_window: PrimaryWindowSelection::HighestPeriod,
        }
    }
}

fn is_currency(unit: &QuotaUnit) -> bool {
    matches!(unit, QuotaUnit::Currency(_))
}

fn usage_percent(window: &QuotaWindow) -> Option<f64> {
    window
        .used
        .or_else(|| match (window.used_raw, window.limit) {
            (Some(used), Some(limit)) if limit > 0.0 => {
                Some((used / limit * 100.0).clamp(0.0, 100.0))
            }
            _ => None,
        })
}

fn remaining_amount(window: &QuotaWindow) -> Option<f64> {
    window
        .remaining
        .map(|value| value.max(0.0))
        .or_else(|| match (window.limit, window.used_raw) {
            (Some(limit), Some(used)) => Some((limit - used).max(0.0)),
            _ => None,
        })
        .or_else(|| match (window.limit, window.used) {
            (Some(limit), Some(used_percent)) => {
                Some((limit * (1.0 - used_percent / 100.0)).max(0.0))
            }
            _ => None,
        })
}

fn period_priority(period: WindowPeriod) -> i32 {
    match period {
        WindowPeriod::Month => 5,
        WindowPeriod::Week => 4,
        WindowPeriod::Day => 3,
        WindowPeriod::Hours5 => 2,
        WindowPeriod::Custom(_) => 1,
    }
}

fn primary_window(
    snapshot: &ProviderSnapshot,
    selection: PrimaryWindowSelection,
) -> Option<&QuotaWindow> {
    let candidates = snapshot
        .windows
        .iter()
        .filter(|window| window.limit.is_some_and(|limit| limit > 0.0))
        .filter(|window| {
            !matches!(selection, PrimaryWindowSelection::HighestNonCurrency)
                || !is_currency(&window.unit)
        });
    match selection {
        PrimaryWindowSelection::MostUsed => candidates.max_by(|a, b| {
            usage_percent(a)
                .unwrap_or(0.0)
                .total_cmp(&usage_percent(b).unwrap_or(0.0))
        }),
        _ => candidates.max_by_key(|window| period_priority(window.period)),
    }
}

fn primary_currency_value(snapshot: &ProviderSnapshot) -> Option<&QuotaWindow> {
    snapshot
        .windows
        .iter()
        .filter(|window| is_currency(&window.unit))
        .filter(|window| window.limit.is_none() && window.used_raw.is_some())
        .max_by_key(|window| period_priority(window.period))
}

fn primary_metric(snapshot: &ProviderSnapshot, config: CardConfig) -> CardPrimary {
    match snapshot.billing {
        BillingMode::Subscription => {
            let Some(window) = primary_window(snapshot, config.primary_window) else {
                return CardPrimary {
                    label: "未提供额度余量".into(),
                    value: None,
                    unit: None,
                    health_used_percent: None,
                };
            };
            let remaining = remaining_amount(window);
            // 套餐主值统一显示剩余百分比；原始 token / 请求量仍完整保留在
            // 下方额度行中，避免主值在不同供应商之间一会儿是数量、一会儿是百分比。
            let remaining_percent = usage_percent(window).map(|used| (100.0 - used).max(0.0));
            let (value, unit) = match remaining_percent {
                Some(value) => (Some(value), Some(QuotaUnit::Percent)),
                None => (remaining, Some(window.unit.clone())),
            };
            CardPrimary {
                label: format!("{}余量", window.label),
                value,
                unit,
                health_used_percent: usage_percent(window),
            }
        }
        BillingMode::PayAsYouGo => match snapshot.balance.as_ref() {
            Some(balance) => CardPrimary {
                label: if balance.available {
                    "可用余额"
                } else {
                    "可用余额 · 余额不足"
                }
                .into(),
                value: Some(balance.total),
                unit: Some(QuotaUnit::Currency(balance.currency.clone())),
                health_used_percent: None,
            },
            None => match primary_currency_value(snapshot) {
                Some(window) => CardPrimary {
                    label: window.label.clone(),
                    value: window.used_raw,
                    unit: Some(window.unit.clone()),
                    health_used_percent: None,
                },
                None => CardPrimary {
                    label: "未提供余额".into(),
                    value: None,
                    unit: None,
                    health_used_percent: None,
                },
            },
        },
    }
}

fn quota_item(window: &QuotaWindow) -> CardItem {
    CardItem::Quota {
        label: window.label.clone(),
        used_percent: usage_percent(window),
        used: window.used_raw,
        limit: window.limit,
        unit: window.unit.clone(),
        reset_at: window.reset_at,
    }
}

fn value_item(window: &QuotaWindow) -> CardItem {
    let value = window
        .used_raw
        .or(window.remaining)
        .or_else(|| usage_percent(window));
    CardItem::Balance {
        label: window.label.clone(),
        value,
        unit: window.unit.clone(),
    }
}

fn remaining_balance_item(window: &QuotaWindow, label_suffix: &'static str) -> CardItem {
    CardItem::Balance {
        label: format!("{}{}", window.label, label_suffix),
        value: remaining_amount(window),
        unit: window.unit.clone(),
    }
}

fn balance_breakdown(balance: &Balance, role: BalanceRole) -> Vec<CardItem> {
    match role {
        BalanceRole::Primary {
            topped_up_label,
            granted_label,
        } => {
            let unit = QuotaUnit::Currency(balance.currency.clone());
            let mut items = Vec::new();
            if let Some(value) = balance.topped_up {
                items.push(CardItem::Balance {
                    label: topped_up_label.into(),
                    value: Some(value),
                    unit: unit.clone(),
                });
            }
            if let Some(value) = balance.granted {
                items.push(CardItem::Balance {
                    label: granted_label.into(),
                    value: Some(value),
                    unit,
                });
            }
            items
        }
        BalanceRole::Supplemental { label } => vec![CardItem::Balance {
            label: label.into(),
            value: Some(balance.total),
            unit: QuotaUnit::Currency(balance.currency.clone()),
        }],
        BalanceRole::Hidden => vec![],
    }
}

pub fn present(
    snapshot: &ProviderSnapshot,
    brand: Brand,
    config: CardConfig,
    plan_tier: Option<u8>,
) -> AccountCardModel {
    let mut quota_items = Vec::new();
    let mut window_values = Vec::new();

    for window in &snapshot.windows {
        if is_currency(&window.unit)
            && matches!(
                config.currency_window_role,
                CurrencyWindowRole::RemainingBalance { .. }
            )
        {
            let CurrencyWindowRole::RemainingBalance { label_suffix } = config.currency_window_role
            else {
                unreachable!();
            };
            window_values.push(remaining_balance_item(window, label_suffix));
        } else if window.limit.is_some_and(|limit| limit > 0.0) {
            quota_items.push(quota_item(window));
        } else {
            window_values.push(value_item(window));
        }
    }

    let mut items = quota_items;
    if let Some(balance) = snapshot.balance.as_ref() {
        items.extend(balance_breakdown(balance, config.balance_role));
    }
    items.extend(window_values);

    AccountCardModel {
        account_id: snapshot.account_id.clone(),
        account_label: snapshot.account_label.clone(),
        provider: CardProvider {
            id: snapshot.provider_id.clone(),
            name: snapshot.display_name.clone(),
            brand: brand.style(),
            detail_url: config.detail_url.map(str::to_string),
        },
        plan: snapshot.plan_name.as_ref().map(|name| CardPlan {
            name: name.clone(),
            tier: plan_tier.filter(|tier| *tier <= 5),
        }),
        primary: primary_metric(snapshot, config),
        items,
        status: snapshot.status,
        fetched_at: snapshot.fetched_at,
        last_error: snapshot.last_error.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::providers::{registry, Fidelity};

    fn snapshot(provider_id: &str, billing: BillingMode) -> ProviderSnapshot {
        ProviderSnapshot {
            account_id: format!("{provider_id}#1"),
            account_label: Some("user@example.com".into()),
            provider_id: provider_id.into(),
            display_name: provider_id.into(),
            plan_name: None,
            billing,
            balance: None,
            windows: vec![],
            fidelity: Fidelity::Exact,
            status: HealthStatus::Ok,
            fetched_at: 1,
            last_error: None,
        }
    }

    #[test]
    fn subscription_keeps_primary_quota_in_details() {
        let mut source = snapshot("codex", BillingMode::Subscription);
        source.plan_name = Some("Pro 5X".into());
        source.windows.push(QuotaWindow {
            period: WindowPeriod::Week,
            label: "本周".into(),
            used: Some(14.0),
            used_raw: None,
            limit: Some(100.0),
            remaining: Some(86.0),
            unit: QuotaUnit::Percent,
            reset_at: Some(99),
        });
        source.balance = Some(Balance {
            total: 0.0,
            granted: None,
            topped_up: Some(0.0),
            currency: "USD".into(),
            available: false,
        });
        let mut config = CardConfig::for_billing(source.billing);
        config.balance_role = BalanceRole::Supplemental {
            label: "Credit 余额",
        };

        let card = present(&source, Brand::OpenAI, config, Some(4));
        assert_eq!(card.primary.value, Some(86.0));
        assert_eq!(card.primary.health_used_percent, Some(14.0));
        assert_eq!(card.plan.as_ref().and_then(|plan| plan.tier), Some(4));
        assert!(matches!(card.items[0], CardItem::Quota { .. }));
        assert!(matches!(card.items[1], CardItem::Balance { .. }));
    }

    #[test]
    fn counted_subscription_uses_percent_for_primary_and_counts_for_detail() {
        let mut source = snapshot("glm_coding_plan", BillingMode::Subscription);
        source.windows.push(QuotaWindow {
            period: WindowPeriod::Week,
            label: "7 天额度".into(),
            used: Some(25.0),
            used_raw: Some(250.0),
            limit: Some(1000.0),
            remaining: Some(750.0),
            unit: QuotaUnit::Tokens,
            reset_at: None,
        });

        let card = present(
            &source,
            Brand::Glm,
            CardConfig::for_billing(source.billing),
            Some(4),
        );
        assert_eq!(card.primary.value, Some(75.0));
        assert_eq!(card.primary.unit, Some(QuotaUnit::Percent));
        match &card.items[0] {
            CardItem::Quota {
                used, limit, unit, ..
            } => {
                assert_eq!(*used, Some(250.0));
                assert_eq!(*limit, Some(1000.0));
                assert_eq!(*unit, QuotaUnit::Tokens);
            }
            _ => panic!("原始计数应保留在额度明细"),
        }
    }

    #[test]
    fn kimi_currency_limit_becomes_remaining_balance_not_primary() {
        let mut source = snapshot("kimi_code", BillingMode::Subscription);
        source.windows = vec![
            QuotaWindow {
                period: WindowPeriod::Week,
                label: "本周".into(),
                used: Some(20.0),
                used_raw: None,
                limit: Some(100.0),
                remaining: Some(80.0),
                unit: QuotaUnit::Percent,
                reset_at: None,
            },
            QuotaWindow {
                period: WindowPeriod::Month,
                label: "Extra Usage".into(),
                used: Some(25.0),
                used_raw: Some(25.0),
                limit: Some(100.0),
                remaining: Some(75.0),
                unit: QuotaUnit::Currency("CNY".into()),
                reset_at: None,
            },
        ];
        let mut config = CardConfig::for_billing(source.billing);
        config.primary_window = PrimaryWindowSelection::HighestNonCurrency;
        config.currency_window_role = CurrencyWindowRole::RemainingBalance {
            label_suffix: " 余额",
        };

        let card = present(&source, Brand::Kimi, config, Some(4));
        assert_eq!(card.primary.label, "本周余量");
        assert_eq!(card.primary.value, Some(80.0));
        assert!(matches!(card.items[0], CardItem::Quota { .. }));
        match &card.items[1] {
            CardItem::Balance { label, value, .. } => {
                assert_eq!(label, "Extra Usage 余额");
                assert_eq!(*value, Some(75.0));
            }
            _ => panic!("Extra Usage 应映射为余额行"),
        }
    }

    #[test]
    fn payg_balance_uses_configured_breakdown_labels() {
        let mut source = snapshot("moonshot", BillingMode::PayAsYouGo);
        source.balance = Some(Balance {
            total: 50.0,
            granted: Some(10.0),
            topped_up: Some(40.0),
            currency: "CNY".into(),
            available: true,
        });
        let mut config = CardConfig::for_billing(source.billing);
        config.balance_role = BalanceRole::Primary {
            topped_up_label: "现金余额",
            granted_label: "代金券余额",
        };

        let card = present(&source, Brand::Moonshot, config, None);
        assert_eq!(card.primary.value, Some(50.0));
        match &card.items[0] {
            CardItem::Balance { label, .. } => assert_eq!(label, "现金余额"),
            _ => panic!("现金余额应是余额行"),
        }
        match &card.items[1] {
            CardItem::Balance { label, .. } => assert_eq!(label, "代金券余额"),
            _ => panic!("代金券余额应是余额行"),
        }
    }

    #[test]
    fn unbounded_usage_windows_become_value_rows() {
        let mut source = snapshot("openai_platform", BillingMode::PayAsYouGo);
        source.windows.push(QuotaWindow {
            period: WindowPeriod::Month,
            label: "本月花费".into(),
            used: None,
            used_raw: Some(12.5),
            limit: None,
            remaining: None,
            unit: QuotaUnit::Currency("USD".into()),
            reset_at: None,
        });

        let card = present(
            &source,
            Brand::OpenAI,
            CardConfig::for_billing(source.billing),
            None,
        );
        assert_eq!(card.primary.label, "本月花费");
        assert_eq!(card.primary.value, Some(12.5));
        assert_eq!(card.primary.unit, Some(QuotaUnit::Currency("USD".into())));
        assert!(matches!(card.items[0], CardItem::Balance { .. }));
        let json = serde_json::to_string(&card).expect("卡片应可序列化");
        assert!(json.contains("\"kind\":\"balance\""));
    }

    #[test]
    fn every_registered_provider_can_emit_the_standard_contract() {
        let providers = registry();
        assert_eq!(providers.len(), 18);

        for provider in providers {
            let mut source = snapshot(provider.id(), provider.billing_mode());
            source.display_name = provider.display_name().into();
            let card = provider.present(&source);
            assert_eq!(card.provider.id, provider.id());
            assert!(!card.provider.brand.key.is_empty());
            serde_json::to_string(&card).expect("所有 Provider 都应能输出标准卡片契约");
        }
    }
}
