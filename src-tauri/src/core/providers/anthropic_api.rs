//! Anthropic 普通按量 API。
//!
//! Anthropic 没有公开账户余额接口；官方 Admin API 能查询组织成本与 Messages
//! Token 用量。因此本产品使用 Admin API Key，以“本月花费”为主值，并完整保留
//! 输入、输出、缓存和 Web Search 用量。

use super::*;
use async_trait::async_trait;
use chrono::{Datelike, SecondsFormat, Utc};
use serde_json::{json, Value};

const BASE_URL: &str = "https://api.anthropic.com/v1/organizations";
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicApiProvider;

impl AnthropicApiProvider {
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

fn result_rows(root: &Value) -> impl Iterator<Item = &Value> {
    root.get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|bucket| {
            bucket
                .get("results")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
}

fn monthly_cost(root: &Value) -> (f64, String) {
    let mut amount_minor = 0.0;
    let mut currency = "USD".to_string();
    for row in result_rows(root) {
        amount_minor += number(row.get("amount")).unwrap_or(0.0);
        if let Some(value) = text(row.get("currency")) {
            currency = value.to_uppercase();
        }
    }
    // Anthropic Cost API 的 amount 使用最小货币单位；USD 的 123.45 表示 $1.2345。
    (amount_minor / 100.0, currency)
}

#[derive(Debug, Default, PartialEq)]
struct UsageTotals {
    input: f64,
    output: f64,
    cache_creation: f64,
    cache_read: f64,
    web_searches: f64,
}

fn usage_totals(root: &Value) -> UsageTotals {
    let mut totals = UsageTotals::default();
    for row in result_rows(root) {
        totals.input += number(
            row.get("uncached_input_tokens")
                .or_else(|| row.get("input_tokens")),
        )
        .unwrap_or(0.0);
        totals.output += number(row.get("output_tokens")).unwrap_or(0.0);
        totals.cache_read += number(row.get("cache_read_input_tokens")).unwrap_or(0.0);
        if let Some(cache) = row.get("cache_creation").and_then(Value::as_object) {
            totals.cache_creation += cache
                .values()
                .filter_map(|value| number(Some(value)))
                .sum::<f64>();
        } else {
            totals.cache_creation += number(row.get("cache_creation_input_tokens")).unwrap_or(0.0);
        }
        totals.web_searches += row
            .get("server_tool_use")
            .and_then(|value| number(value.get("web_search_requests")))
            .unwrap_or(0.0);
    }
    totals
}

fn value_window(label: &str, value: f64, unit: QuotaUnit) -> QuotaWindow {
    QuotaWindow {
        period: WindowPeriod::Month,
        label: label.into(),
        used: None,
        used_raw: Some(value.max(0.0)),
        limit: None,
        remaining: None,
        unit,
        reset_at: None,
    }
}

fn usage_windows(cost: f64, currency: &str, usage: Option<&UsageTotals>) -> Vec<QuotaWindow> {
    let mut windows = vec![value_window(
        "本月花费",
        cost,
        QuotaUnit::Currency(currency.into()),
    )];
    let Some(usage) = usage else { return windows };
    windows.push(value_window("输入 Token", usage.input, QuotaUnit::Tokens));
    windows.push(value_window("输出 Token", usage.output, QuotaUnit::Tokens));
    if usage.cache_creation > 0.0 {
        windows.push(value_window(
            "缓存写入 Token",
            usage.cache_creation,
            QuotaUnit::Tokens,
        ));
    }
    if usage.cache_read > 0.0 {
        windows.push(value_window(
            "缓存读取 Token",
            usage.cache_read,
            QuotaUnit::Tokens,
        ));
    }
    if usage.web_searches > 0.0 {
        windows.push(value_window(
            "Web Search",
            usage.web_searches,
            QuotaUnit::Requests,
        ));
    }
    windows
}

async fn response_json(response: reqwest::Response, label: &str) -> anyhow::Result<Value> {
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        anyhow::bail!(
            "Anthropic {label}接口请求失败：HTTP {} · {}",
            status.as_u16(),
            body.trim()
        );
    }
    serde_json::from_str(&body)
        .map_err(|error| anyhow::anyhow!("Anthropic {label}接口响应无法解析：{error}"))
}

fn account_label(root: &Value) -> Option<String> {
    text(root.get("name")).or_else(|| text(root.get("id")))
}

#[async_trait]
impl Provider for AnthropicApiProvider {
    fn id(&self) -> &'static str {
        "anthropic_api"
    }

