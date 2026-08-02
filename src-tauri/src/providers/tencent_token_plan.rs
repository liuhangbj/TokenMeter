//! 腾讯 Token Plan（订阅，Token 数计量，官方路径）
//!
//! 认证：SecretId / SecretKey，TC3-HMAC-SHA256（见 `tencent.rs`）
//! 端点：`DescribeTokenPlanList` 取 TeamId → `DescribeTokenPlan` 取详情
//! 返回 Name / StopReason / 额度包余量（字段路径待实测，做防御式解析）。
//!
//! ⚠️ 注意：API 版本 2026-03-22 为前瞻版本，真实字段需接入后校准。
//!
//! 🔴 2026-08-02 实测：本接口面向**企业版**（CAM SecretId/Key + tp-ent- 套餐）。
//! 用户购买的**个人版** Token Plan 走专属 `sk-tp-` Key + tencentcloudmaas.com，
//! 该域名只暴露推理 API、**无查用量端点（全 404）**。个人版官方查询路径走不通，
//! 故按用户决策隐藏入口；企业版代码保留，未来官方开放个人版 API 时改回 true。

use super::*;
use crate::providers::tencent;
use crate::providers::Brand;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};

pub struct TencentTokenPlanProvider;

impl TencentTokenPlanProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Provider for TencentTokenPlanProvider {
    fn id(&self) -> &'static str {
        "tencent_token_plan"
    }
    fn display_name(&self) -> &'static str {
        "腾讯 Token Plan"
    }
    fn brand(&self) -> Brand {
        Brand::Tencent
    }
    fn billing_mode(&self) -> BillingMode {
        BillingMode::Subscription
    }
    /// 个人版无官方查询 API（2026-08-02 实测），隐藏「添加供应商」入口。
    /// 企业版代码保留，未来官方开放个人版 API 时改回 true 即恢复。
    fn enabled(&self) -> bool {
        false
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
        let list = tencent::tencent_post(
            &client,
            "tokenhub",
            "tokenhub.tencentcloudapi.com",
            "DescribeTokenPlanList",
            "2026-03-22",
            sid,
            skey,
            None,
            &json!({}),
        )
        .await?;

        let team_id = list
            .get("Response")
            .and_then(|r| r.get("TeamId"))
            .and_then(|v| v.as_str())
            .or_else(|| {
                list.get("Response")
                    .and_then(|r| r.get("PlanList"))
                    .and_then(|p| p.get(0))
                    .and_then(|p| p.get("TeamId"))
                    .and_then(|v| v.as_str())
            })
            .ok_or_else(|| anyhow::anyhow!("无法解析 TeamId（API 字段待实测）"))?;

        let detail = tencent::tencent_post(
            &client,
            "tokenhub",
            "tokenhub.tencentcloudapi.com",
            "DescribeTokenPlan",
            "2026-03-22",
            sid,
            skey,
            None,
            &json!({ "TeamId": team_id }),
        )
        .await?;

        let resp = detail.get("Response").cloned().unwrap_or(Value::Null);
        let plan_name = resp.get("Name").and_then(|v| v.as_str()).map(|s| s.to_string());
        let stop_reason = resp.get("StopReason").and_then(|v| v.as_str()).unwrap_or("NORMAL");
        let status = match stop_reason {
            "EXHAUSTED" => HealthStatus::Exhausted,
            "FROZEN" | "ISOLATED" | "DESTROYED" => HealthStatus::AuthExpired,
            _ => HealthStatus::Ok,
        };

        let remaining = resp
            .get("Remaining")
            .and_then(tencent::value_num)
            .or_else(|| {
                resp.get("Quota")
                    .and_then(|q| q.get("Remaining"))
                    .and_then(tencent::value_num)
            });
        let limit = resp
            .get("Total")
            .and_then(tencent::value_num)
            .or_else(|| {
                resp.get("Quota")
                    .and_then(|q| q.get("Total"))
                    .and_then(tencent::value_num)
            });

        let mut windows = vec![];
        if let (Some(rem), Some(lim)) = (remaining, limit) {
            let used_raw = (lim - rem).max(0.0);
            let used_pct = if lim > 0.0 {
                Some((used_raw / lim * 100.0).min(100.0))
            } else {
                None
            };
            windows.push(QuotaWindow {
                period: WindowPeriod::Month,
                label: "本月额度".into(),
                used: used_pct,
                used_raw: Some(used_raw),
                limit: Some(lim),
                remaining: Some(rem),
                unit: QuotaUnit::Tokens,
                reset_at: None,
            });
        }

        Ok(ProviderSnapshot {
            provider_id: self.id().to_string(),
            display_name: self.display_name().to_string(),
            plan_name,
            billing: BillingMode::Subscription,
            balance: None,
            windows,
            fidelity: Fidelity::Exact,
            status,
            fetched_at: Utc::now().timestamp(),
            last_error: None,
        })
    }
}
