//! MiniMax Token Plan（国际站 / 国内站）。
//!
//! 使用独立的 Subscription Key，并通过官方 `/v1/token_plan/remains` 查询
//! 多模态共享额度。该 Key 与普通按量 API Key 不通用。

use super::*;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};
use std::fs;

const GLOBAL_BASE: &str = "https://api.minimax.io";
const CN_BASE: &str = "https://api.minimaxi.com";

pub struct MiniMaxTokenPlanProvider;

impl MiniMaxTokenPlanProvider {
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

fn first_number(source: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| number(source.get(*key)))
}

fn first_text(source: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| text(source.get(*key)))
}

fn normalize_plan_name(raw: &str) -> String {
    let lower = raw.trim().to_lowercase();
    if lower.contains("ultra") {
        "Ultra".into()
    } else if lower.contains("plus") {
        "Plus".into()
    } else if lower.contains("starter") {
        "Starter".into()
    } else if lower.contains("free") {
        "Free".into()
    } else if lower == "max" || lower.ends_with(" max") || lower.contains("plan max") {
        "Max".into()
    } else {
        raw.trim().to_string()
    }
}

fn plan_name(root: &Value) -> Option<String> {
    let data = data_object(root);
    let keys = [
        "current_subscribe_title",
        "currentSubscribeTitle",
        "plan_name",
        "planName",
        "plan",
        "current_plan_title",
        "combo_title",
    ];
    first_text(data, &keys)
        .or_else(|| first_text(root, &keys))
        .map(|value| normalize_plan_name(&value))
}

fn points_balance(root: &Value) -> Option<f64> {
    let data = data_object(root);
    let keys = [
        "points_balance",
        "point_balance",
        "credits_balance",
        "credit_balance",
    ];
    first_number(data, &keys).or_else(|| first_number(root, &keys))
}

fn account_label(root: &Value) -> Option<String> {
    let data = data_object(root);
    let keys = [
        "username",
        "user_name",
        "display_name",
        "email",
        "account_id",
        "user_id",
        "group_id",
    ];
    first_text(data, &keys).or_else(|| first_text(root, &keys))
}

fn epoch_seconds(value: Option<f64>) -> Option<i64> {
    let value = value?;
    if value <= 0.0 {
        return None;
    }
    Some(if value >= 10_000_000_000.0 {
        (value / 1000.0).round() as i64
    } else {
        value.round() as i64
    })
}

fn reset_at(end: Option<f64>, remains: Option<f64>) -> Option<i64> {
    if let Some(end) = epoch_seconds(end) {
        return Some(end);
    }
    let remains = remains?;
    if remains <= 0.0 {
        return None;
    }
    // 历史响应同时出现秒和毫秒；Token Plan 最长是周窗口，超过 8 天秒数
    // 的值按毫秒解释。
    let seconds = if remains > 8.0 * 24.0 * 60.0 * 60.0 {
        remains / 1000.0
    } else {
        remains
    };
    Some(
        Utc::now()
            .timestamp()
            .saturating_add(seconds.round() as i64),
    )
}

fn duration_seconds(start: Option<f64>, end: Option<f64>) -> Option<i64> {
    let start = epoch_seconds(start)?;
    let end = epoch_seconds(end)?;
    (end > start).then_some(end - start)
}

fn interval_period(duration: Option<i64>) -> WindowPeriod {
    match duration {
        Some(seconds) if (seconds - 5 * 60 * 60).abs() <= 15 * 60 => WindowPeriod::Hours5,
        Some(seconds) if (seconds - 24 * 60 * 60).abs() <= 30 * 60 => WindowPeriod::Day,
        Some(seconds) => WindowPeriod::Custom((seconds / 60).max(1)),
        None => WindowPeriod::Hours5,
    }
}

fn interval_label(period: WindowPeriod) -> &'static str {
    match period {
        WindowPeriod::Hours5 => "5 小时",
        WindowPeriod::Day => "24 小时",
        _ => "周期",
    }
}

