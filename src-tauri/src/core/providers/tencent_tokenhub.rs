//! 腾讯 TokenHub Token Plan 企业版（既有账户兼容，暂停新增）。
//!
//! 腾讯公开 `DescribeTokenPlan` 契约只提供跨周期累计 `PackageInfo.TotalUsed`
//! 和本期上限 `PackageInfo.CycleQuota`，没有可与本期上限安全相减的套餐级本期
//! 已用/剩余额度。API Key 的 `Balance.TotalUsed` 属于子额度包，不能冒充套餐主
//! 额度；`TokenSummary` 也只给原始 Token 明细，不能换算专业套餐积分。
//!
//! 国际站还使用独立签名 Host。当前阶段既未获授权新增该 Host，也没有真实账号
//! 验证。因此本 Provider 暂停新增；既有账户与凭证继续保留，但刷新只返回明确的
//! Degraded 快照，不发送凭证、不显示推测的 0/100%。

use super::*;
use crate::core::providers::Brand;
use async_trait::async_trait;
use chrono::Utc;

const DOMESTIC_REGION: &str = "ap-guangzhou";

pub struct TencentTokenHubProvider;

impl TencentTokenHubProvider {
    pub fn new() -> Self {
        Self
    }
}

fn paused_reason(cred: &Credential) -> String {
    let region = cred
        .data
        .get("region")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DOMESTIC_REGION);

    if region != DOMESTIC_REGION {
        format!(
            "该账户配置为 {region}；腾讯 TokenHub 国际站使用独立签名 Host，本版本未获授权接入，未发送 SecretId/SecretKey。账户与凭证已保留，可继续管理或删除。"
        )
    } else {
        "腾讯 TokenHub 官方公开契约未提供可与本期额度对应的套餐级本期已用/剩余额度；本版本已暂停查询，未显示推测的 0/100%。账户与凭证已保留，可继续管理或删除。".to_string()
    }
}

fn paused_snapshot(cred: &Credential) -> ProviderSnapshot {
    let account_label = cred
        .data
        .get("team_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    ProviderSnapshot {
        account_id: String::new(),
        account_label,
        provider_id: "tencent_tokenhub".to_string(),
        display_name: "腾讯 TokenHub".to_string(),
        plan_name: None,
        billing: BillingMode::Subscription,
        balance: None,
        windows: vec![],
        fidelity: Fidelity::Partial,
        status: HealthStatus::Degraded,
        fetched_at: Utc::now().timestamp(),
        last_error: Some(paused_reason(cred)),
    }
}

#[async_trait]
impl Provider for TencentTokenHubProvider {
    fn id(&self) -> &'static str {
        "tencent_tokenhub"
    }

    fn display_name(&self) -> &'static str {
        "腾讯 TokenHub"
    }

    fn brand(&self) -> Brand {
        Brand::Tencent
    }

    fn billing_mode(&self) -> BillingMode {
        BillingMode::Subscription
    }

    fn add_product_name(&self) -> &'static str {
        "Token Plan 企业版"
    }

    fn add_description(&self) -> &'static str {
        "当前周期额度接口待官方补齐"
    }

    fn detail_url(&self) -> Option<&'static str> {
        Some("https://console.cloud.tencent.com/tokenhub/token-plan")
    }

    fn card_config(&self) -> presentation::CardConfig {
        let mut config = presentation::CardConfig::for_billing(self.billing_mode());
        config.detail_url = self.detail_url();
        config.primary_window = presentation::PrimaryWindowSelection::HighestNonCurrency;
        config
    }

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
                AuthField {
                    key: "region",
                    label: "套餐地域",
                    placeholder: "",
                    secret: false,
                    required: true,
                    options: Some(vec![(DOMESTIC_REGION, "广州（中国站）")]),
                },
                AuthField {
                    key: "team_id",
                    label: "TeamId（多套餐时必填）",
                    placeholder: "tp-ent-...",
                    secret: false,
                    required: false,
                    options: None,
                },
            ],
        }
    }

    async fn fetch(&self, cred: &Credential) -> anyhow::Result<ProviderSnapshot> {
        Ok(paused_snapshot(cred))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_is_hidden_until_package_cycle_balance_is_proven() {
        let provider = TencentTokenHubProvider::new();
        assert!(!provider.enabled());
        let AuthSpec::CloudSecret { fields } = provider.auth_spec() else {
            panic!("TokenHub 应保留云密钥兼容契约");
        };
        let regions = fields
            .iter()
            .find(|field| field.key == "region")
            .and_then(|field| field.options.as_deref());
        assert_eq!(regions, Some(&[(DOMESTIC_REGION, "广州（中国站）")][..]));
    }

    #[test]
    fn domestic_legacy_account_degrades_without_inventing_quota() {
        let credential = Credential {
            data: serde_json::json!({
                "secret_id": "not-read",
                "secret_key": "not-read",
                "region": DOMESTIC_REGION,
                "team_id": "tp-ent-redacted"
            }),
        };
        let snapshot = paused_snapshot(&credential);
        assert_eq!(snapshot.account_label.as_deref(), Some("tp-ent-redacted"));
        assert_eq!(snapshot.status, HealthStatus::Degraded);
        assert_eq!(snapshot.fidelity, Fidelity::Partial);
        assert!(snapshot.balance.is_none());
        assert!(snapshot.windows.is_empty());
        let reason = snapshot.last_error.expect("暂停原因必须对用户可见");
        assert!(reason.contains("套餐级本期已用/剩余额度"));
        assert!(reason.contains("未显示推测的 0/100%"));
    }

    #[test]
    fn international_legacy_account_never_falls_back_to_domestic_host() {
        let credential = Credential {
            data: serde_json::json!({
                "secret_id": "not-sent",
                "secret_key": "not-sent",
                "region": "ap-singapore"
            }),
        };
        let snapshot = paused_snapshot(&credential);
        assert_eq!(snapshot.status, HealthStatus::Degraded);
        assert!(snapshot.windows.is_empty());
        let reason = snapshot.last_error.expect("国际站暂停原因必须对用户可见");
        assert!(reason.contains("独立签名 Host"));
        assert!(reason.contains("未发送 SecretId/SecretKey"));
    }
}
