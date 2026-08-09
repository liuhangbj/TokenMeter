//! GLM Coding Plan（Z.ai / 智谱国内站）。
//!
//! 套餐使用 API Key 认证，但计费语义是订阅额度而非按量 API。额度接口返回
//! 5 小时、周窗口与工具调用窗口；订阅列表仅用于补充套餐名，失败不影响额度。

use super::*;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

const GLOBAL_BASE: &str = "https://api.z.ai";
const CN_BASE: &str = "https://open.bigmodel.cn";

pub struct GlmCodingPlanProvider;

impl GlmCodingPlanProvider {
    pub fn new() -> Self {
        Self
    }
}

fn number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|value| value.is_finite())
}

fn text(value: Option<&Value>) -> Option<String> {
    value?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn data_object(root: &Value) -> &Value {
    root.get("data")
        .filter(|value| value.is_object())
        .unwrap_or(root)
}

fn normalize_plan_name(raw: &str) -> String {
    let lower = raw.trim().to_lowercase();
    if lower.contains("max") {
        "Max".into()
    } else if lower.contains("pro") {
        "Pro".into()
    } else if lower.contains("standard") {
        "Standard".into()
    } else if lower.contains("lite") {
        "Lite".into()
    } else if lower.contains("free") {
        "Free".into()
    } else {
        raw.trim().to_string()
    }
}

fn plan_from_quota(root: &Value) -> Option<String> {
    let data = data_object(root);
    ["planName", "plan", "plan_type", "packageName", "level"]
        .iter()
        .find_map(|key| text(data.get(*key)))
        .map(|name| normalize_plan_name(&name))
}

fn plan_from_subscription(root: &Value) -> Option<String> {
    let first = root
        .get("data")
        .and_then(Value::as_array)
        .and_then(|items| items.first())?;
    ["productName", "planName", "packageName", "name"]
        .iter()
        .find_map(|key| text(first.get(*key)))
        .map(|name| normalize_plan_name(&name))
}

fn account_label(quota: &Value, subscription: Option<&Value>) -> Option<String> {
    let keys = [
        "username",
        "userName",
        "display_name",
        "displayName",
        "email",
        "user_id",
        "userId",
        "account_id",
    ];
    let quota_data = data_object(quota);
    keys.iter()
        .find_map(|key| text(quota_data.get(*key)))
        .or_else(|| {
            subscription
                .and_then(|root| root.get("data"))
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| keys.iter().find_map(|key| text(item.get(*key))))
        })
}

fn epoch_seconds(value: Option<&Value>) -> Option<i64> {
    let raw = number(value)?;
    if raw <= 0.0 {
        return None;
    }
    Some(if raw >= 10_000_000_000.0 {
        (raw / 1000.0).round() as i64
    } else {
        raw.round() as i64
    })
}

fn duration_minutes(unit: i64, number: f64) -> Option<i64> {
    let unit_minutes = match unit {
        1 | 4 => 24 * 60, // 历史响应曾用 1/4 表示 day
        3 => 60,
        5 => 30 * 24 * 60,
        6 => 7 * 24 * 60,
        _ => return None,
    };
    let result = number * unit_minutes as f64;
    (result.is_finite() && result > 0.0).then_some(result.round() as i64)
}

fn period_for(minutes: i64) -> WindowPeriod {
    if (minutes - 5 * 60).abs() <= 15 {
        WindowPeriod::Hours5
    } else if (minutes - 7 * 24 * 60).abs() <= 60 {
        WindowPeriod::Week
    } else if (minutes - 24 * 60).abs() <= 15 {
        WindowPeriod::Day
    } else if (28 * 24 * 60..=31 * 24 * 60).contains(&minutes) {
        WindowPeriod::Month
    } else {
        WindowPeriod::Custom(minutes)
    }
}