fn is_general_model(name: &str) -> bool {
    let name = name.trim().to_lowercase();
    name == "general"
        || name.contains("minimax-m")
        || name.contains("text generation")
        || name == "text"
}

fn model_label(name: &str) -> String {
    let lower = name.to_lowercase();
    if lower.contains("speech") || lower.contains("audio") {
        "Speech".into()
    } else if lower.contains("image") {
        "Image".into()
    } else if lower.contains("video") {
        "Video".into()
    } else if lower.contains("music") {
        "Music".into()
    } else {
        name.trim().to_string()
    }
}

struct WindowFields<'a> {
    total: &'a [&'a str],
    ambiguous_usage: &'a [&'a str],
    explicit_used: &'a [&'a str],
    explicit_remaining: &'a [&'a str],
    remaining_percent: &'a [&'a str],
    start: &'a [&'a str],
    end: &'a [&'a str],
    remains: &'a [&'a str],
    status: &'a [&'a str],
}

const INTERVAL_FIELDS: WindowFields<'static> = WindowFields {
    total: &["current_interval_total_count", "currentIntervalTotalCount"],
    ambiguous_usage: &["current_interval_usage_count", "currentIntervalUsageCount"],
    explicit_used: &["current_interval_used_count", "currentIntervalUsedCount"],
    explicit_remaining: &[
        "current_interval_remaining_count",
        "currentIntervalRemainingCount",
        "current_interval_remains_count",
        "currentIntervalRemainsCount",
    ],
    remaining_percent: &[
        "current_interval_remaining_percent",
        "currentIntervalRemainingPercent",
    ],
    start: &["start_time", "startTime"],
    end: &["end_time", "endTime"],
    remains: &["remains_time", "remainsTime"],
    status: &["current_interval_status", "currentIntervalStatus"],
};

const WEEKLY_FIELDS: WindowFields<'static> = WindowFields {
    total: &["current_weekly_total_count", "currentWeeklyTotalCount"],
    ambiguous_usage: &["current_weekly_usage_count", "currentWeeklyUsageCount"],
    explicit_used: &["current_weekly_used_count", "currentWeeklyUsedCount"],
    explicit_remaining: &[
        "current_weekly_remaining_count",
        "currentWeeklyRemainingCount",
        "current_weekly_remains_count",
        "currentWeeklyRemainsCount",
    ],
    remaining_percent: &[
        "current_weekly_remaining_percent",
        "currentWeeklyRemainingPercent",
    ],
    start: &["weekly_start_time", "weeklyStartTime"],
    end: &["weekly_end_time", "weeklyEndTime"],
    remains: &["weekly_remains_time", "weeklyRemainsTime"],
    status: &["current_weekly_status", "currentWeeklyStatus"],
};

/// 新版官方 endpoint 把 usage_count 定义为已用；历史 remains endpoint 曾把它
/// 当余量。若返回 remaining_percent，则用它自动判别两种语义。
fn used_and_remaining(
    item: &Value,
    fields: &WindowFields<'_>,
) -> (Option<f64>, Option<f64>, Option<f64>) {
    let total = first_number(item, fields.total);
    let remaining_percent =
        first_number(item, fields.remaining_percent).map(|value| value.clamp(0.0, 100.0));
    let explicit_used = first_number(item, fields.explicit_used);
    let explicit_remaining = first_number(item, fields.explicit_remaining);
    let ambiguous = first_number(item, fields.ambiguous_usage);

    let used = match (total, explicit_used, explicit_remaining, ambiguous) {
        (_, Some(used), _, _) => Some(used.max(0.0)),
        (Some(total), _, Some(remaining), _) => Some((total - remaining).clamp(0.0, total)),
        (Some(total), _, _, Some(value)) if total > 0.0 => {
            let as_used = value.clamp(0.0, total);
            let as_remaining = (total - value).clamp(0.0, total);
            match remaining_percent {
                Some(percent) => {
                    let expected_used = total * (1.0 - percent / 100.0);
                    if (as_remaining - expected_used).abs() < (as_used - expected_used).abs() {
                        Some(as_remaining)
                    } else {
                        Some(as_used)
                    }
                }
                // 当前官方 /v1/token_plan/remains 契约。
                None => Some(as_used),
            }
        }
        _ => None,
    };
    let remaining = match (total, explicit_remaining, used) {
        (_, Some(value), _) => Some(value.max(0.0)),
        (Some(total), _, Some(used)) => Some((total - used).max(0.0)),
        _ => None,
    };
    (total, used, remaining)
}

