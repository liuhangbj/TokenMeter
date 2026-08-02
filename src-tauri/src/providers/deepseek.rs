//! DeepSeek（按量侧，官方路径）
//!
//! 端点：`GET https://api.deepseek.com/user/balance`
//! 返回 balance_infos（total / granted / topped_up，字符串），需解析为 f64。
//! 说明：官方无用量接口。本 App 已决定不做日/周/月消耗统计（用户 2026-08-02），
//!       只实时显示当前余额（total/granted/topped_up 三段）。

use super::*;
use crate::providers::Brand;
use async_trait::async_trait;
use chrono::Utc;
use reqwest::Client;
use serde::Deserialize;

pub struct DeepSeekProvider;

impl DeepSeekProvider {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Deserialize)]
struct DsBalanceResp {
    is_available: bool,
    balance_infos: Vec<DsBalanceInfo>,
}

#[derive(Deserialize)]
struct DsBalanceInfo {
    currency: String,
    total_balance: String,
    granted_balance: String,
    topped_up_balance: String,
}

#[async_trait]
impl Provider for DeepSeekProvider {
    fn id(&self) -> &'static str {
        "deepseek"
    }
    fn display_name(&self) -> &'static str {
        "DeepSeek"
    }
    fn brand(&self) -> Brand {
        Brand::DeepSeek
    }
    fn billing_mode(&self) -> BillingMode {
        BillingMode::PayAsYouGo
    }
    fn auth_spec(&self) -> AuthSpec {
        AuthSpec::ApiKey {
            fields: vec![AuthField {
                key: "api_key",
                label: "API Key",
                placeholder: "sk-...",
                secret: true,
                required: true,
                options: None,
            }],
            hint: "在 platform.deepseek.com 的 API 页面创建 Key。",
        }
    }
    async fn fetch(&self, cred: &Credential) -> anyhow::Result<ProviderSnapshot> {
        let api_key = cred
            .data
            .get("api_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 api_key"))?;

        let client = Client::new();
        let resp = client
            .get("https://api.deepseek.com/user/balance")
            .bearer_auth(api_key)
            .send()
            .await?
            .error_for_status()?
            .json::<DsBalanceResp>()
            .await?;

        let info = resp
            .balance_infos
            .first()
            .ok_or_else(|| anyhow::anyhow!("DeepSeek 未返回余额信息"))?;

        let total = info.total_balance.parse::<f64>().unwrap_or(0.0);
        let granted = info.granted_balance.parse::<f64>().unwrap_or(0.0);
        let topped_up = info.topped_up_balance.parse::<f64>().unwrap_or(0.0);

        Ok(ProviderSnapshot {
            provider_id: self.id().to_string(),
            display_name: self.display_name().to_string(),
            plan_name: None,
            billing: BillingMode::PayAsYouGo,
            balance: Some(Balance {
                total,
                granted: Some(granted),
                topped_up: Some(topped_up),
                currency: info.currency.clone(),
                available: resp.is_available,
            }),
            windows: vec![],
            fidelity: Fidelity::Exact,
            status: if resp.is_available {
                HealthStatus::Ok
            } else {
                HealthStatus::Exhausted
            },
            fetched_at: Utc::now().timestamp(),
        })
    }
}
