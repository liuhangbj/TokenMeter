//! 火山引擎费用中心账户余额。
//!
//! 使用官方 QueryBalanceAcct（2022-01-01）和火山 OpenAPI HMAC-SHA256 签名。
//! 方舟推理 API Key 无法查询账务数据，因此这里明确要求费用中心 AK/SK。

use super::*;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

const HOST: &str = "open.volcengineapi.com";
const REGION: &str = "cn-beijing";
const SERVICE: &str = "billing";
const VERSION: &str = "2022-01-01";
const ACTION: &str = "QueryBalanceAcct";

pub struct VolcengineProvider;

impl VolcengineProvider {
    pub fn new() -> Self {
        Self
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256(value: &[u8]) -> String {
    hex(&Sha256::digest(value))
}

fn hmac(key: &[u8], value: &str) -> anyhow::Result<Vec<u8>> {
    let mut mac = HmacSha256::new_from_slice(key)?;
    mac.update(value.as_bytes());
    Ok(mac.finalize().into_bytes().to_vec())
}

fn authorization(
    access_key: &str,
    secret_key: &str,
    security_token: Option<&str>,
    now: DateTime<Utc>,
    payload: &[u8],
) -> anyhow::Result<(String, String, String)> {
    let x_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let short_date = now.format("%Y%m%d").to_string();
    let payload_hash = sha256(payload);
    let signed_headers = if security_token.is_some() {
        "host;x-content-sha256;x-date;x-security-token"
    } else {
        "host;x-content-sha256;x-date"
    };
    let mut canonical_headers =
        format!("host:{HOST}\nx-content-sha256:{payload_hash}\nx-date:{x_date}\n");
    if let Some(token) = security_token {
        canonical_headers.push_str(&format!("x-security-token:{token}\n"));
    }
    let query = format!("Action={ACTION}&Version={VERSION}");
    let canonical_request =
        format!("POST\n/\n{query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}");
    let scope = format!("{short_date}/{REGION}/{SERVICE}/request");
    let string_to_sign = format!(
        "HMAC-SHA256\n{x_date}\n{scope}\n{}",
        sha256(canonical_request.as_bytes())
    );
    let date_key = hmac(secret_key.as_bytes(), &short_date)?;
    let region_key = hmac(&date_key, REGION)?;
    let service_key = hmac(&region_key, SERVICE)?;
    let signing_key = hmac(&service_key, "request")?;
    let signature = hex(&hmac(&signing_key, &string_to_sign)?);
    Ok((
        format!(
            "HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
        ),
        x_date,
        payload_hash,
    ))
}

fn number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|value| value.is_finite())
}

fn account_id(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
    .filter(|value| !value.is_empty())
}

fn balance_value(label: &str, value: Option<f64>) -> Option<QuotaWindow> {
    value.filter(|value| *value > 0.0).map(|value| QuotaWindow {
        period: WindowPeriod::Custom(0),
        label: label.into(),
        used: None,
        used_raw: Some(value),
        limit: None,
        remaining: None,
        unit: QuotaUnit::Currency("CNY".into()),
        reset_at: None,
    })
}

fn response_error(root: &Value) -> Option<String> {
    let error = root.get("ResponseMetadata")?.get("Error")?;
    let code = error
        .get("Code")
        .and_then(Value::as_str)
        .unwrap_or("Unknown");
    let message = error
        .get("Message")
        .and_then(Value::as_str)
        .unwrap_or("未知错误");
    Some(format!("{code}: {message}"))
}