fn make_window(item: &Value, model: &str, weekly: bool) -> Option<QuotaWindow> {
    let fields = if weekly {
        &WEEKLY_FIELDS
    } else {
        &INTERVAL_FIELDS
    };
    let status = first_number(item, fields.status).map(|value| value.round() as i64);
    let remaining_percent =
        first_number(item, fields.remaining_percent).map(|value| value.clamp(0.0, 100.0));
    let (total, used, remaining) = used_and_remaining(item, fields);

    if status == Some(3)
        && total.unwrap_or(0.0) <= 0.0
        && remaining.unwrap_or(0.0) <= 0.0
        && remaining_percent.is_some_and(|value| value >= 100.0)
    {
        return None;
    }

    let duration = duration_seconds(
        first_number(item, fields.start),
        first_number(item, fields.end),
    );
    let period = if weekly {
        WindowPeriod::Week
    } else {
        interval_period(duration)
    };
    let general = is_general_model(model);
    let label = if general {
        if weekly {
            "7 天额度".into()
        } else {
            format!("{}额度", interval_label(period))
        }
    } else if weekly {
        format!("{} 7 天额度", model_label(model))
    } else {
        format!("{} {}额度", model_label(model), interval_label(period))
    };

    if total.is_some_and(|value| value > 0.0) {
        let total = total?;
        let used = used?;
        Some(QuotaWindow {
            period,
            label,
            used: Some((used / total * 100.0).clamp(0.0, 100.0)),
            used_raw: Some(used),
            limit: Some(total),
            remaining,
            unit: QuotaUnit::Requests,
            reset_at: reset_at(
                first_number(item, fields.end),
                first_number(item, fields.remains),
            ),
        })
    } else {
        let remaining_percent = remaining_percent?;
        Some(QuotaWindow {
            period,
            label,
            used: Some((100.0 - remaining_percent).clamp(0.0, 100.0)),
            used_raw: None,
            limit: Some(100.0),
            remaining: Some(remaining_percent),
            unit: QuotaUnit::Percent,
            reset_at: reset_at(
                first_number(item, fields.end),
                first_number(item, fields.remains),
            ),
        })
    }
}

fn windows(root: &Value) -> Vec<QuotaWindow> {
    let data = data_object(root);
    let Some(items) = data
        .get("model_remains")
        .or_else(|| data.get("modelRemains"))
        .or_else(|| root.get("model_remains"))
        .and_then(Value::as_array)
    else {
        return vec![];
    };
    let mut items = items.iter().collect::<Vec<_>>();
    // 标准主值按同周期取最后一项；通用文本额度放到最后，避免 Speech/Image
    // 的周额度被选成账户主值。
    items.sort_by_key(|item| {
        let name = first_text(item, &["model_name", "modelName"]).unwrap_or_default();
        is_general_model(&name)
    });

    let mut result = Vec::new();
    for item in items {
        let model =
            first_text(item, &["model_name", "modelName"]).unwrap_or_else(|| "General".into());
        if let Some(window) = make_window(item, &model, false) {
            result.push(window);
        }
        if let Some(window) = make_window(item, &model, true) {
            result.push(window);
        }
    }
    result
}