fn period_label(period: WindowPeriod, minutes: i64) -> String {
    match period {
        WindowPeriod::Hours5 => "5 小时额度".into(),
        WindowPeriod::Week => "7 天额度".into(),
        WindowPeriod::Day => "每日额度".into(),
        WindowPeriod::Month => "每月额度".into(),
        WindowPeriod::Custom(_) if minutes % (24 * 60) == 0 => {
            format!("{} 天额度", minutes / (24 * 60))
        }
        WindowPeriod::Custom(_) if minutes % 60 == 0 => {
            format!("{} 小时额度", minutes / 60)
        }
        WindowPeriod::Custom(_) => "周期额度".into(),
    }
}

fn quota_window(entry: &Value) -> Option<QuotaWindow> {
    let kind = text(entry.get("type").or_else(|| entry.get("name")))?.to_uppercase();
    let unit_code = number(entry.get("unit"))?.round() as i64;
    let count = number(entry.get("number"))?;
    let minutes = duration_minutes(unit_code, count)?;
    let total = number(entry.get("usage"));
    let current = number(entry.get("currentValue"));
    let remaining = number(entry.get("remaining")).or_else(|| match (total, current) {
        (Some(total), Some(used)) => Some((total - used).max(0.0)),
        _ => None,
    });
    let used_percent = number(entry.get("percentage"))
        .or_else(|| match (current, total) {
            (Some(used), Some(limit)) if limit > 0.0 => Some(used / limit * 100.0),
            _ => None,
        })
        .map(|value| value.clamp(0.0, 100.0));
    let current = current.or_else(|| match (total, used_percent) {
        (Some(total), Some(percent)) => Some(total * percent / 100.0),
        _ => None,
    });
    let remaining = remaining.or_else(|| match (total, used_percent) {
        (Some(total), Some(percent)) => Some(total * (1.0 - percent / 100.0)),
        (None, Some(percent)) => Some(100.0 - percent),
        _ => None,
    });

    match kind.as_str() {
        "TOKENS_LIMIT" | "CREDIT_LIMIT" => {
            let period = period_for(minutes);
            Some(QuotaWindow {
                period,
                label: period_label(period, minutes),
                used: used_percent,
                used_raw: current,
                limit: total.or(used_percent.is_some().then_some(100.0)),
                remaining,
                unit: if total.is_some() {
                    if kind == "TOKENS_LIMIT" {
                        QuotaUnit::Tokens
                    } else {
                        QuotaUnit::Requests
                    }
                } else {
                    QuotaUnit::Percent
                },
                reset_at: epoch_seconds(entry.get("nextResetTime")),
            })
        }
        "TIME_LIMIT" | "TIMES_LIMIT" => Some(QuotaWindow {
            // 工具额度通常按月，但不应抢占周 Token 额度的主值位置。
            period: WindowPeriod::Custom(minutes),
            label: "MCP / Web 额度".into(),
            used: used_percent,
            used_raw: current,
            limit: total,
            remaining,
            unit: QuotaUnit::Requests,
            reset_at: epoch_seconds(entry.get("nextResetTime")),
        }),
        _ => None,
    }
}

