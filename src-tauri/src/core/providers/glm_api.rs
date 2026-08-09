//! GLM 普通按量 API（智谱国内站）。
//!
//! 余额与资源包来自 BigModel 控制台实时接口。资源包的 `tokensMagnitude` 是
//! 总额度，`availableBalance` 是当前可用额度，不是需要相乘的数量倍率。
//! 接口对历史 Key 同时出现过
//! `Bearer <key>` 与原始 `Authorization: <key>` 两种认证方式，因此仅在第一种
//! 失败时回退一次，避免用户已有 Key 因网关差异无法接入。

use super::*;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};
use std::fs;

const BASE_URL: &str = "https://open.bigmodel.cn";
const BALANCE_PATH: &str = "/api/biz/account/query-customer-account-report";
const PACKAGES_PATH: &str = "/api/biz/tokenAccounts/list/my?pageNum=1&pageSize=100";

pub struct GlmApiProvider;

impl GlmApiProvider {
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
    if root.get("success").and_then(Value::as_bool) == Some(false) {
        let message = text(root.get("msg")).unwrap_or_else(|| "未知错误".into());
        anyhow::bail!("GLM API 返回失败：{message}");
    }
    if let Some(code) = number(root.get("code")) {
        let code = code.round() as i64;
        if !matches!(code, 0 | 200) {
            let message = text(root.get("msg")).unwrap_or_else(|| "未知错误".into());
            anyhow::bail!("GLM API 返回失败：{message} ({code})");
        }
    }
    Ok(())
}

async fn send_json(path: &str, api_key: &str, bearer: bool) -> anyhow::Result<Value> {
    let request = super::http_client()
        .get(format!("{BASE_URL}{path}"))
        .header("Accept", "application/json");
    let request = if bearer {
        request.bearer_auth(api_key)
    } else {
        request.header("Authorization", api_key)
    };
    let response = request.send().await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        anyhow::bail!("HTTP {}：{}", status.as_u16(), body.trim());
    }
    let root = serde_json::from_str::<Value>(&body)?;
    validate_body(&root)?;
    Ok(root)
}

async fn fetch_json(path: &str, api_key: &str) -> anyhow::Result<Value> {
    match send_json(path, api_key, true).await {
        Ok(root) => Ok(root),
        Err(bearer_error) => send_json(path, api_key, false).await.map_err(|raw_error| {
            anyhow::anyhow!("Bearer 认证失败：{bearer_error}；原始 Key 认证失败：{raw_error}")
        }),
    }
}

fn balance_amount(root: &Value) -> Option<f64> {
    let data = data_object(root);
    number(data.get("availableBalance"))
        .or_else(|| number(data.get("available_balance")))
        .or_else(|| number(data.get("balance")))
}

fn account_label(root: &Value) -> Option<String> {
    let data = data_object(root);
    ["username", "userName", "email", "accountId", "userId"]
        .iter()
        .find_map(|key| text(data.get(*key)))
}

fn today_spend_window(root: &Value) -> Option<QuotaWindow> {
    let data = data_object(root);
    number(data.get("todaySpendAmount")).map(|value| QuotaWindow {
        period: WindowPeriod::Day,
        label: "今日花费".into(),
        used: None,
        used_raw: Some(value.max(0.0)),
        limit: None,
        remaining: None,
        unit: QuotaUnit::Currency("CNY".into()),
        reset_at: None,
    })
}

fn package_windows(root: &Value) -> Vec<QuotaWindow> {
    let rows = root
        .get("rows")
        .or_else(|| root.get("data").and_then(|data| data.get("rows")))
        .or_else(|| root.get("data"))
        .and_then(Value::as_array);
    let Some(rows) = rows else { return vec![] };

    rows.iter()
        .filter(|row| {
            text(row.get("status"))
                .map(|status| status.eq_ignore_ascii_case("EFFECTIVE"))
                .unwrap_or(true)
        })
        .filter_map(|row| {
            let remaining = number(
                row.get("availableBalance")
                    .or_else(|| row.get("available_balance")),
            )
            .or_else(|| number(row.get("tokenBalance").or_else(|| row.get("token_balance"))))?;
            if remaining < 0.0 {
                return None;
            }
            let name = text(
                row.get("resourcePackageName")
                    .or_else(|| row.get("resource_package_name")),
            )
            .or_else(|| text(row.get("suitableModel")))
            .unwrap_or_else(|| "Token 资源包".into());
            let unit =
                match text(row.get("consumeType").or_else(|| row.get("consume_type"))).as_deref() {
                    Some(value) if value.eq_ignore_ascii_case("TIMES") => QuotaUnit::Requests,
                    _ => QuotaUnit::Tokens,
                };
            Some(QuotaWindow {
                period: WindowPeriod::Custom(0),
                label: name,
                used: None,
                used_raw: Some(remaining),
                limit: None,
                remaining: None,
                unit,
                reset_at: None,
            })
        })
        .collect()
}

fn credential(api_key: &str, source: &str) -> Credential {
    Credential {
        data: json!({
            "api_key": api_key,
            "source_kind": source,
        }),
    }
}

