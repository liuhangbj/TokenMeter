//! Moonshot API（按量侧，官方路径）
//!
//! 端点：`GET https://api.moonshot.cn/v1/users/me/balance`（国内站 CNY）
//!       `GET https://api.moonshot.ai/v1/users/me/balance`（国际站 USD）
//! 说明：官方仅提供余额，无用量接口。本 App 已决定不做日/周/月消耗统计
//!       （用户 2026-08-02 拍板，越轻越好），故只实时显示当前余额。

use super::*;
use crate::providers::Brand;
use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;

pub struct MoonshotProvider;

impl MoonshotProvider {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Deserialize)]
struct BalanceResp {
    code: i64,
    data: BalanceData,
    status: bool,
}

#[derive(Deserialize)]
struct BalanceData {
    available_balance: f64,
    voucher_balance: f64,
    cash_balance: f64,
}

#[async_trait]
impl Provider for MoonshotProvider {
    fn id(&self) -> &'static str {
        "moonshot"
    }
    fn display_name(&self) -> &'static str {
        "Moonshot API"
    }
    fn brand(&self) -> Brand {
        Brand::Moonshot
    }
    fn billing_mode(&self) -> BillingMode {
        BillingMode::PayAsYouGo
    }
    fn auth_spec(&self) -> AuthSpec {
        AuthSpec::ApiKey {
            fields: vec![
                AuthField {
                    key: "api_key",
                    label: "API Key",
                    placeholder: "sk-...",
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
                        ("cn", "国内站 (moonshot.cn, CNY)"),
                        ("intl", "国际站 (moonshot.ai, USD)"),
                    ]),
                },
            ],
            hint: "在 platform.moonshot.cn 或 platform.moonshot.ai 的 API 页面创建 Key。国内站与国际站 Key 不通用。",
        }
    }
    async fn fetch(&self, cred: &Credential) -> anyhow::Result<ProviderSnapshot> {
        let api_key = cred
            .data
            .get("api_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 api_key"))?;
        let region = cred.data.get("region").and_then(|v| v.as_str()).unwrap_or("cn");
        let (base, currency) = if region == "intl" {
            ("https://api.moonshot.ai", "USD")
        } else {
            ("https://api.moonshot.cn", "CNY")
        };

        let client = super::http_client();
        let resp = client
            .get(format!("{base}/v1/users/me/balance"))
            .bearer_auth(api_key)
            .send()
            .await?
            .error_for_status()?
            .json::<BalanceResp>()
            .await?;

        if !resp.status {
            anyhow::bail!("Moonshot 返回失败：code={}", resp.code);
        }

        Ok(ProviderSnapshot {
            provider_id: self.id().to_string(),
            display_name: self.display_name().to_string(),
            plan_name: None,
            billing: BillingMode::PayAsYouGo,
            balance: Some(Balance {
                total: resp.data.available_balance,
                granted: Some(resp.data.voucher_balance),
                topped_up: Some(resp.data.cash_balance),
                currency: currency.to_string(),
                available: resp.data.available_balance > 0.0,
            }),
            windows: vec![], // 无用量接口；不做消耗统计，仅实时余额
            fidelity: Fidelity::Exact,
            status: HealthStatus::Ok,
            fetched_at: Utc::now().timestamp(),
            last_error: None,
        })
    }
}