fn windows_from_quota(root: &Value) -> Vec<QuotaWindow> {
    let data = data_object(root);
    let mut windows = data
        .get("limits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(quota_window)
        .collect::<Vec<_>>();
    windows.sort_by_key(|window| match window.period {
        WindowPeriod::Hours5 => 0,
        WindowPeriod::Week => 1,
        WindowPeriod::Day => 2,
        WindowPeriod::Month | WindowPeriod::Custom(_) => 3,
    });
    windows
}

fn api_key_credential(api_key: &str, region: &str, source: &str) -> Credential {
    Credential {
        data: json!({
            "api_key": api_key,
            "region": region,
            "source_kind": source,
        }),
    }
}

fn read_plain_key(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn zcode_credential() -> Option<Credential> {
    let path = super::home_dir().join(".zcode/v2/config.json");
    let root = serde_json::from_str::<Value>(&fs::read_to_string(path).ok()?).ok()?;
    let providers = root.get("provider")?.as_object()?;
    for (provider_id, provider) in providers {
        if provider.get("enabled").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        let Some(options) = provider.get("options") else {
            continue;
        };
        let base = options
            .get("baseURL")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();
        let identity = provider_id.to_lowercase();
        if !base.contains("z.ai")
            && !base.contains("bigmodel.cn")
            && !identity.contains("zai")
            && !identity.contains("zhipu")
            && !identity.contains("glm")
        {
            continue;
        }
        let Some(key) = options
            .get("apiKey")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let region = if base.contains("bigmodel.cn") {
            "cn"
        } else {
            "global"
        };
        return Some(api_key_credential(key, region, "zcode"));
    }
    None
}

#[async_trait]
impl Provider for GlmCodingPlanProvider {
    fn id(&self) -> &'static str {
        "glm_coding_plan"
    }

    fn display_name(&self) -> &'static str {
        "GLM Coding Plan"
    }

    fn brand(&self) -> Brand {
        Brand::Glm
    }

    fn billing_mode(&self) -> BillingMode {
        BillingMode::Subscription
    }

    fn add_product_name(&self) -> &'static str {
        "GLM Coding Plan"
    }

    fn add_description(&self) -> &'static str {
        "5 小时与周额度"
    }

    fn detail_url(&self) -> Option<&'static str> {
        Some("https://z.ai/manage-apikey/coding-plan/personal/my-plan")
    }

    fn card_config(&self) -> presentation::CardConfig {
        let mut config = presentation::CardConfig::for_billing(self.billing_mode());
        config.detail_url = self.detail_url();
        config.primary_window = presentation::PrimaryWindowSelection::HighestNonCurrency;
        config
    }

    fn plan_tier(&self, plan_name: &str) -> Option<u8> {
        let name = plan_name.to_lowercase();
        if name.contains("free") {
            Some(0)
        } else if name.contains("lite") {
            Some(2)
        } else if name.contains("standard") {
            Some(3)
        } else if name.contains("pro") {
            Some(4)
        } else if name.contains("max") {
            Some(5)
        } else {
            None
        }
    }

    fn auth_spec(&self) -> AuthSpec {
        AuthSpec::ApiKey {
            fields: vec![
                AuthField {
                    key: "api_key",
                    label: "Coding Plan API Key",
                    placeholder: "请输入 GLM Coding Plan Key",
                    secret: true,
                    required: true,
                    options: None,
                },
                AuthField {
                    key: "region",
                    label: "站点区域",
                    placeholder: "",
                    secret: false,
                    required: true,
                    options: Some(vec![("global", "国际站 (z.ai)"), ("cn", "国内站 (bigmodel.cn)")]),
                },
            ],
            hint: "请填写 GLM Coding Plan 使用的 API Key；普通按量账户若没有 Coding Plan 额度将无法添加。",
        }
    }

    fn supports_local_import(&self) -> bool {
        true
    }

    async fn detect_local(&self) -> Option<Credential> {
        for name in ["Z_AI_API_KEY", "ZAI_API_KEY"] {
            if let Ok(value) = std::env::var(name) {
                let key = value.trim();
                if !key.is_empty() {
                    return Some(api_key_credential(key, "global", "environment"));
                }
            }
        }
        for name in [
            "BIGMODEL_API_KEY",
            "ZHIPU_API_KEY",
            "ZHIPUAI_API_KEY",
            "GLM_API_KEY",
        ] {
            if let Ok(value) = std::env::var(name) {
                let key = value.trim();
                if !key.is_empty() {
                    return Some(api_key_credential(key, "cn", "environment"));
                }
            }
        }
        if let Some(credential) = zcode_credential() {
            return Some(credential);
        }
        for relative in [
            ".coding-relay/glm-api-key",
            ".config/bigmodel/api_key",
            ".config/zhipu/api_key",
        ] {
            if let Some(key) = read_plain_key(&super::home_dir().join(relative)) {
                return Some(api_key_credential(&key, "cn", "file"));
            }
        }
        None
    }

    async fn fetch(&self, cred: &Credential) -> anyhow::Result<ProviderSnapshot> {
        let api_key = cred
            .data
            .get("api_key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("缺少 api_key"))?;
        let region = cred
            .data
            .get("region")
            .and_then(Value::as_str)
            .unwrap_or("global");
        let base = if region == "cn" { CN_BASE } else { GLOBAL_BASE };
        let client = super::http_client();
        let quota_request = client
            .get(format!("{base}/api/monitor/usage/quota/limit"))
            .bearer_auth(api_key)
            .header("Accept", "application/json")
            .send();
        let subscription_request = client
            .get(format!("{base}/api/biz/subscription/list"))
            .bearer_auth(api_key)
            .header("Accept", "application/json")
            .send();
        let (quota_response, subscription_response) =
            tokio::join!(quota_request, subscription_request);

        let quota_response = quota_response?.error_for_status()?;
        let quota = quota_response.json::<Value>().await?;
        if quota.get("success").and_then(Value::as_bool) == Some(false) {
            let message = text(quota.get("msg")).unwrap_or_else(|| "未知错误".into());
            anyhow::bail!("GLM Coding Plan 返回失败：{message}");
        }
        let windows = windows_from_quota(&quota);
        if windows.is_empty() {
            anyhow::bail!("GLM Coding Plan 未返回可用额度窗口");
        }
        let subscription = match subscription_response {
            Ok(response) if response.status().is_success() => response.json::<Value>().await.ok(),
            _ => None,
        };
        let plan_name = subscription
            .as_ref()
            .and_then(plan_from_subscription)
            .or_else(|| plan_from_quota(&quota));
        let main_used = windows
            .iter()
            .filter(|window| matches!(window.unit, QuotaUnit::Tokens | QuotaUnit::Percent))
            .max_by_key(|window| match window.period {
                WindowPeriod::Week => 3,
                WindowPeriod::Hours5 => 2,
                _ => 1,
            })
            .and_then(|window| window.used);

        Ok(ProviderSnapshot {
            account_id: String::new(),
            account_label: account_label(&quota, subscription.as_ref()),
            provider_id: self.id().into(),
            display_name: self.display_name().into(),
            plan_name,
            billing: BillingMode::Subscription,
            balance: None,
            windows,
            fidelity: Fidelity::Estimated,
            status: if main_used.is_some_and(|value| value >= 100.0) {
                HealthStatus::Exhausted
            } else {
                HealthStatus::Ok
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
    fn maps_token_windows_and_keeps_monthly_tools_out_of_primary_priority() {
        let payload = json!({
            "success": true,
            "data": {
                "level": "pro",
                "limits": [
                    {"type":"TOKENS_LIMIT","unit":3,"number":5,"usage":800000000,"currentValue":127694464,"remaining":672305536,"percentage":15,"nextResetTime":1770648402389_i64},
                    {"type":"TOKENS_LIMIT","unit":6,"number":1,"usage":4000000000_i64,"currentValue":1000000000_i64,"remaining":3000000000_i64,"percentage":25},
                    {"type":"TIME_LIMIT","unit":5,"number":1,"usage":4000,"currentValue":1828,"remaining":2172,"percentage":45}
                ]
            }
        });
        let windows = windows_from_quota(&payload);
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].period, WindowPeriod::Hours5);
        assert_eq!(windows[1].period, WindowPeriod::Week);
        assert_eq!(windows[1].used_raw, Some(1_000_000_000.0));
        assert!(matches!(windows[2].period, WindowPeriod::Custom(_)));
        assert_eq!(plan_from_quota(&payload).as_deref(), Some("Pro"));
    }

    #[test]
    fn parses_subscription_product_name() {
        let payload = json!({"data":[{"productName":"GLM Coding Max"}]});
        assert_eq!(plan_from_subscription(&payload).as_deref(), Some("Max"));
    }
}
