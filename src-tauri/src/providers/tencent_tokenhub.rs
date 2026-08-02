//! 腾讯 TokenHub 按量（官方路径，数据完整度最高）
//!
//! 认证：SecretId / SecretKey（同 Token Plan）
//! 端点：
//!   - `DescribeUsageRankList`（Token 数，按 apikey/ endpoint / model 维度）
//!   - `billing.DescribeAccountBalance`（账户余额）
//!
//! ⚠️ API 版本 2026-03-22 为前瞻版本，真实字段需接入后校准。

use super::*;
use crate::providers::tencent;
use crate::providers::Brand;
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

        // Token 用量
        let usage = tencent::tencent_post(
            &client,
            "tokenhub",
            "tokenhub.tencentcloudapi.com",
            "DescribeUsageRankList",
            "2026-03-22",
            sid,
            skey,
            None,
            &json!({ "Dimension": "apikey", "MetricType": "tokens" }),
        )
        .await?;
        let usage_resp = usage.get("Response").cloned().unwrap_or(Value::Null);
        let total_token = usage_resp
            .get("TotalToken")
            .and_then(tencent::value_num)
            .or_else(|| {
                usage_resp
                    .get("TotalStats")
                    .and_then(|t| t.get("TotalToken"))
                    .and_then(tencent::value_num)
            });
        let mut fidelity = Fidelity::Exact;
        let mut windows = vec![];
        if let Some(t) = total_token {
            windows.push(QuotaWindow {
                period: WindowPeriod::Month,
                label: "本月 Token".into(),
                used: None, // 无上限，前端按 used_raw + Tokens 单位展示
                used_raw: Some(t),
                limit: None,
                remaining: None,
                unit: QuotaUnit::Tokens,
                reset_at: None,
            });
        } else {
            log::warn!("TokenHub DescribeUsageRankList 未返回 TotalToken（字段路径待校准），本次用量缺失");
            fidelity = Fidelity::Partial;
        }

        // 账户余额
        let bal = tencent::tencent_post(
            &client,
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
        // 真实字段可能为 Balance / RealBalance，防御式解析
        let balance_total = bal_resp
            .get("Balance")
            .and_then(tencent::value_num)
            .or_else(|| bal_resp.get("RealBalance").and_then(tencent::value_num));

        let (balance, status) = match balance_total {
            Some(v) => (
                Some(Balance {
                    total: v,
                    granted: None,
                    topped_up: None,
                    currency: "CNY".into(),
                    available: v > 0.0,
                }),
                if v > 0.0 {
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
