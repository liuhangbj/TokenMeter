//! 腾讯 TokenHub 按量（官方路径，数据完整度最高）
//!
//! 认证：SecretId / SecretKey（同 Token Plan）
//! 端点：
//!   - `DescribeUsageRankList`（Token 数，按 apikey/ endpoint / model 维度）
//!   - `billing.DescribeAccountBalance`（账户余额）
//!
//! ⚠️ API 版本 2026-03-22 为前瞻版本，真实字段需接入后校准。

use super::*;
use crate::core::providers::tencent;
use crate::core::providers::Brand;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};

pub struct TencentTokenHubProvider;

impl TencentTokenHubProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Provider for TencentTokenHubProvider {
    fn id(&self) -> &'static str {
        "tencent_tokenhub"
    }
    fn display_name(&self) -> &'static str {
        "腾讯 TokenHub 按量"
    }
    fn brand(&self) -> Brand {
        Brand::Tencent
    }
    fn billing_mode(&self) -> BillingMode {
        BillingMode::PayAsYouGo
    }
    fn add_account_type(&self) -> AddAccountType {
        // TokenHub 套餐按月购买 Token 总量；使用 SecretId/Key 只是认证方式，
        // 不应因此在添加界面归类为 API 按量账户。
        AddAccountType::Plan
    }
    fn add_product_name(&self) -> &'static str {
        "TokenHub Token 套餐"
    }
    fn add_description(&self) -> &'static str {
        "月度 Token 总量"
    }
    fn detail_url(&self) -> Option<&'static str> {
        Some("https://console.cloud.tencent.com/tokenhub")
    }
    fn auth_spec(&self) -> AuthSpec {
        AuthSpec::CloudSecret {
            fields: vec![
                AuthField {
                    key: "secret_id",
                    label: "SecretId",
                    placeholder: "AKID...",
                    secret: false,
                    required: true,
                    options: None,
                },
                AuthField {
                    key: "secret_key",
                    label: "SecretKey",
                    placeholder: "",
                    secret: true,
                    required: true,
                    options: None,
                },
            ],
        }
    }
    async fn fetch(&self, cred: &Credential) -> anyhow::Result<ProviderSnapshot> {
        let sid = cred
            .data
            .get("secret_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 secret_id"))?;
        let skey = cred
            .data
            .get("secret_key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 secret_key"))?;

        let client = super::http_client();
        let now = Utc::now();
        let start_time = (now - chrono::Duration::days(30)).to_rfc3339();
        let end_time = now.to_rfc3339();

        // Token 用量：按 API Key 维度拿整段汇总，同时保留输入/输出/缓存分项。
        let usage = tencent::tencent_post(
            client,
            "tokenhub",
            "tokenhub.tencentcloudapi.com",
            "DescribeUsageRankList",
            "2026-03-22",
            sid,
            skey,
            None,
            &json!({
                "Dimension": "apikey",
                "MetricType": "tokens",
                "StartTime": start_time,
                "EndTime": end_time,
                "Period": 86400,
                "ShowAll": true,
            }),
        )
        .await?;
        let usage_resp = usage.get("Response").cloned().unwrap_or(Value::Null);
        let usage_stats = usage_resp.get("TotalStats").unwrap_or(&usage_resp);
        let total_token = usage_stats.get("TotalToken").and_then(tencent::value_num);
        let mut fidelity = Fidelity::Exact;
        let mut windows = vec![];
        for (key, label) in [
            ("TotalToken", "近 30 天 Token"),
            ("InputTotalToken", "输入 Token"),
            ("OutputTotalToken", "输出 Token"),
            ("CacheTotalToken", "缓存命中 Token"),
        ] {
            if let Some(value) = usage_stats.get(key).and_then(tencent::value_num) {
                windows.push(QuotaWindow {
                    period: WindowPeriod::Month,
                    label: label.into(),
                    used: None,
                    used_raw: Some(value),
                    limit: None,
                    remaining: None,
                    unit: QuotaUnit::Tokens,
                    reset_at: None,
                });
            }
        }
        if total_token.is_none() {
            log::warn!("TokenHub DescribeUsageRankList 未返回 TotalStats.TotalToken，本次用量缺失");
            fidelity = Fidelity::Partial;
        }

        // 账户余额
        let bal = tencent::tencent_post(
            client,
            "billing",
            "billing.tencentcloudapi.com",
            "DescribeAccountBalance",
            "2018-07-09",
            sid,
            skey,
            None,
            &json!({}),
        )
        .await?;
        let bal_resp = bal.get("Response").cloned().unwrap_or(Value::Null);
        let account_label = bal_resp
            .get("Uin")
            .and_then(tencent::value_num)
            .map(|uin| format!("UIN {:.0}", uin));
        // 腾讯云余额接口单位为分，统一换算为元再进入 UI。
        let balance_total_cents = bal_resp
            .get("Balance")
            .and_then(tencent::value_num)
            .or_else(|| bal_resp.get("RealBalance").and_then(tencent::value_num));
        let cash_balance = bal_resp
            .get("CashAccountBalance")
            .and_then(tencent::value_num)
            .map(|value| value / 100.0);
        let present_balance = bal_resp
            .get("PresentAccountBalance")
            .and_then(tencent::value_num)
            .map(|value| value / 100.0);

        let (balance, status) = match balance_total_cents.map(|value| value / 100.0) {
            Some(value) => (
                Some(Balance {
                    total: value,
                    granted: present_balance,
                    topped_up: cash_balance,
                    currency: "CNY".into(),
                    available: value > 0.0,
                }),
                if value > 0.0 {
                    HealthStatus::Ok
                } else {
                    HealthStatus::Exhausted
                },
            ),
            None => {
                // 接口成功但字段缺失：明确标记降级，而不是把 0 当"余额耗尽"误报。
                log::warn!("TokenHub DescribeAccountBalance 未返回 Balance/RealBalance，余额未知");
                (None, HealthStatus::Degraded)
            }
        };

        Ok(ProviderSnapshot {
            account_id: String::new(),
            account_label,
            provider_id: self.id().to_string(),
            display_name: self.display_name().to_string(),
            plan_name: None,
            billing: BillingMode::PayAsYouGo,
            balance,
            windows,
            fidelity,
            status,
            fetched_at: Utc::now().timestamp(),
            last_error: None,
        })
    }
}