fn read_plain_key(relative: &str) -> Option<String> {
    fs::read_to_string(super::home_dir().join(relative))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[async_trait]
impl Provider for GlmApiProvider {
    fn id(&self) -> &'static str {
        "glm_api"
    }

    fn display_name(&self) -> &'static str {
        "GLM API"
    }

    fn brand(&self) -> Brand {
        Brand::Glm
    }

    fn billing_mode(&self) -> BillingMode {
        BillingMode::PayAsYouGo
    }

    fn add_product_name(&self) -> &'static str {
        "GLM API"
    }

    fn add_description(&self) -> &'static str {
        "国内站余额与资源包"
    }

    fn detail_url(&self) -> Option<&'static str> {
        Some("https://open.bigmodel.cn/usercenter/financial-center/billing")
    }

    fn auth_spec(&self) -> AuthSpec {
        AuthSpec::ApiKey {
            fields: vec![AuthField {
                key: "api_key",
                label: "API Key",
                placeholder: "请输入 BigModel API Key",
                secret: true,
                required: true,
                options: None,
            }],
            hint: "用于智谱 BigModel 国内站的普通按量 API；GLM Coding Plan Key 请从同厂商的套餐入口添加。",
        }
    }

    fn supports_local_import(&self) -> bool {
        true
    }

    async fn detect_local(&self) -> Option<Credential> {
        for name in [
            "BIGMODEL_API_KEY",
            "ZHIPU_API_KEY",
            "ZHIPUAI_API_KEY",
            "GLM_API_KEY",
        ] {
            if let Ok(value) = std::env::var(name) {
                let api_key = value.trim();
                if !api_key.is_empty() {
                    return Some(credential(api_key, "environment"));
                }
            }
        }
        for relative in [
            ".coding-relay/glm-api-key",
            ".config/bigmodel/api_key",
            ".config/zhipu/api_key",
        ] {
            if let Some(api_key) = read_plain_key(relative) {
                return Some(credential(&api_key, "file"));
            }
        }
        None
    }

    async fn fetch(&self, cred: &Credential) -> anyhow::Result<ProviderSnapshot> {
        let api_key = cred
            .data
            .get("api_key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("缺少 api_key"))?;

        let balance_root = fetch_json(BALANCE_PATH, api_key).await?;
        let total = balance_amount(&balance_root)
            .ok_or_else(|| anyhow::anyhow!("GLM API 未返回可用余额"))?;
        let packages = fetch_json(PACKAGES_PATH, api_key).await.ok();
        let mut windows = today_spend_window(&balance_root)
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(packages) = packages.as_ref() {
            windows.extend(package_windows(packages));
        }

        Ok(ProviderSnapshot {
            account_id: String::new(),
            account_label: account_label(&balance_root),
            provider_id: self.id().into(),
            display_name: self.display_name().into(),
            plan_name: None,
            billing: BillingMode::PayAsYouGo,
            balance: Some(Balance {
                total,
                granted: None,
                topped_up: None,
                currency: "CNY".into(),
                available: total > 0.0,
            }),
            windows,
            fidelity: if packages.is_some() {
                Fidelity::Estimated
            } else {
                Fidelity::Partial
            },
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
    fn maps_available_balance_and_resource_packages_without_multiplying_totals() {
        let balance = json!({
            "code": 200,
            "success": true,
            "data": {"availableBalance": 89.16, "todaySpendAmount": 1.5}
        });
        assert_eq!(balance_amount(&balance), Some(89.16));
        assert_eq!(today_spend_window(&balance).unwrap().used_raw, Some(1.5));

        let packages = json!({"code":200,"rows":[
            {
                "tokenBalance":6000000,
                "availableBalance":6000000,
                "tokensMagnitude":6000000,
                "consumeType":"TOKENS",
                "status":"EFFECTIVE",
                "resourcePackageName":"600万 GLM-4.6V 资源包"
            },
            {
                "tokenBalance":20,
                "availableBalance":20,
                "tokensMagnitude":20,
                "consumeType":"TIMES",
                "status":"EFFECTIVE",
                "resourcePackageName":"20次图片/视频生成资源包"
            },
            {
                "tokenBalance":5138970,
                "availableBalance":5138970,
                "tokensMagnitude":10000000,
                "consumeType":"TOKENS",
                "status":"EXPIRED",
                "resourcePackageName":"已过期资源包"
            }
        ]});
        let windows = package_windows(&packages);
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].label, "600万 GLM-4.6V 资源包");
        assert_eq!(windows[0].used_raw, Some(6_000_000.0));
        assert_eq!(windows[0].limit, None);
        assert_eq!(windows[0].remaining, None);
        assert_eq!(windows[0].unit, QuotaUnit::Tokens);
        assert_eq!(windows[1].used_raw, Some(20.0));
        assert_eq!(windows[1].limit, None);
        assert_eq!(windows[1].unit, QuotaUnit::Requests);
    }

    #[test]
    fn rejects_success_false_even_when_http_would_be_ok() {
        let error = validate_body(&json!({"success":false,"code":1001,"msg":"invalid key"}))
            .expect_err("业务错误不能当作成功响应");
        assert!(error.to_string().contains("invalid key"));
    }
}
