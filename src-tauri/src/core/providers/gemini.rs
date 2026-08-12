//! Gemini Code Assist / Gemini CLI 套餐额度。
//!
//! 普通 AI Studio API Key 不提供账户余额或套餐额度查询；本 Provider 对齐 Google
//! 官方 Gemini CLI，使用 OAuth + Code Assist `retrieveUserQuota` 返回模型配额桶。

use super::*;
use crate::core::oauth_google;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::fs;

const CODE_ASSIST_URL: &str = "https://cloudcode-pa.googleapis.com/v1internal";
const USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";

pub struct GeminiProvider;

impl GeminiProvider {
    pub fn new() -> Self {
        Self
    }
}

fn text(value: Option<&Value>) -> Option<String> {
    value?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|value| value.is_finite())
}

fn auth_expired(provider: &GeminiProvider) -> ProviderSnapshot {
    ProviderSnapshot {
        account_id: String::new(),
        account_label: None,
        provider_id: provider.id().into(),
        display_name: provider.display_name().into(),
        plan_name: None,
        billing: BillingMode::Subscription,
        balance: None,
        windows: vec![],
        fidelity: Fidelity::Partial,
        status: HealthStatus::AuthExpired,
        fetched_at: Utc::now().timestamp(),
        last_error: Some("Google OAuth 凭证已过期".into()),
    }
}

async fn post(access_token: &str, method: &str, body: &Value) -> anyhow::Result<Value> {
    let response = super::http_client()
        .post(format!("{CODE_ASSIST_URL}:{method}"))
        .bearer_auth(access_token)
        .header("Content-Type", "application/json")
        .json(body)
        .send()
        .await?;
    let status = response.status();
    let source = response.text().await?;
    if !status.is_success() {
        anyhow::bail!(
            "Gemini Code Assist HTTP {}：{}",
            status.as_u16(),
            source.trim()
        );
    }
    serde_json::from_str(&source).map_err(Into::into)
}

fn metadata(project: Option<&str>) -> Value {
    let mut value = json!({
        "ideType": "IDE_UNSPECIFIED",
        "platform": "PLATFORM_UNSPECIFIED",
        "pluginType": "GEMINI",
    });
    if let Some(project) = project {
        value["duetProject"] = json!(project);
    }
    value
}

async fn load_account(access_token: &str) -> anyhow::Result<Value> {
    post(
        access_token,
        "loadCodeAssist",
        &json!({
            "metadata": metadata(None),
            "mode": "FULL_ELIGIBILITY_CHECK",
        }),
    )
    .await
}

async fn account_context(
    access_token: &str,
) -> anyhow::Result<(String, Option<String>, Option<Value>)> {
    let mut account = load_account(access_token).await?;
    let has_current_tier = account
        .get("currentTier")
        .is_some_and(|value| value.is_object());
    if !has_current_tier {
        let tier = account
            .get("allowedTiers")
            .and_then(Value::as_array)
            .and_then(|tiers| {
                tiers
                    .iter()
                    .find(|tier| tier.get("isDefault").and_then(Value::as_bool) == Some(true))
                    .or_else(|| tiers.first())
            })
            .ok_or_else(|| anyhow::anyhow!("Gemini 账户没有可启用的套餐"))?;
        let tier_id =
            text(tier.get("id")).ok_or_else(|| anyhow::anyhow!("Gemini 默认套餐缺少 id"))?;
        let mut operation = post(
            access_token,
            "onboardUser",
            &json!({
                "tierId": tier_id,
                "metadata": metadata(None),
            }),
        )
        .await?;
        for _ in 0..24 {
            if operation.get("done").and_then(Value::as_bool) == Some(true) {
                break;
            }
            let name = text(operation.get("name"))
                .ok_or_else(|| anyhow::anyhow!("Gemini 启用操作缺少 name"))?;
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            let response = super::http_client()
                .get(format!("{CODE_ASSIST_URL}/{name}"))
                .bearer_auth(access_token)
                .send()
                .await?;
            let status = response.status();
            let source = response.text().await?;
            if !status.is_success() {
                anyhow::bail!(
                    "Gemini 启用状态 HTTP {}：{}",
                    status.as_u16(),
                    source.trim()
                );
            }
            operation = serde_json::from_str(&source)?;
        }
        if operation.get("done").and_then(Value::as_bool) != Some(true) {
            anyhow::bail!("Gemini 首次启用等待超时，请重新授权");
        }
        account = load_account(access_token).await?;
    }

    let project = text(account.get("cloudaicompanionProject"))
        .or_else(|| {
            account
                .get("response")
                .and_then(|value| value.get("cloudaicompanionProject"))
                .and_then(|value| text(value.get("id")))
        })
        .ok_or_else(|| anyhow::anyhow!("Gemini Code Assist 未返回托管项目 ID"))?;
    let paid = account
        .get("paidTier")
        .filter(|value| value.is_object())
        .cloned();
    let current = account.get("currentTier").filter(|value| value.is_object());
    let plan_name = paid
        .as_ref()
        .and_then(|tier| text(tier.get("name")).or_else(|| text(tier.get("id"))))
        .or_else(|| {
            current.and_then(|tier| text(tier.get("name")).or_else(|| text(tier.get("id"))))
        });
    Ok((project, plan_name, paid))
}

