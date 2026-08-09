//! MiniMax 普通按量 API（国际站 / 国内站）。
//!
//! 官方 CLI 会把 `sk-api-*` Key 路由到 `/account/query_balance`；该接口返回
//! 可用、现金、代金券、Credit 与欠款余额，和 Token Plan 的套餐余量相互独立。

use super::*;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};
use std::fs;

const GLOBAL_BASE: &str = "https://api.minimax.io";
const CN_BASE: &str = "https://api.minimaxi.com";

pub struct MiniMaxApiProvider;

impl MiniMaxApiProvider {
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

fn validate_body(root: &Value) -> anyhow::Result<()> {
    let data = data_object(root);
    let base_resp = data
        .get("base_resp")
        .or_else(|| data.get("baseResp"))
        .or_else(|| root.get("base_resp"));
    if let Some(base_resp) = base_resp {
        let code = number(
            base_resp
                .get("status_code")
                .or_else(|| base_resp.get("statusCode")),
        )
        .unwrap_or(0.0)
        .round() as i64;
        if code != 0 {
            let message = text(
                base_resp
                    .get("status_msg")
                    .or_else(|| base_resp.get("statusMsg")),
            )
            .unwrap_or_else(|| "未知错误".into());
            anyhow::bail!("MiniMax API 返回失败：{message} ({code})");
        }
    }
    Ok(())
}

fn amount(root: &Value, key: &str) -> Option<f64> {
    number(data_object(root).get(key))
}

fn amount_window(label: &str, value: Option<f64>, currency: &str) -> Option<QuotaWindow> {
    value.map(|value| QuotaWindow {
        period: WindowPeriod::Custom(0),
        label: label.into(),
        used: None,
        used_raw: Some(value.max(0.0)),
        limit: None,
        remaining: None,
        unit: QuotaUnit::Currency(currency.into()),
        reset_at: None,
    })
}

fn balance_windows(root: &Value, currency: &str) -> Vec<QuotaWindow> {
    let mut windows = [
        amount_window("现金余额", amount(root, "cash_balance"), currency),
        amount_window("代金券余额", amount(root, "voucher_balance"), currency),
        amount_window("Credit 余额", amount(root, "credit_balance"), currency),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    if amount(root, "owed_amount").is_some_and(|value| value > 0.0) {
        windows.extend(amount_window("欠款", amount(root, "owed_amount"), currency));
    }
    windows
}

fn account_label(root: &Value) -> Option<String> {
    let data = data_object(root);
    ["username", "user_name", "email", "account_id", "group_id"]
        .iter()
        .find_map(|key| text(data.get(*key)))
}

fn normalize_region(value: Option<&str>) -> &'static str {
    match value.unwrap_or("").trim().to_lowercase().as_str() {
        "cn" | "china" | "domestic" => "cn",
        _ => "global",
    }
}

fn is_payg_key(value: &str) -> bool {
    value.trim().starts_with("sk-api-")
}

fn credential(api_key: &str, region: &str, source: &str) -> Credential {
    Credential {
        data: json!({
            "api_key": api_key,
            "region": normalize_region(Some(region)),
            "source_kind": source,
        }),
    }
}

fn read_mmx_config() -> Option<Credential> {
    let path = super::home_dir().join(".mmx/config.json");
    let root = serde_json::from_str::<Value>(&fs::read_to_string(path).ok()?).ok()?;
    let api_key = root
        .get("api_key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| is_payg_key(value))?;
    let region = normalize_region(root.get("region").and_then(Value::as_str));
    Some(credential(api_key, region, "mmx_config"))
}

#[async_trait]
impl Provider for MiniMaxApiProvider {
    fn id(&self) -> &'static str {
        "minimax_api"
    }

    fn display_name(&self) -> &'static str {
        "MiniMax API"
    }

    fn brand(&self) -> Brand {
        Brand::MiniMax
    }

    fn billing_mode(&self) -> BillingMode {
        BillingMode::PayAsYouGo
    }

