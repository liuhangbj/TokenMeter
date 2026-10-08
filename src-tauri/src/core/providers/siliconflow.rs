//! SiliconFlow 普通按量 API。
//!
//! 中国站官方已公告 `/v1/user/info` 于 2026-08-14 停止服务，且截至
//! 2026-10-02 尚未公布账户级替代 API。国际站官方文档仍保留该接口，不能把
//! 中国站公告外推到国际站。新账户只开放国际站；既有中国站账户保留但不再
//! 发起必然失败的请求，而是返回无余额的明确降级快照。

use super::*;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

pub struct SiliconFlowProvider;

impl SiliconFlowProvider {
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

fn endpoint(region: &str) -> (&'static str, &'static str) {
    if region.eq_ignore_ascii_case("global") {
        ("https://api.siliconflow.com/v1/user/info", "USD")
    } else {
        ("https://api.siliconflow.cn/v1/user/info", "CNY")
    }
}

fn validate_response(root: &Value) -> anyhow::Result<&Value> {
    let code = number(root.get("code")).map(|value| value.round() as i64);
    let success = root.get("status").and_then(Value::as_bool);
    if code.is_some_and(|code| !matches!(code, 0 | 200 | 20000)) || success == Some(false) {
        let message = text(root.get("message")).unwrap_or_else(|| "未知错误".into());
        anyhow::bail!(
            "SiliconFlow API 返回失败：{message} ({})",
            code.unwrap_or(-1)
        );
    }
    root.get("data")
        .filter(|value| value.is_object())
        .ok_or_else(|| anyhow::anyhow!("SiliconFlow API 未返回 data"))
}

#[async_trait]
impl Provider for SiliconFlowProvider {
    fn id(&self) -> &'static str {
        "siliconflow"
    }

    fn display_name(&self) -> &'static str {
        "SiliconFlow"
    }

    fn brand(&self) -> Brand {
        Brand::SiliconFlow
    }

    fn billing_mode(&self) -> BillingMode {
        BillingMode::PayAsYouGo
    }

    fn add_product_name(&self) -> &'static str {
        "SiliconFlow API"
    }

    fn add_description(&self) -> &'static str {
        "国际站充值与赠送余额"
    }

    fn detail_url(&self) -> Option<&'static str> {
        Some("https://cloud.siliconflow.cn/expensebill")
    }

    fn card_config(&self) -> presentation::CardConfig {
        let mut config = presentation::CardConfig::for_billing(self.billing_mode());
        config.detail_url = self.detail_url();
        config.balance_role = presentation::BalanceRole::Primary {
            topped_up_label: "充值余额",
            granted_label: "赠送余额",
        };
        config
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
                    options: Some(vec![("global", "国际站（美元）")]),
                },
            ],
            hint: "国际站账户接口返回总余额、充值余额与赠送余额。中国站 /user/info 已于 2026-08-14 停服，暂不接受新增；既有账户仍可管理或删除。",
        }
    }

    fn supports_local_import(&self) -> bool {
        false
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
            .unwrap_or("cn");
        if !region.eq_ignore_ascii_case("global") {
            return Ok(ProviderSnapshot {
                account_id: String::new(),
                account_label: None,
                provider_id: self.id().into(),
                display_name: self.display_name().into(),
                plan_name: None,
                billing: BillingMode::PayAsYouGo,
                balance: None,
                windows: vec![],
                fidelity: Fidelity::Partial,
                status: HealthStatus::Degraded,
                fetched_at: Utc::now().timestamp(),
                last_error: Some(
                    "SiliconFlow 中国站 /user/info 已于 2026-08-14 停止服务，官方尚未提供账户余额替代接口"
                        .into(),
                ),
            });
        }
        let (url, currency) = endpoint(region);
        let response = super::http_client()
            .get(url)
            .bearer_auth(api_key)
            .header("Accept", "application/json")
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            anyhow::bail!("SiliconFlow HTTP {}：{}", status.as_u16(), body.trim());
        }
        let root = serde_json::from_str::<Value>(&body)?;
        let data = validate_response(&root)?;
        let total = number(data.get("totalBalance"))
            .or_else(|| number(data.get("total_balance")))
            .ok_or_else(|| anyhow::anyhow!("SiliconFlow API 未返回 totalBalance"))?;
        let topped_up = number(
            data.get("chargeBalance")
                .or_else(|| data.get("charge_balance")),
        );
        let granted = number(data.get("balance"));
        let service_ok = text(data.get("status"))
            .map(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "normal" | "active" | "ok"
                )
            })
            .unwrap_or(true);
        let account_label = ["name", "email", "id", "userId"]
            .iter()
            .find_map(|key| text(data.get(*key)));
        let available = service_ok && total > 0.0;

        Ok(ProviderSnapshot {
            account_id: String::new(),
            account_label,
            provider_id: self.id().into(),
            display_name: self.display_name().into(),
            plan_name: None,
            billing: BillingMode::PayAsYouGo,
            balance: Some(Balance {
                total,
                granted,
                topped_up,
                currency: currency.into(),
                available,
            }),
            windows: vec![],
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

    #[test]
    fn maps_total_charge_and_granted_balances() {
        let root = serde_json::json!({
            "code": 20000,
            "status": true,
            "data": {
                "totalBalance": "38.50",
                "chargeBalance": "30.00",
                "balance": "8.50",
                "status": "normal",
                "email": ""
            }
        });
        let data = validate_response(&root).unwrap();
        assert_eq!(number(data.get("totalBalance")), Some(38.5));
        assert_eq!(number(data.get("chargeBalance")), Some(30.0));
        assert_eq!(number(data.get("balance")), Some(8.5));
        assert_eq!(text(data.get("email")), None);
    }

    #[test]
    fn supports_cn_and_global_endpoints() {
        assert_eq!(endpoint("cn").1, "CNY");
        assert_eq!(endpoint("global").1, "USD");
    }

    #[tokio::test]
    async fn existing_cn_account_becomes_explicit_degraded_snapshot_without_balance() {
        let provider = SiliconFlowProvider::new();
        let snapshot = provider
            .fetch(&Credential {
                data: serde_json::json!({"api_key":"redacted","region":"cn"}),
            })
            .await
            .unwrap();
        assert!(snapshot.balance.is_none());
        assert_eq!(snapshot.status, HealthStatus::Degraded);
        assert_eq!(snapshot.fidelity, Fidelity::Partial);
        assert!(snapshot
            .last_error
            .as_deref()
            .unwrap()
            .contains("2026-08-14"));
    }

    #[test]
    fn new_accounts_only_offer_the_still_documented_global_site() {
        let provider = SiliconFlowProvider::new();
        let AuthSpec::ApiKey { fields, .. } = provider.auth_spec() else {
            panic!("SiliconFlow 应继续使用 API Key 认证");
        };
        let region = fields.iter().find(|field| field.key == "region").unwrap();
        assert_eq!(
            region.options.as_deref(),
            Some(&[("global", "国际站（美元）")][..])
        );
        assert!(!provider.supports_local_import());
        assert!(provider.enabled());
    }
}