fn reset_at(value: Option<&Value>) -> Option<i64> {
    let text = value?.as_str()?;
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|value| value.timestamp())
}

fn period_for_reset(reset: Option<i64>, now: i64) -> WindowPeriod {
    let seconds = reset.map(|value| value.saturating_sub(now)).unwrap_or(0);
    if seconds >= 2 * 86_400 {
        WindowPeriod::Week
    } else if seconds >= 18 * 3_600 {
        WindowPeriod::Day
    } else if seconds >= 4 * 3_600 {
        WindowPeriod::Hours5
    } else {
        WindowPeriod::Custom(seconds.max(0))
    }
}

fn model_label(bucket: &Value) -> String {
    text(bucket.get("modelId"))
        .or_else(|| text(bucket.get("tokenType")))
        .unwrap_or_else(|| "Gemini 配额".into())
        .replace('-', " ")
}

fn quota_windows(root: &Value, now: i64) -> Vec<QuotaWindow> {
    root.get("buckets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|bucket| {
            let remaining_fraction = number(bucket.get("remainingFraction"))?;
            let remaining_fraction = remaining_fraction.clamp(0.0, 1.0);
            let remaining = number(bucket.get("remainingAmount"));
            let limit = match (remaining, remaining_fraction) {
                (Some(remaining), fraction) if fraction > 0.0 => Some(remaining / fraction),
                _ => Some(100.0),
            };
            let used_raw = match (remaining, limit) {
                (Some(remaining), Some(limit)) => Some((limit - remaining).max(0.0)),
                _ => Some((1.0 - remaining_fraction) * 100.0),
            };
            let unit = match text(bucket.get("tokenType"))
                .unwrap_or_default()
                .to_ascii_uppercase()
                .as_str()
            {
                value if value.contains("TOKEN") => QuotaUnit::Tokens,
                _ => QuotaUnit::Requests,
            };
            let reset = reset_at(bucket.get("resetTime"));
            Some(QuotaWindow {
                period: period_for_reset(reset, now),
                label: model_label(bucket),
                used: Some((1.0 - remaining_fraction) * 100.0),
                used_raw,
                limit,
                remaining,
                unit,
                reset_at: reset,
            })
        })
        .collect()
}

fn ai_credits(paid_tier: Option<&Value>) -> Option<f64> {
    paid_tier?
        .get("availableCredits")?
        .as_array()?
        .iter()
        .filter(|credit| {
            text(credit.get("creditType"))
                .map(|value| value == "GOOGLE_ONE_AI")
                .unwrap_or(false)
        })
        .filter_map(|credit| number(credit.get("creditAmount")))
        .reduce(|sum, value| sum + value)
}

fn local_account_label() -> Option<String> {
    let source = fs::read_to_string(super::home_dir().join(".gemini/google_accounts.json")).ok()?;
    let value = serde_json::from_str::<Value>(&source).ok()?;
    text(value.get("active"))
}