#[async_trait]
impl Provider for VolcengineProvider {
    fn id(&self) -> &'static str {
        "volcengine"
    }

    fn display_name(&self) -> &'static str {
        "火山引擎"
    }

    fn brand(&self) -> Brand {
        Brand::Volcengine
    }

    fn billing_mode(&self) -> BillingMode {
        BillingMode::PayAsYouGo
    }

    fn add_product_name(&self) -> &'static str {
        "火山引擎 API"
    }

    fn add_description(&self) -> &'static str {
        "费用中心余额"
    }

    fn detail_url(&self) -> Option<&'static str> {
        Some("https://console.volcengine.com/finance/overview/")
    }

    fn card_config(&self) -> presentation::CardConfig {
        let mut config = presentation::CardConfig::for_billing(self.billing_mode());
        config.detail_url = self.detail_url();
        config.balance_role = presentation::BalanceRole::Primary {
            topped_up_label: "现金余额",
            granted_label: "信用额度",
        };
        config
    }

    fn auth_spec(&self) -> AuthSpec {
        AuthSpec::CloudSecret {
            fields: vec![
                AuthField {
                    key: "access_key",
                    label: "Access Key ID",
                    placeholder: "请输入费用中心 AK",
                    secret: false,
                    required: true,
                    options: None,
                },
                AuthField {
                    key: "secret_key",
                    label: "Secret Access Key",
                    placeholder: "",
                    secret: true,
                    required: true,
                    options: None,
                },
                AuthField {
                    key: "security_token",
                    label: "Security Token（可选）",
                    placeholder: "仅临时凭证需要填写",
                    secret: true,
                    required: false,
                    options: None,
                },
            ],
        }
    }

    async fn fetch(&self, cred: &Credential) -> anyhow::Result<ProviderSnapshot> {
        let access_key = cred
            .data
            .get("access_key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("缺少 access_key"))?;
        let secret_key = cred
            .data
            .get("secret_key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("缺少 secret_key"))?;
        let security_token = cred
            .data
            .get("security_token")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let payload = b"{}";
        let (auth, x_date, payload_hash) =
            authorization(access_key, secret_key, security_token, Utc::now(), payload)?;
        let url = format!("https://{HOST}/?Action={ACTION}&Version={VERSION}");
        let mut request = super::http_client()
            .post(url)
            .header("Host", HOST)
            .header("Content-Type", "application/json; charset=utf-8")
            .header("X-Date", x_date)
            .header("X-Content-Sha256", payload_hash)
            .header("Authorization", auth)
            .body(payload.to_vec());
        if let Some(token) = security_token {
            request = request.header("X-Security-Token", token);
        }
        let response = request.send().await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            anyhow::bail!("火山引擎 HTTP {}：{}", status.as_u16(), body.trim());
        }
        let root = serde_json::from_str::<Value>(&body)?;
        if let Some(error) = response_error(&root) {
            anyhow::bail!("火山引擎费用中心返回失败：{error}");
        }
        let result = root
            .get("Result")
            .filter(|value| value.is_object())
            .ok_or_else(|| anyhow::anyhow!("火山引擎费用中心未返回 Result"))?;
        let available_balance = number(result.get("AvailableBalance"))
            .ok_or_else(|| anyhow::anyhow!("火山引擎未返回 AvailableBalance"))?;
        let cash_balance = number(result.get("CashBalance"));
        let credit_limit = number(result.get("CreditLimit"));
        let freeze_amount = number(result.get("FreezeAmount"));
        let arrears_balance = number(result.get("ArrearsBalance"));
        let windows = [
            balance_value("冻结金额", freeze_amount),
            balance_value("欠费金额", arrears_balance),
        ]
        .into_iter()
        .flatten()
        .collect();
        let available = available_balance > 0.0;

        Ok(ProviderSnapshot {
            account_id: String::new(),
            account_label: account_id(result.get("AccountID")),
            provider_id: self.id().into(),
            display_name: self.display_name().into(),
            plan_name: None,
            billing: BillingMode::PayAsYouGo,
            balance: Some(Balance {
                total: available_balance,
                granted: credit_limit.filter(|value| *value > 0.0),
                topped_up: cash_balance,
                currency: "CNY".into(),
                available,
            }),
            windows,
            fidelity: Fidelity::Exact,
            status: if available {
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
    use chrono::TimeZone;

    #[test]
    fn signature_is_deterministic_and_scoped_to_billing() {
        let now = Utc.with_ymd_and_hms(2025, 3, 11, 6, 51, 37).unwrap();
        let (auth, x_date, payload_hash) =
            authorization("AKTEST", "SKTEST", None, now, b"{}").unwrap();
        assert_eq!(x_date, "20250311T065137Z");
        assert_eq!(payload_hash.len(), 64);
        assert!(auth.contains("AKTEST/20250311/cn-beijing/billing/request"));
        assert!(auth.contains("SignedHeaders=host;x-content-sha256;x-date"));
    }

    #[test]
    fn keeps_only_nonzero_account_adjustments() {
        assert!(balance_value("冻结金额", Some(0.0)).is_none());
        assert_eq!(
            balance_value("欠费金额", Some(2.5)).unwrap().used_raw,
            Some(2.5)
        );
    }
}