fn api_key_credential(
    api_key: &str,
    region: &str,
    source: &str,
    account: Option<&str>,
) -> Credential {
    Credential {
        data: json!({
            "api_key": api_key,
            "region": region,
            "source_kind": source,
            "account_label": account,
        }),
    }
}

fn read_json(path: &std::path::Path) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn oauth_auth_expired(provider: &MiniMaxTokenPlanProvider, cred: &Credential) -> ProviderSnapshot {
    ProviderSnapshot {
        account_id: String::new(),
        account_label: cred
            .data
            .get("account_label")
            .and_then(Value::as_str)
            .map(str::to_string),
        provider_id: provider.id().into(),
        display_name: provider.display_name().into(),
        plan_name: None,
        billing: BillingMode::Subscription,
        balance: None,
        windows: vec![],
        fidelity: Fidelity::Exact,
        status: HealthStatus::AuthExpired,
        fetched_at: Utc::now().timestamp(),
        last_error: None,
    }
}

#[async_trait]
impl Provider for MiniMaxTokenPlanProvider {
    fn id(&self) -> &'static str {
        "minimax_token_plan"
    }

    fn display_name(&self) -> &'static str {
        "MiniMax Token Plan"
    }

    fn brand(&self) -> Brand {
        Brand::MiniMax
    }

    fn billing_mode(&self) -> BillingMode {
        BillingMode::Subscription
    }

    fn add_product_name(&self) -> &'static str {
        "MiniMax Token Plan"
    }

    fn add_description(&self) -> &'static str {
        "多模态套餐额度"
    }

    fn detail_url(&self) -> Option<&'static str> {
        Some("https://platform.minimax.io/user-center/payment/token-plan")
    }

    fn card_config(&self) -> presentation::CardConfig {
        let mut config = presentation::CardConfig::for_billing(self.billing_mode());
        config.detail_url = self.detail_url();
        config.primary_window = presentation::PrimaryWindowSelection::HighestNonCurrency;
        config.balance_role = presentation::BalanceRole::Supplemental {
            label: "Credits 余额",
        };
        config
    }

    fn plan_tier(&self, plan_name: &str) -> Option<u8> {
        let name = plan_name.to_lowercase();
        if name.contains("free") {
            Some(0)
        } else if name.contains("starter") {
            Some(1)
        } else if name.contains("plus") {
            Some(2)
        } else if name.contains("ultra") {
            Some(4)
        } else if name == "max" || name.ends_with(" max") || name.contains("plan max") {
            Some(3)
        } else {
            None
        }
    }

    fn auth_spec(&self) -> AuthSpec {
        AuthSpec::ApiKey {
            fields: vec![
                AuthField {
                    key: "api_key",
                    label: "Subscription Key",
                    placeholder: "sk-cp-...",
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
                    options: Some(vec![("global", "国际站 (minimax.io)"), ("cn", "国内站 (minimaxi.com)")]),
                },
            ],
            hint: "请填写 Token Plan 页面提供的 Subscription Key（通常为 sk-cp- 前缀）；普通按量 API Key 不可混用。",
        }
    }

    fn supports_local_import(&self) -> bool {
        true
    }

    async fn detect_local(&self) -> Option<Credential> {
        for name in ["MINIMAX_TOKEN_PLAN_CN_KEY", "MINIMAX_CN_API_KEY"] {
            if let Ok(value) = std::env::var(name) {
                let key = value.trim();
                if !key.is_empty() {
                    return Some(api_key_credential(key, "cn", "environment", None));
                }
            }
        }
        for name in [
            "MINIMAX_TOKEN_PLAN_GLOBAL_KEY",
            "MINIMAX_CODING_API_KEY",
            "MINIMAX_API_KEY",
            "MINIMAX_API_TOKEN",
        ] {
            if let Ok(value) = std::env::var(name) {
                let key = value.trim();
                if !key.is_empty() {
                    return Some(api_key_credential(key, "global", "environment", None));
                }
            }
        }

        let config_path = super::home_dir().join(".mmx/config.json");
        let config = read_json(&config_path);
        let configured_region = config
            .as_ref()
            .and_then(|value| value.get("region"))
            .and_then(Value::as_str)
            .unwrap_or("global")
            .to_string();

        // 官方 mmx CLI 优先使用 OAuth 凭证，再回退到 config.json 的 API Key。
        let credentials_path = super::home_dir().join(".mmx/credentials.json");
        if let Some(root) = read_json(&credentials_path) {
            if let Some(access) = root
                .get("access_token")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                let refresh = root
                    .get("refresh_token")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                return Some(Credential {
                    data: json!({
                        "api_key": access,
                        "refresh_token": refresh,
                        "expires_at": root.get("expires_at").and_then(Value::as_str),
                        "region": &configured_region,
                        "account_label": root.get("account").and_then(Value::as_str),
                        "source_kind": "mmx_oauth",
                    }),
                });
            }
        }
        if let Some(root) = config {
            if let Some(key) = root
                .get("api_key")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return Some(api_key_credential(
                    key,
                    &configured_region,
                    "mmx_config",
                    None,
                ));
            }
        }
        None
    }

    async fn refresh(&self, cred: &Credential) -> anyhow::Result<Option<Credential>> {
        let refresh_token = cred
            .data
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let Some(refresh_token) = refresh_token else {
            return Ok(None);
        };
        let region = cred
            .data
            .get("region")
            .and_then(Value::as_str)
            .unwrap_or("global");
        let base = if region == "cn" { CN_BASE } else { GLOBAL_BASE };
        let response = super::http_client()
            .post(format!("{base}/v1/oauth/token"))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ])
            .send()
            .await?
            .error_for_status()?;
        let value = response.json::<Value>().await?;
        let access_token = value
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("MiniMax 刷新响应缺少 access_token"))?;
        let new_refresh_token = value
            .get("refresh_token")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or(refresh_token);
        let expires_in = number(value.get("expires_in")).unwrap_or(3600.0).round() as i64;
        let mut data = cred.data.clone();
        data["api_key"] = json!(access_token);
        data["refresh_token"] = json!(new_refresh_token);
        data["expires_at"] = json!(Utc::now().timestamp().saturating_add(expires_in));
        Ok(Some(Credential { data }))
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
            .unwrap_or("global");
        let base = if region == "cn" { CN_BASE } else { GLOBAL_BASE };
        let response = super::http_client()
            .get(format!("{base}/v1/token_plan/remains"))
            .bearer_auth(api_key)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("MM-API-Source", "TokenMeter")
            .send()
            .await?;
        if matches!(response.status().as_u16(), 401 | 403)
            && cred
                .data
                .get("refresh_token")
                .and_then(Value::as_str)
                .is_some()
        {
            return Ok(oauth_auth_expired(self, cred));
        }
        let response = response.error_for_status()?;
        let root = response.json::<Value>().await?;
        let data = data_object(&root);
        let base_resp = data.get("base_resp").or_else(|| root.get("base_resp"));
        if let Some(base_resp) = base_resp {
            let code = number(base_resp.get("status_code")).unwrap_or(0.0).round() as i64;
            if code != 0 {
                let message =
                    text(base_resp.get("status_msg")).unwrap_or_else(|| "未知错误".into());
                if code == 1004
                    && cred
                        .data
                        .get("refresh_token")
                        .and_then(Value::as_str)
                        .is_some()
                {
                    return Ok(oauth_auth_expired(self, cred));
                }
                anyhow::bail!("MiniMax Token Plan 返回失败：{message} ({code})");
            }
        }
        let windows = windows(&root);
        if windows.is_empty() {
            anyhow::bail!("MiniMax Token Plan 未返回可用额度窗口");
        }
        let credits = points_balance(&root);
        let main_used = windows
            .iter()
            .filter(|window| window.label == "7 天额度" || window.label == "5 小时额度")
            .max_by_key(|window| matches!(window.period, WindowPeriod::Week))
            .and_then(|window| window.used)
            .or_else(|| windows.last().and_then(|window| window.used));

        Ok(ProviderSnapshot {
            account_id: String::new(),
            account_label: account_label(&root).or_else(|| {
                cred.data
                    .get("account_label")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            }),
            provider_id: self.id().into(),
            display_name: self.display_name().into(),
            plan_name: plan_name(&root),
            billing: BillingMode::Subscription,
            balance: credits.map(|value| Balance {
                total: value,
                granted: None,
                topped_up: None,
                currency: "Credits".into(),
                available: value > 0.0,
            }),
            windows,
            fidelity: Fidelity::Exact,
            status: if main_used.is_some_and(|value| value >= 100.0) {
                HealthStatus::Exhausted
            } else {
                HealthStatus::Ok
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
    fn current_endpoint_usage_count_is_used_when_no_remaining_percent_exists() {
        let payload = json!({
            "base_resp":{"status_code":0,"status_msg":"success"},
            "model_remains":[{
                "model_name":"MiniMax-M*",
                "start_time":1776355200000_i64,
                "end_time":1776373200000_i64,
                "current_interval_total_count":1500,
                "current_interval_usage_count":228,
                "current_weekly_total_count":6000,
                "current_weekly_usage_count":500
            }]
        });
        let windows = windows(&payload);
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].label, "5 小时额度");
        assert_eq!(windows[0].used_raw, Some(228.0));
        assert_eq!(windows[0].remaining, Some(1272.0));
        assert_eq!(windows[1].used_raw, Some(500.0));
    }

    #[test]
    fn remaining_percent_disambiguates_legacy_usage_count() {
        let payload = json!({
            "model_remains":[{
                "model_name":"general",
                "current_interval_total_count":1500,
                "current_interval_usage_count":1473,
                "current_interval_remaining_percent":98
            }]
        });
        let windows = windows(&payload);
        assert_eq!(windows[0].used_raw, Some(27.0));
        assert_eq!(windows[0].remaining, Some(1473.0));
    }

    #[test]
    fn percentage_only_and_multimodal_rows_are_kept_but_placeholders_are_skipped() {
        let payload = json!({
            "current_subscribe_title":"Token Plan Ultra",
            "points_balance":1400,
            "model_remains":[
                {"model_name":"video","current_interval_total_count":0,"current_interval_usage_count":0,"current_interval_remaining_percent":100,"current_interval_status":3},
                {"model_name":"general","current_interval_total_count":0,"current_interval_usage_count":0,"current_interval_remaining_percent":97,"current_weekly_total_count":0,"current_weekly_usage_count":0,"current_weekly_remaining_percent":88}
            ]
        });
        let windows = windows(&payload);
        assert_eq!(windows.len(), 2);
        assert!(matches!(windows[0].unit, QuotaUnit::Percent));
        assert_eq!(windows[0].used, Some(3.0));
        assert_eq!(plan_name(&payload).as_deref(), Some("Ultra"));
        assert_eq!(points_balance(&payload), Some(1400.0));
    }

    #[test]
    fn minimax_vendor_name_does_not_turn_plus_or_ultra_into_max() {
        assert_eq!(normalize_plan_name("MiniMax Token Plan Plus"), "Plus");
        assert_eq!(normalize_plan_name("MiniMax Token Plan Ultra"), "Ultra");
        let provider = MiniMaxTokenPlanProvider::new();
        assert_eq!(provider.plan_tier("Plus"), Some(2));
        assert_eq!(provider.plan_tier("Max"), Some(3));
        assert_eq!(provider.plan_tier("Ultra"), Some(4));
    }
}
