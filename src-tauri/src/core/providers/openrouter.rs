//! OpenRouter API（按量计费）。
//!
//! `/credits` 提供累计充值与累计消耗，主余额为两者之差；`/key` 作为可选
//! 增强，补充 API Key 预算和日/周/月花费。增强接口失败时仍保留可靠余额。

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
        "Credits 余额与 Key 用量"
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
                hint: "也可以在 openrouter.ai/settings/keys 手动创建 API Key；设置消费上限后可显示预算进度。",
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

        let credits = credits_response?
            .error_for_status()?
            .json::<CreditsResponse>()
            .await?
            .data;
        let key = match key_response {
            Ok(response) if response.status().is_success() => response
                .json::<KeyResponse>()
                .await
                .ok()
                .map(|value| value.data),
            _ => None,
        };
        let balance = (credits.total_credits - credits.total_usage).max(0.0);
        let account_label = key
            .as_ref()
            .and_then(|key| clean_label(key.label.as_deref()))
            .or_else(|| {
                key.as_ref()
                    .and_then(|key| clean_label(key.creator_user_id.as_deref()))
            });

        Ok(ProviderSnapshot {
            account_id: String::new(),
            account_label,
            provider_id: self.id().into(),
            display_name: self.display_name().into(),
            plan_name: None,
            billing: BillingMode::PayAsYouGo,
            balance: Some(Balance {
                total: balance,
                granted: None,
                topped_up: None,
                currency: "USD".into(),
                available: balance > 0.0,
            }),
            windows: key_windows(key.as_ref()),
            fidelity: if key.is_some() {
                Fidelity::Exact
            } else {
                Fidelity::Partial
            },
            status: if balance > 0.0 {
                HealthStatus::Ok
            } else {
                HealthStatus::Exhausted
            },
            fetched_at: Utc::now().timestamp(),
            last_error: None,
        })
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
}
