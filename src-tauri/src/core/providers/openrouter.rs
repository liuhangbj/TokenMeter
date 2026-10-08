//! OpenRouter API（按量计费）。
//!
//! `/credits` 提供累计充值与累计消耗，但官方当前要求 Management Key；OAuth
//! 和普通 API Key 只能可靠调用 `/key`。两个端点互为降级来源：有 Management
//! Key 时展示账户余额，普通 Key 至少展示自身预算与日/周/月花费。

use super::*;
use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;

const BASE_URL: &str = "https://openrouter.ai/api/v1";

pub struct OpenRouterProvider;

impl OpenRouterProvider {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize)]
struct CreditsResponse {
    data: CreditsData,
}

#[derive(Debug, Deserialize)]
struct CreditsData {
    total_credits: f64,
    total_usage: f64,
}

#[derive(Debug, Deserialize)]
struct KeyResponse {
    data: KeyData,
}

#[derive(Debug, Deserialize)]
struct KeyData {
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    creator_user_id: Option<String>,
    #[serde(default)]
    limit: Option<f64>,
    #[serde(default)]
    limit_remaining: Option<f64>,
    #[serde(default)]
    limit_reset: Option<String>,
    #[serde(default)]
    usage: Option<f64>,
    #[serde(default)]
    usage_daily: Option<f64>,
    #[serde(default)]
    usage_weekly: Option<f64>,
    #[serde(default)]
    usage_monthly: Option<f64>,
}

