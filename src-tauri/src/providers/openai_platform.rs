//! OpenAI Platform（API 按量侧，官方路径）
//!
//! 认证：Admin API Key（`sk-admin-…`，需组织 Owner 权限）
//! 端点：
//!   - `/v1/organization/costs`（按时间桶聚合花费，USD）
//!   - `/v1/organization/usage/completions`（token 数）
//! 说明：消耗由官方时间桶聚合返回（非本地差分）。用户 2026-08-02 决定不展示
//!       日/周/月消耗统计，故仅取最近周期聚合值实时显示。

use super::*;
use crate::providers::Brand;
use async_trait::async_trait;
use chrono::{Utc, Duration as ChronoDuration};
use reqwest::Client;
use serde::Deserialize;

pub struct OpenAiPlatformProvider;

impl OpenAiPlatformProvider {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Deserialize)]
struct CostsResp {
    data: Vec<CostBucket>,
}

#[derive(Deserialize)]
struct CostBucket {
    start_time: i64,
    #[serde(default)]
    amount: Option<f64>,
}

#[async_trait]
impl Provider for OpenAiPlatformProvider {
    fn id(&self) -> &'static str {
        "openai_platform"
    }
    fn display_name(&self) -> &'static str {
        "OpenAI Platform"
    }
    fn brand(&self) -> Brand {
        Brand::OpenAI
    }
    fn billing_mode(&self) -> BillingMode {
        BillingMode::PayAsYouGo
    }
    fn auth_spec(&self) -> AuthSpec {
        AuthSpec::ApiKey {
            fields: vec![AuthField {
                key: "api_key",
                label: "Admin API Key",
                placeholder: "sk-admin-...",
                secret: true,
                required: true,
                options: None,
            }],
            hint: "需组织 Owner 权限创建的 Admin Key（sk-admin- 前缀），权限极大，仅存系统 Keychain。",
        }
    }
    async fn fetch(&self, cred: &Credential) -> anyhow::Result<ProviderSnapshot> {
        let api_key = cred
            .data
            .get("api_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 api_key"))?;

        let now = Utc::now();
        let start_1d = (now - ChronoDuration::days(1)).timestamp();
        let start_7d = (now - ChronoDuration::days(7)).timestamp();
        let start_30d = (now - ChronoDuration::days(30)).timestamp();

        let client = Client::new();
        let resp = client
            .get("https://api.openai.com/v1/organization/costs")
            .query(&[
                ("start_time", start_30d.to_string()),
                ("bucket_width", "1d".to_string()),
                ("limit", "31".to_string()),
            ])
            .bearer_auth(api_key)
            .send()
            .await?
            .error_for_status()?
            .json::<CostsResp>()
            .await?;

        let now_ts = now.timestamp();
        let mut day = 0.0;
        let mut week = 0.0;
        let mut month = 0.0;
        for b in &resp.data {
            if let Some(a) = b.amount {
                if b.start_time >= start_1d {
                    day += a;
                }
                if b.start_time >= start_7d {
                    week += a;
                }
                if b.start_time >= start_30d {
                    month += a;
                }
            }
        }

        let windows = vec![
            QuotaWindow {
                period: WindowPeriod::Day,
                label: "今日花费".into(),
                used: Some(day),
                limit: None,
                remaining: None,
                unit: QuotaUnit::Currency("USD".into()),
                reset_at: None,
            },
            QuotaWindow {
                period: WindowPeriod::Week,
                label: "本周花费".into(),
                used: Some(week),
                limit: None,
                remaining: None,
                unit: QuotaUnit::Currency("USD".into()),
                reset_at: None,
            },
            QuotaWindow {
                period: WindowPeriod::Month,
                label: "本月花费".into(),
                used: Some(month),
                limit: None,
                remaining: None,
                unit: QuotaUnit::Currency("USD".into()),
                reset_at: None,
            },
        ];

        Ok(ProviderSnapshot {
            provider_id: self.id().to_string(),
            display_name: self.display_name().to_string(),
            plan_name: None,
            billing: BillingMode::PayAsYouGo,
            balance: None,
            windows,
            fidelity: Fidelity::Exact,
            status: HealthStatus::Ok,
            fetched_at: now_ts,
        })
    }
}