    fn add_product_name(&self) -> &'static str {
        "MiniMax API"
    }

    fn add_description(&self) -> &'static str {
        "按量余额"
    }

    fn detail_url(&self) -> Option<&'static str> {
        Some("https://platform.minimax.io/user-center/payment/balance")
    }

    fn auth_spec(&self) -> AuthSpec {
        AuthSpec::ApiKey {
            fields: vec![
                AuthField {
                    key: "api_key",
                    label: "API Key",
                    placeholder: "sk-api-...",
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
                    options: Some(vec![
                        ("global", "国际站 (minimax.io)"),
                        ("cn", "国内站 (minimaxi.com)"),
                    ]),
                },
            ],
            hint: "请填写普通按量 API Key（sk-api- 前缀）；Subscription Key 请从同厂商的 Token Plan 入口添加。",
        }
    }

    fn supports_local_import(&self) -> bool {
        true
    }

    async fn detect_local(&self) -> Option<Credential> {
        for (name, region) in [
            ("MINIMAX_CN_API_KEY", "cn"),
            ("MINIMAX_API_KEY", "global"),
            ("MINIMAX_API_TOKEN", "global"),
        ] {
            if let Ok(value) = std::env::var(name) {
                let api_key = value.trim();
                if is_payg_key(api_key) {
                    return Some(credential(api_key, region, "environment"));
                }
            }
        }
        read_mmx_config()
    }

    async fn fetch(&self, cred: &Credential) -> anyhow::Result<ProviderSnapshot> {
        let api_key = cred
            .data
            .get("api_key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("缺少 api_key"))?;
        let region = normalize_region(cred.data.get("region").and_then(Value::as_str));
        let (base, currency) = if region == "cn" {
            (CN_BASE, "CNY")
        } else {
            (GLOBAL_BASE, "USD")
        };
        let response = super::http_client()
            .get(format!("{base}/account/query_balance"))
            .bearer_auth(api_key)
            .header("Accept", "application/json")
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        let root = serde_json::from_str::<Value>(&body)
            .map_err(|error| anyhow::anyhow!("MiniMax API 响应无法解析：{error}"))?;
        validate_body(&root)?;
        if !status.is_success() {
            anyhow::bail!("MiniMax API 请求失败：HTTP {}", status.as_u16());
        }
        let total = amount(&root, "available_amount")
            .ok_or_else(|| anyhow::anyhow!("MiniMax API 未返回可用余额"))?;

        Ok(ProviderSnapshot {
            account_id: String::new(),
            account_label: account_label(&root),
            provider_id: self.id().into(),
            display_name: self.display_name().into(),
            plan_name: None,
            billing: BillingMode::PayAsYouGo,
            balance: Some(Balance {
                total,
                granted: None,
                topped_up: None,
                currency: currency.into(),
                available: total > 0.0,
            }),
            windows: balance_windows(&root, currency),
            fidelity: Fidelity::Exact,
            status: if total > 0.0 {
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
    fn maps_official_balance_fields_without_merging_credit_types() {
        let root = json!({
            "available_amount":"98.00",
            "cash_balance":"0.00",
            "voucher_balance":"70.00",
            "credit_balance":"28.00",
            "owed_amount":"0.00",
            "base_resp":{"status_code":0,"status_msg":"success"}
        });
        validate_body(&root).unwrap();
        assert_eq!(amount(&root, "available_amount"), Some(98.0));
        let windows = balance_windows(&root, "USD");
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[1].label, "代金券余额");
        assert_eq!(windows[1].used_raw, Some(70.0));
        assert_eq!(windows[2].label, "Credit 余额");
    }

    #[test]
    fn body_auth_error_is_not_accepted_as_zero_balance() {
        let error = validate_body(&json!({
            "base_resp":{"status_code":1004,"status_msg":"not authorized"}
        }))
        .expect_err("认证错误不能作为正常余额保存");
        assert!(error.to_string().contains("not authorized"));
    }
}