fn clean_label(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn key_period(reset: Option<&str>) -> WindowPeriod {
    match reset.unwrap_or("").trim().to_lowercase().as_str() {
        "daily" | "day" => WindowPeriod::Day,
        "weekly" | "week" => WindowPeriod::Week,
        "monthly" | "month" => WindowPeriod::Month,
        _ => WindowPeriod::Custom(0),
    }
}

fn key_quota(key: &KeyData) -> Option<QuotaWindow> {
    let limit = key.limit.filter(|limit| *limit > 0.0)?;
    let used_raw = key
        .limit_remaining
        .map(|remaining| limit - remaining.clamp(0.0, limit))
        .or(match key.limit_reset.as_deref() {
            Some("daily") => key.usage_daily,
            Some("weekly") => key.usage_weekly,
            Some("monthly") => key.usage_monthly,
            _ => key.usage,
        })
        .unwrap_or(0.0)
        .max(0.0);
    let used = (used_raw / limit * 100.0).clamp(0.0, 100.0);
    Some(QuotaWindow {
        period: key_period(key.limit_reset.as_deref()),
        label: "API Key 预算".into(),
        used: Some(used),
        used_raw: Some(used_raw),
        limit: Some(limit),
        remaining: Some((limit - used_raw).max(0.0)),
        unit: QuotaUnit::Currency("USD".into()),
        reset_at: None,
    })
}

fn spend_window(period: WindowPeriod, label: &str, value: Option<f64>) -> Option<QuotaWindow> {
    value.map(|value| QuotaWindow {
        period,
        label: label.into(),
        used: None,
        used_raw: Some(value.max(0.0)),
        limit: None,
        remaining: None,
        unit: QuotaUnit::Currency("USD".into()),
        reset_at: None,
    })
}

fn key_windows(key: Option<&KeyData>) -> Vec<QuotaWindow> {
    let Some(key) = key else { return vec![] };
    let mut windows = Vec::new();
    if let Some(quota) = key_quota(key) {
        windows.push(quota);
    }
    windows.extend(
        [
            spend_window(WindowPeriod::Day, "今日花费", key.usage_daily),
            spend_window(WindowPeriod::Week, "本周花费", key.usage_weekly),
            spend_window(WindowPeriod::Month, "本月花费", key.usage_monthly),
        ]
        .into_iter()
        .flatten(),
    );
    windows
}

fn build_snapshot(
    credits: Option<CreditsData>,
    key: Option<KeyData>,
    credits_error: Option<String>,
    key_error: Option<String>,
) -> anyhow::Result<ProviderSnapshot> {
    if credits.is_none() && key.is_none() {
        let detail = [credits_error, key_error]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("；");
        anyhow::bail!("OpenRouter Credits 与当前 Key 信息均不可用：{detail}");
    }

    let balance_value = credits
        .as_ref()
        .map(|credits| (credits.total_credits - credits.total_usage).max(0.0));
    let balance = balance_value.map(|balance| Balance {
        total: balance,
        granted: None,
        topped_up: None,
        currency: "USD".into(),
        available: balance > 0.0,
    });
    let account_label = key
        .as_ref()
        .and_then(|key| clean_label(key.label.as_deref()))
        .or_else(|| {
            key.as_ref()
                .and_then(|key| clean_label(key.creator_user_id.as_deref()))
        });
    let key_exhausted = key
        .as_ref()
        .and_then(key_quota)
        .and_then(|window| window.used)
        .is_some_and(|used| used >= 100.0);
    let exact = credits.is_some() && key.is_some();
    let status = if balance_value.is_some_and(|balance| balance <= 0.0) || key_exhausted {
        HealthStatus::Exhausted
    } else if exact {
        HealthStatus::Ok
    } else {
        HealthStatus::Degraded
    };
    let last_error = match (credits.is_some(), key.is_some()) {
        (false, true) => credits_error.or_else(|| Some("OpenRouter Credits 余额暂不可用".into())),
        (true, false) => key_error.or_else(|| Some("OpenRouter 当前 Key 用量暂不可用".into())),
        _ => None,
    };

    Ok(ProviderSnapshot {
        account_id: String::new(),
        account_label,
        provider_id: "openrouter".into(),
        display_name: "OpenRouter".into(),
        plan_name: None,
        billing: BillingMode::PayAsYouGo,
        balance,
        windows: key_windows(key.as_ref()),
        fidelity: if exact {
            Fidelity::Exact
        } else {
            Fidelity::Partial
        },
        status,
        fetched_at: Utc::now().timestamp(),
        last_error,
    })
}

#[async_trait]
impl Provider for OpenRouterProvider {
    fn id(&self) -> &'static str {
        "openrouter"
    }
    fn display_name(&self) -> &'static str {
        "OpenRouter"
    }
    fn brand(&self) -> Brand {
        Brand::OpenRouter
    }
    fn billing_mode(&self) -> BillingMode {
        BillingMode::PayAsYouGo
    }
    fn add_product_name(&self) -> &'static str {
        "OpenRouter API"
    }
    fn add_description(&self) -> &'static str {
        "Credits 余额或 Key 用量"
    }
    fn detail_url(&self) -> Option<&'static str> {
        Some("https://openrouter.ai/settings/credits")
    }
    fn card_config(&self) -> presentation::CardConfig {
        let mut config = presentation::CardConfig::for_billing(self.billing_mode());
        config.detail_url = self.detail_url();
        // credits.total_credits 是累计充值，不是当前余额的独立组成项，避免重复展示。
        config.balance_role = presentation::BalanceRole::Primary {
            topped_up_label: "充值余额",
            granted_label: "赠送余额",
        };
        config
    }
    fn auth_spec(&self) -> AuthSpec {
        AuthSpec::Hybrid {
            primary: Box::new(AuthSpec::OAuth {
                authorize_url: "https://openrouter.ai/auth",
                token_url: "https://openrouter.ai/api/v1/auth/keys",
                client_id: "",
                scopes: &[],
                pkce: true,
            }),
            fallback: Box::new(AuthSpec::ApiKey {
                fields: vec![AuthField {
                    key: "api_key",
                    label: "API Key",
                    placeholder: "sk-or-v1-...",
                    secret: true,
                    required: true,
                    options: None,
                }],
                hint: "普通/OAuth Key 可显示自身预算和花费；账户 Credits 余额仅 Management Key 有权读取。",
            }),
        }
    }

    async fn detect_local(&self) -> Option<Credential> {
        let api_key = std::env::var("OPENROUTER_API_KEY").ok()?;
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return None;
        }
        Some(Credential {
            data: serde_json::json!({
                "api_key": api_key,
                "source_kind": "environment",
            }),
        })
    }

    async fn fetch(&self, cred: &Credential) -> anyhow::Result<ProviderSnapshot> {
        let api_key = cred
            .data
            .get("api_key")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("缺少 api_key"))?;
        let client = super::http_client();
        let credits_request = client
            .get(format!("{BASE_URL}/credits"))
            .bearer_auth(api_key)
            .header("Accept", "application/json")
            .header("X-Title", "TokenMeter")
            .send();
        let key_request = client
            .get(format!("{BASE_URL}/key"))
            .bearer_auth(api_key)
            .header("Accept", "application/json")
            .header("X-Title", "TokenMeter")
            .send();
        let (credits_response, key_response) = tokio::join!(credits_request, key_request);

        let (credits, credits_error) = match credits_response {
            Ok(response) if response.status().is_success() => {
                match response.json::<CreditsResponse>().await {
                    Ok(value) => (Some(value.data), None),
                    Err(error) => (None, Some(format!("OpenRouter Credits 响应解析失败：{error}"))),
                }
            }
            Ok(response) if matches!(response.status().as_u16(), 401 | 403) => (
                None,
                Some(
                    "当前 OpenRouter Key 无权读取 Credits 余额；仅显示 Key 用量，账户余额需 Management Key"
                        .into(),
                ),
            ),
            Ok(response) => (
                None,
                Some(format!(
                    "OpenRouter Credits 余额暂不可用（HTTP {}）",
                    response.status().as_u16()
                )),
            ),
            Err(error) => (
                None,
                Some(format!("OpenRouter Credits 请求失败：{error}")),
            ),
        };
        let (key, key_error) = match key_response {
            Ok(response) if response.status().is_success() => {
                match response.json::<KeyResponse>().await {
                    Ok(value) => (Some(value.data), None),
                    Err(error) => (None, Some(format!("OpenRouter Key 响应解析失败：{error}"))),
                }
            }
            Ok(response) => (
                None,
                Some(format!(
                    "OpenRouter Key 用量暂不可用（HTTP {}）",
                    response.status().as_u16()
                )),
            ),
            Err(error) => (None, Some(format!("OpenRouter Key 请求失败：{error}"))),
        };
        build_snapshot(credits, key, credits_error, key_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_budget_prefers_server_remaining_and_keeps_spend_rows() {
        let key = KeyData {
            label: Some("Personal".into()),
            creator_user_id: None,
            limit: Some(100.0),
            limit_remaining: Some(75.0),
            limit_reset: Some("monthly".into()),
            usage: Some(40.0),
            usage_daily: Some(1.0),
            usage_weekly: Some(5.0),
            usage_monthly: Some(25.0),
        };
        let windows = key_windows(Some(&key));
        assert_eq!(windows.len(), 4);
        assert_eq!(windows[0].used_raw, Some(25.0));
        assert_eq!(windows[0].remaining, Some(75.0));
        assert_eq!(windows[0].used, Some(25.0));
    }

    #[test]
    fn credits_balance_is_total_added_less_total_usage() {
        let credits = CreditsData {
            total_credits: 50.0,
            total_usage: 12.75,
        };
        assert_eq!(
            (credits.total_credits - credits.total_usage).max(0.0),
            37.25
        );
    }

    #[test]
    fn normal_key_without_management_credits_still_returns_partial_usage() {
        let key = KeyData {
            label: Some("OAuth key".into()),
            creator_user_id: None,
            limit: Some(20.0),
            limit_remaining: Some(15.0),
            limit_reset: Some("monthly".into()),
            usage: Some(5.0),
            usage_daily: Some(1.0),
            usage_weekly: Some(3.0),
            usage_monthly: Some(5.0),
        };
        let snapshot = build_snapshot(
            None,
            Some(key),
            Some(
                "当前 OpenRouter Key 无权读取 Credits 余额；仅显示 Key 用量，账户余额需 Management Key"
                    .into(),
            ),
            None,
        )
        .unwrap();
        assert!(snapshot.balance.is_none());
        assert_eq!(snapshot.fidelity, Fidelity::Partial);
        assert_eq!(snapshot.status, HealthStatus::Degraded);
        assert_eq!(snapshot.windows[0].remaining, Some(15.0));
        assert!(snapshot
            .last_error
            .as_deref()
            .unwrap()
            .contains("Management Key"));
    }

    #[test]
    fn credits_and_key_together_produce_an_exact_snapshot() {
        let credits = CreditsData {
            total_credits: 50.0,
            total_usage: 12.75,
        };
        let key = KeyData {
            label: None,
            creator_user_id: Some("user-1".into()),
            limit: None,
            limit_remaining: None,
            limit_reset: None,
            usage: None,
            usage_daily: Some(1.0),
            usage_weekly: None,
            usage_monthly: None,
        };
        let snapshot = build_snapshot(Some(credits), Some(key), None, None).unwrap();
        assert_eq!(
            snapshot.balance.as_ref().map(|value| value.total),
            Some(37.25)
        );
        assert_eq!(snapshot.fidelity, Fidelity::Exact);
        assert_eq!(snapshot.status, HealthStatus::Ok);
        assert_eq!(snapshot.account_label.as_deref(), Some("user-1"));
    }
}