    fn display_name(&self) -> &'static str {
        "Anthropic API"
    }

    fn brand(&self) -> Brand {
        Brand::Anthropic
    }

    fn billing_mode(&self) -> BillingMode {
        BillingMode::PayAsYouGo
    }

    fn add_product_name(&self) -> &'static str {
        "Anthropic API"
    }

    fn add_description(&self) -> &'static str {
        "组织成本与 Token 用量"
    }

    fn detail_url(&self) -> Option<&'static str> {
        Some("https://platform.claude.com/usage")
    }

    fn auth_spec(&self) -> AuthSpec {
        AuthSpec::ApiKey {
            fields: vec![AuthField {
                key: "api_key",
                label: "Admin API Key",
                placeholder: "sk-ant-admin...",
                secret: true,
                required: true,
                options: None,
            }],
            hint: "需组织管理员创建的 Admin API Key。Anthropic 暂无公开余额接口，因此显示本月花费与 Token 用量；普通 sk-ant-api Key 无法读取这些报表。",
        }
    }

    fn supports_local_import(&self) -> bool {
        true
    }

    async fn detect_local(&self) -> Option<Credential> {
        let api_key = std::env::var("ANTHROPIC_ADMIN_API_KEY").ok()?;
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return None;
        }
        Some(Credential {
            data: json!({
                "api_key": api_key,
                "source_kind": "environment",
            }),
        })
    }

    async fn fetch(&self, cred: &Credential) -> anyhow::Result<ProviderSnapshot> {
        let api_key = cred
            .data
            .get("api_key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("缺少 api_key"))?;

        let now = Utc::now();
        let start = now
            .date_naive()
            .with_day(1)
            .expect("每个月都有 1 日")
            .and_hms_opt(0, 0, 0)
            .expect("有效时间")
            .and_utc()
            .to_rfc3339_opts(SecondsFormat::Secs, true);
        let end = now.to_rfc3339_opts(SecondsFormat::Secs, true);
        let client = super::http_client();
        let request = |path: &str| {
            client
                .get(format!("{BASE_URL}{path}"))
                .header("x-api-key", api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("Accept", "application/json")
        };
        let cost_request = request("/cost_report")
            .query(&[
                ("starting_at", start.as_str()),
                ("ending_at", end.as_str()),
                ("bucket_width", "1d"),
                ("limit", "31"),
            ])
            .send();
        let usage_request = request("/usage_report/messages")
            .query(&[
                ("starting_at", start.as_str()),
                ("ending_at", end.as_str()),
                ("bucket_width", "1d"),
                ("limit", "31"),
            ])
            .send();
        let organization_request = request("/me").send();
        let (cost_response, usage_response, organization_response) =
            tokio::join!(cost_request, usage_request, organization_request);

        let cost_root = response_json(cost_response?, "成本").await?;
        let usage_root = match usage_response {
            Ok(response) => response_json(response, "用量").await.ok(),
            Err(_) => None,
        };
        let organization_root = match organization_response {
            Ok(response) => response_json(response, "组织").await.ok(),
            Err(_) => None,
        };
        let (cost, currency) = monthly_cost(&cost_root);
        let usage = usage_root.as_ref().map(usage_totals);

        Ok(ProviderSnapshot {
            account_id: String::new(),
            account_label: organization_root.as_ref().and_then(account_label),
            provider_id: self.id().into(),
            display_name: self.display_name().into(),
            plan_name: None,
            billing: BillingMode::PayAsYouGo,
            balance: None,
            windows: usage_windows(cost, &currency, usage.as_ref()),
            fidelity: if usage.is_some() {
                Fidelity::Exact
            } else {
                Fidelity::Partial
            },
            status: HealthStatus::Ok,
            fetched_at: now.timestamp(),
            last_error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_minor_currency_units_and_sums_usage_dimensions() {
        let costs = json!({"data":[{"results":[
            {"amount":"123.45","currency":"USD"},
            {"amount":"76.55","currency":"USD"}
        ]}]});
        assert_eq!(monthly_cost(&costs), (2.0, "USD".into()));

        let usage = json!({"data":[{"results":[{
            "uncached_input_tokens":1500,
            "cache_creation":{"ephemeral_1h_input_tokens":1000,"ephemeral_5m_input_tokens":500},
            "cache_read_input_tokens":200,
            "output_tokens":500,
            "server_tool_use":{"web_search_requests":10}
        }]}]});
        assert_eq!(
            usage_totals(&usage),
            UsageTotals {
                input: 1500.0,
                output: 500.0,
                cache_creation: 1500.0,
                cache_read: 200.0,
                web_searches: 10.0,
            }
        );
    }

    #[test]
    fn primary_cost_is_kept_together_with_token_rows() {
        let totals = UsageTotals {
            input: 100.0,
            output: 20.0,
            ..UsageTotals::default()
        };
        let windows = usage_windows(1.25, "USD", Some(&totals));
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].label, "本月花费");
        assert_eq!(windows[1].label, "输入 Token");
        assert_eq!(windows[2].label, "输出 Token");
    }
}