#[async_trait]
impl Provider for GeminiProvider {
    fn id(&self) -> &'static str {
        "gemini"
    }

    fn display_name(&self) -> &'static str {
        "Gemini"
    }

    fn brand(&self) -> Brand {
        Brand::Gemini
    }

    fn billing_mode(&self) -> BillingMode {
        BillingMode::Subscription
    }

    fn add_product_name(&self) -> &'static str {
        "Gemini Code Assist"
    }

    fn add_description(&self) -> &'static str {
        "模型额度与 AI Credits"
    }

    fn detail_url(&self) -> Option<&'static str> {
        Some("https://gemini.google.com/")
    }

    fn card_config(&self) -> presentation::CardConfig {
        let mut config = presentation::CardConfig::for_billing(self.billing_mode());
        config.detail_url = self.detail_url();
        config.balance_role = presentation::BalanceRole::Supplemental {
            label: "AI Credits",
        };
        // 模型桶通常共享重置周期；主值选择当前使用比例最高的一档，
        // 才能代表这个账号最先会触及的限制。
        config.primary_window = presentation::PrimaryWindowSelection::MostUsed;
        config
    }

    fn plan_tier(&self, plan_name: &str) -> Option<u8> {
        let name = plan_name.to_ascii_lowercase();
        if name.contains("free") {
            Some(0)
        } else if name.contains("ultra") {
            Some(5)
        } else if name.contains("pro") || name.contains("standard") || name.contains("advanced") {
            Some(2)
        } else {
            None
        }
    }

    fn auth_spec(&self) -> AuthSpec {
        AuthSpec::OAuth {
            authorize_url: "https://accounts.google.com/o/oauth2/v2/auth",
            token_url: oauth_google::TOKEN_URL,
            client_id: oauth_google::CLIENT_ID,
            scopes: &[
                "https://www.googleapis.com/auth/cloud-platform",
                "https://www.googleapis.com/auth/userinfo.email",
                "https://www.googleapis.com/auth/userinfo.profile",
            ],
            pkce: true,
        }
    }

    fn supports_local_import(&self) -> bool {
        true
    }

    async fn detect_local(&self) -> Option<Credential> {
        let path = super::home_dir().join(".gemini/oauth_creds.json");
        let source = fs::read_to_string(path).ok()?;
        let mut data = serde_json::from_str::<Value>(&source).ok()?;
        let object = data.as_object_mut()?;
        object.insert("source_kind".into(), json!("gemini_cli"));
        if let Some(label) = local_account_label() {
            object.insert("account_label".into(), json!(label));
        }
        Some(Credential { data })
    }

    async fn refresh(&self, cred: &Credential) -> anyhow::Result<Option<Credential>> {
        let refresh_token = cred
            .data
            .get("refresh_token")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty());
        let Some(refresh_token) = refresh_token else {
            return Ok(None);
        };
        let response = super::http_client()
            .post(oauth_google::TOKEN_URL)
            .form(&[
                ("client_id", oauth_google::CLIENT_ID),
                ("client_secret", oauth_google::CLIENT_SECRET),
                ("refresh_token", refresh_token),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await?;
        let status = response.status();
        let source = response.text().await?;
        if !status.is_success() {
            anyhow::bail!(
                "Gemini 凭证刷新失败 HTTP {}：{}",
                status.as_u16(),
                source.trim()
            );
        }
        let value = serde_json::from_str::<Value>(&source)?;
        let access_token = value
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Gemini 刷新响应缺少 access_token"))?;
        let expires_in = value
            .get("expires_in")
            .and_then(Value::as_i64)
            .unwrap_or(3600);
        let mut data = cred.data.clone();
        data["access_token"] = json!(access_token);
        data["expiry_date"] = json!(Utc::now().timestamp_millis() + expires_in * 1000);
        if let Some(scope) = value.get("scope").and_then(Value::as_str) {
            data["scope"] = json!(scope);
        }
        Ok(Some(Credential { data }))
    }

    async fn fetch(&self, cred: &Credential) -> anyhow::Result<ProviderSnapshot> {
        let access_token = cred
            .data
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("缺少 access_token"))?;
        let expiry = cred
            .data
            .get("expiry_date")
            .and_then(Value::as_i64)
            .or_else(|| cred.data.get("expiry").and_then(Value::as_i64));
        if expiry.is_some_and(|expiry| expiry <= Utc::now().timestamp_millis() + 60_000) {
            return Ok(auth_expired(self));
        }
        let (project, plan_name, paid_tier) = match account_context(access_token).await {
            Ok(value) => value,
            Err(error) if error.to_string().contains("HTTP 401") => return Ok(auth_expired(self)),
            Err(error) => return Err(error),
        };
        let quota = post(
            access_token,
            "retrieveUserQuota",
            &json!({ "project": project }),
        )
        .await?;
        let now = Utc::now().timestamp();
        let windows = quota_windows(&quota, now);
        if windows.is_empty() {
            anyhow::bail!("Gemini Code Assist 未返回可用配额桶");
        }
        let userinfo = super::http_client()
            .get(USERINFO_URL)
            .bearer_auth(access_token)
            .send()
            .await
            .ok();
        let account_label = match userinfo {
            Some(response) if response.status().is_success() => response
                .json::<Value>()
                .await
                .ok()
                .and_then(|value| text(value.get("name")).or_else(|| text(value.get("email")))),
            _ => None,
        }
        .or_else(|| text(cred.data.get("account_label")))
        .or_else(local_account_label);
        let credits = ai_credits(paid_tier.as_ref());
        let status = if windows
            .iter()
            .all(|window| window.used.unwrap_or(0.0) >= 100.0)
        {
            HealthStatus::Exhausted
        } else {
            HealthStatus::Ok
        };

        Ok(ProviderSnapshot {
            account_id: String::new(),
            account_label,
            provider_id: self.id().into(),
            display_name: self.display_name().into(),
            plan_name,
            billing: BillingMode::Subscription,
            balance: credits.map(|total| Balance {
                total,
                granted: None,
                topped_up: None,
                currency: "Credits".into(),
                available: total > 0.0,
            }),
            windows,
            fidelity: Fidelity::Exact,
            status,
            fetched_at: now,
            last_error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_remaining_fraction_to_used_quota() {
        let root = json!({ "buckets": [{
            "modelId": "gemini-2.5-pro",
            "tokenType": "REQUESTS",
            "remainingAmount": "75",
            "remainingFraction": 0.75,
            "resetTime": "2025-10-22T16:01:15Z"
        }]});
        let windows = quota_windows(&root, 1_761_100_000);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].used, Some(25.0));
        assert_eq!(windows[0].used_raw, Some(25.0));
        assert_eq!(windows[0].limit, Some(100.0));
        assert_eq!(windows[0].remaining, Some(75.0));
        assert_eq!(windows[0].unit, QuotaUnit::Requests);
    }

    #[test]
    fn extracts_google_one_ai_credits_only() {
        let tier = json!({ "availableCredits": [
            {"creditType":"GOOGLE_ONE_AI", "creditAmount":"120"},
            {"creditType":"OTHER", "creditAmount":"999"}
        ]});
        assert_eq!(ai_credits(Some(&tier)), Some(120.0));
    }

    #[test]
    fn maps_plan_price_bands() {
        let provider = GeminiProvider::new();
        assert_eq!(provider.plan_tier("free-tier"), Some(0));
        assert_eq!(provider.plan_tier("Google AI Pro"), Some(2));
        assert_eq!(provider.plan_tier("Google AI Ultra"), Some(5));
    }

    #[test]
    fn primary_uses_the_most_constrained_model_bucket() {
        let provider = GeminiProvider::new();
        let source = ProviderSnapshot {
            account_id: "gemini#1".into(),
            account_label: None,
            provider_id: provider.id().into(),
            display_name: provider.display_name().into(),
            plan_name: Some("Google AI Pro".into()),
            billing: BillingMode::Subscription,
            balance: None,
            windows: vec![
                QuotaWindow {
                    period: WindowPeriod::Hours5,
                    label: "Gemini Pro".into(),
                    used: Some(28.0),
                    used_raw: Some(28.0),
                    limit: Some(100.0),
                    remaining: Some(72.0),
                    unit: QuotaUnit::Requests,
                    reset_at: None,
                },
                QuotaWindow {
                    period: WindowPeriod::Hours5,
                    label: "Gemini Flash".into(),
                    used: Some(46.0),
                    used_raw: Some(46.0),
                    limit: Some(100.0),
                    remaining: Some(54.0),
                    unit: QuotaUnit::Requests,
                    reset_at: None,
                },
            ],
            fidelity: Fidelity::Exact,
            status: HealthStatus::Ok,
            fetched_at: 1,
            last_error: None,
        };
        let card = provider.present(&source);
        assert_eq!(card.primary.label, "Gemini Flash余量");
        assert_eq!(card.primary.value, Some(54.0));
        assert_eq!(card.primary.health_used_percent, Some(46.0));
        assert_eq!(card.items.len(), 2);
    }
}
