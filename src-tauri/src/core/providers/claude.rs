//! Anthropic Claude 订阅套餐（Claude Code OAuth）。
//!
//! 凭证来源：macOS Keychain `Claude Code-credentials`；Windows/Linux 及兼容
//! 环境使用 `~/.claude/.credentials.json`。额度来自 Claude Code 使用的 OAuth
//! usage/profile 接口，映射到统一套餐卡片契约。

use super::*;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";
const SOURCE_KIND: &str = "source_kind";
const SOURCE_PATH: &str = "source_path";
const SOURCE_PAYLOAD: &str = "source_payload";

pub struct ClaudeProvider;

impl ClaudeProvider {
    pub fn new() -> Self {
        Self
    }

    fn candidate_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Ok(config_dir) = std::env::var("CLAUDE_CONFIG_DIR") {
            if !config_dir.trim().is_empty() {
                paths.push(PathBuf::from(config_dir).join(".credentials.json"));
            }
        }
        paths.push(super::home_dir().join(".claude/.credentials.json"));
        paths
    }
}

#[derive(Debug, Deserialize)]
struct UsageResponse {
    #[serde(default)]
    five_hour: Option<UsageWindow>,
    #[serde(default)]
    seven_day: Option<UsageWindow>,
    #[serde(default)]
    seven_day_oauth_apps: Option<UsageWindow>,
    #[serde(default)]
    seven_day_opus: Option<UsageWindow>,
    #[serde(default)]
    seven_day_sonnet: Option<UsageWindow>,
    #[serde(
        default,
        alias = "seven_day_claude_routines",
        alias = "seven_day_cowork"
    )]
    seven_day_routines: Option<UsageWindow>,
    #[serde(default)]
    limits: Vec<UsageLimit>,
    #[serde(default)]
    extra_usage: Option<ExtraUsage>,
}

#[derive(Debug, Clone, Deserialize)]
struct UsageWindow {
    #[serde(default)]
    utilization: Option<f64>,
    #[serde(default)]
    resets_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UsageLimit {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    percent: Option<f64>,
    #[serde(default)]
    resets_at: Option<String>,
    #[serde(default)]
    scope: Option<LimitScope>,
    #[serde(default)]
    is_active: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct LimitScope {
    #[serde(default)]
    model: Option<LimitModel>,
}

#[derive(Debug, Deserialize)]
struct LimitModel {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExtraUsage {
    #[serde(default)]
    is_enabled: Option<bool>,
    #[serde(default)]
    monthly_limit: Option<f64>,
    #[serde(default)]
    used_credits: Option<f64>,
    #[serde(default)]
    utilization: Option<f64>,
    #[serde(default)]
    currency: Option<String>,
}

fn parse_reset(value: Option<&str>) -> Option<i64> {
    value.and_then(|raw| {
        DateTime::parse_from_rfc3339(raw)
            .ok()
            .map(|date| date.timestamp())
    })
}

fn percentage_window(period: WindowPeriod, label: &str, source: &UsageWindow) -> QuotaWindow {
    let used = source.utilization.map(|value| value.clamp(0.0, 100.0));
    QuotaWindow {
        period,
        label: label.to_string(),
        used,
        used_raw: None,
        limit: Some(100.0),
        remaining: used.map(|value| (100.0 - value).max(0.0)),
        unit: QuotaUnit::Percent,
        reset_at: parse_reset(source.resets_at.as_deref()),
    }
}

fn credential_from_root(
    root: Value,
    source_kind: &str,
    source_path: Option<&Path>,
) -> Option<Credential> {
    let oauth = root.get("claudeAiOauth")?;
    let access_token = oauth.get("accessToken")?.as_str()?.trim();
    if access_token.is_empty() {
        return None;
    }
    Some(Credential {
        data: json!({
            "access_token": access_token,
            "refresh_token": oauth.get("refreshToken").and_then(Value::as_str).unwrap_or(""),
            "expires_at": oauth.get("expiresAt").and_then(Value::as_f64),
            "scopes": oauth.get("scopes").cloned().unwrap_or_else(|| json!([])),
            "rate_limit_tier": oauth.get("rateLimitTier").and_then(Value::as_str),
            "subscription_type": oauth.get("subscriptionType").and_then(Value::as_str),
            (SOURCE_KIND): source_kind,
            (SOURCE_PATH): source_path.map(|path| path.to_string_lossy().to_string()),
            (SOURCE_PAYLOAD): root,
        }),
    })
}

pub(crate) fn oauth_credential<'a>(
    access_token: &str,
    refresh_token: &str,
    expires_in: i64,
    scopes: impl Iterator<Item = &'a str>,
) -> Credential {
    let expires_at = Utc::now().timestamp_millis() + expires_in.saturating_mul(1000);
    let scopes = scopes.collect::<Vec<_>>();
    Credential {
        data: json!({
            "access_token": access_token,
            "refresh_token": refresh_token,
            "expires_at": expires_at,
            "scopes": scopes,
            "source_kind": "oauth",
            "source_payload": {
                "claudeAiOauth": {
                    "accessToken": access_token,
                    "refreshToken": refresh_token,
                    "expiresAt": expires_at,
                    "scopes": scopes,
                }
            },
        }),
    }
}

fn plan_name(credential: &Credential) -> Option<String> {
    let tier = credential
        .data
        .get("rate_limit_tier")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    let subscription = credential
        .data
        .get("subscription_type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    let combined = format!("{tier} {subscription}");

    if combined.contains("20x") {
        Some("Max 20X".into())
    } else if combined.contains("5x") || combined.contains("max") {
        Some("Max 5X".into())
    } else if combined.contains("team") && combined.contains("premium") {
        Some("Team Premium".into())
    } else if combined.contains("team") {
        Some("Team Standard".into())
    } else if combined.contains("enterprise") {
        Some("Enterprise".into())
    } else if combined.contains("pro") {
        Some("Pro".into())
    } else if combined.contains("free") {
        Some("Free".into())
    } else {
        None
    }
}

fn profile_label(value: &Value) -> Option<String> {
    let nested_email = value.get("account").and_then(|account| {
        [
            "displayName",
            "display_name",
            "name",
            "emailAddress",
            "email_address",
            "email",
        ]
        .iter()
        .find_map(|key| account.get(key).and_then(Value::as_str))
    });
    nested_email
        .or_else(|| {
            [
                "displayName",
                "display_name",
                "name",
                "emailAddress",
                "email_address",
                "email",
            ]
            .iter()
            .find_map(|key| value.get(key).and_then(Value::as_str))
        })
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(str::to_string)
        .or_else(|| {
            value
                .get("organization")
                .and_then(|organization| organization.get("uuid"))
                .and_then(Value::as_str)
                .or_else(|| value.get("organization_uuid").and_then(Value::as_str))
                .map(str::to_string)
        })
}

fn profile_plan_name(value: &Value) -> Option<String> {
    let organization = value.get("organization")?;
    let tier = organization
        .get("rate_limit_tier")
        .or_else(|| organization.get("rateLimitTier"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let organization_type = organization
        .get("organization_type")
        .or_else(|| organization.get("organizationType"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let account = value.get("account");
    let has_max = account
        .and_then(|account| account.get("has_claude_max"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let has_pro = account
        .and_then(|account| account.get("has_claude_pro"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let flags = format!("{tier} {organization_type}").to_lowercase();
    if flags.contains("20x") {
        Some("Max 20X".into())
    } else if flags.contains("5x") || has_max {
        Some("Max 5X".into())
    } else if flags.contains("team") && flags.contains("premium") {
        Some("Team Premium".into())
    } else if flags.contains("team") {
        Some("Team Standard".into())
    } else if flags.contains("enterprise") {
        Some("Enterprise".into())
    } else if has_pro {
        Some("Pro".into())
    } else {
        None
    }
}

fn build_windows(usage: &UsageResponse) -> Vec<QuotaWindow> {
    let mut windows = Vec::new();
    let mut labels = HashSet::new();
    let mut push = |label: &str, source: &Option<UsageWindow>| {
        if let Some(source) = source {
            labels.insert(label.to_lowercase());
            windows.push(percentage_window(WindowPeriod::Week, label, source));
        }
    };

    // 模型/功能分项先放，通用 7 天额度最后放；标准卡片会把后者选为主值。
    push("OAuth Apps 7 天额度", &usage.seven_day_oauth_apps);
    push("Opus 7 天额度", &usage.seven_day_opus);
    push("Sonnet 7 天额度", &usage.seven_day_sonnet);
    push("Cowork 7 天额度", &usage.seven_day_routines);

    for limit in &usage.limits {
        if limit.is_active == Some(false) {
            continue;
        }
        let model = limit.scope.as_ref().and_then(|scope| scope.model.as_ref());
        let model_name = model
            .and_then(|model| model.display_name.as_deref().or(model.id.as_deref()))
            .map(str::trim)
            .filter(|name| !name.is_empty());
        let Some(percent) = limit.percent else {
            continue;
        };
        let group = limit.group.as_deref().unwrap_or("").to_lowercase();
        let kind = limit.kind.as_deref().unwrap_or("").to_lowercase();
        let period = if group.contains("week") || kind.contains("week") {
            WindowPeriod::Week
        } else if group.contains("day") || kind.contains("day") {
            WindowPeriod::Day
        } else {
            WindowPeriod::Custom(0)
        };
        let label = match model_name {
            Some(name) if matches!(period, WindowPeriod::Week) => format!("{name} 7 天额度"),
            Some(name) => format!("{name} 额度"),
            None => continue,
        };
        if !labels.insert(label.to_lowercase()) {
            continue;
        }
        let used = percent.clamp(0.0, 100.0);
        windows.push(QuotaWindow {
            period,
            label,
            used: Some(used),
            used_raw: None,
            limit: Some(100.0),
            remaining: Some((100.0 - used).max(0.0)),
            unit: QuotaUnit::Percent,
            reset_at: parse_reset(limit.resets_at.as_deref()),
        });
    }

    if let Some(source) = &usage.five_hour {
        windows.push(percentage_window(
            WindowPeriod::Hours5,
            "5 小时额度",
            source,
        ));
    }
    if let Some(source) = &usage.seven_day {
        windows.push(percentage_window(WindowPeriod::Week, "7 天额度", source));
    }

    if let Some(extra) = &usage.extra_usage {
        if extra.is_enabled == Some(true) {
            if let (Some(limit_minor), Some(used_minor)) = (extra.monthly_limit, extra.used_credits)
            {
                // Anthropic OAuth 返回货币最小单位（美分等），统一换算成主单位。
                let limit = limit_minor / 100.0;
                let used_raw = used_minor / 100.0;
                let used = extra
                    .utilization
                    .or_else(|| (limit > 0.0).then_some(used_raw / limit * 100.0))
                    .map(|value| value.clamp(0.0, 100.0));
                windows.push(QuotaWindow {
                    period: WindowPeriod::Month,
                    label: "Extra Usage".into(),
                    used,
                    used_raw: Some(used_raw),
                    limit: Some(limit),
                    remaining: Some((limit - used_raw).max(0.0)),
                    unit: QuotaUnit::Currency(
                        extra.currency.as_deref().unwrap_or("USD").to_uppercase(),
                    ),
                    reset_at: None,
                });
            }
        }
    }
    windows
}

fn auth_expired_snapshot(provider: &ClaudeProvider) -> ProviderSnapshot {
    ProviderSnapshot {
        account_id: String::new(),
        account_label: None,
        provider_id: provider.id().into(),
        display_name: provider.display_name().into(),
        plan_name: None,
        billing: BillingMode::Subscription,
        balance: None,
        windows: vec![],
        fidelity: Fidelity::Estimated,
        status: HealthStatus::AuthExpired,
        fetched_at: Utc::now().timestamp(),
        last_error: None,
    }
}

fn merged_refreshed_credential(
    old: &Credential,
    access_token: &str,
    refresh_token: &str,
    expires_in: i64,
) -> Credential {
    let expires_at = Utc::now().timestamp_millis() + expires_in.saturating_mul(1000);
    let mut data = old.data.clone();
    data["access_token"] = json!(access_token);
    data["refresh_token"] = json!(refresh_token);
    data["expires_at"] = json!(expires_at);
    if let Some(root) = data.get_mut(SOURCE_PAYLOAD) {
        if let Some(oauth) = root.get_mut("claudeAiOauth") {
            oauth["accessToken"] = json!(access_token);
            oauth["refreshToken"] = json!(refresh_token);
            oauth["expiresAt"] = json!(expires_at);
        }
    }
    Credential { data }
}

fn sync_refreshed_file(credential: &Credential) -> anyhow::Result<()> {
    if credential.data.get(SOURCE_KIND).and_then(Value::as_str) != Some("file") {
        return Ok(());
    }
    let Some(path) = credential
        .data
        .get(SOURCE_PATH)
        .and_then(Value::as_str)
        .map(PathBuf::from)
    else {
        return Ok(());
    };
    if !ClaudeProvider::candidate_paths()
        .iter()
        .any(|candidate| candidate == &path)
    {
        anyhow::bail!("拒绝写入非 Claude CLI 凭证路径");
    }
    let root = credential
        .data
        .get(SOURCE_PAYLOAD)
        .ok_or_else(|| anyhow::anyhow!("缺少 Claude CLI 原始凭证"))?;
    let bytes = serde_json::to_vec_pretty(root)?;
    crate::core::store::write_private(&path, &bytes)
}

#[cfg(target_os = "macos")]
async fn read_keychain_payload() -> Option<Value> {
    let mut command = tokio::process::Command::new("/usr/bin/security");
    command
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
        .kill_on_drop(true);
    let output = tokio::time::timeout(std::time::Duration::from_secs(15), command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

#[cfg(target_os = "macos")]
async fn sync_refreshed_keychain(credential: &Credential) -> anyhow::Result<()> {
    if credential.data.get(SOURCE_KIND).and_then(Value::as_str) != Some("keychain") {
        return Ok(());
    }
    let root = credential
        .data
        .get(SOURCE_PAYLOAD)
        .ok_or_else(|| anyhow::anyhow!("缺少 Claude Keychain 原始凭证"))?;
    let payload = serde_json::to_string(root)?;
    let mut command = tokio::process::Command::new("/usr/bin/security");
    command
        .args([
            "add-generic-password",
            "-U",
            "-s",
            KEYCHAIN_SERVICE,
            "-w",
            &payload,
        ])
        .kill_on_drop(true);
    let output = tokio::time::timeout(std::time::Duration::from_secs(15), command.output())
        .await
        .map_err(|_| anyhow::anyhow!("更新 Claude Keychain 超时"))??;
    if !output.status.success() {
        anyhow::bail!("更新 Claude Keychain 失败");
    }
    Ok(())
}

#[async_trait]
impl Provider for ClaudeProvider {
    fn id(&self) -> &'static str {
        "claude"
    }
    fn display_name(&self) -> &'static str {
        "Claude"
    }
    fn brand(&self) -> Brand {
        Brand::Anthropic
    }
    fn billing_mode(&self) -> BillingMode {
        BillingMode::Subscription
    }
    fn add_product_name(&self) -> &'static str {
        "Claude 套餐"
    }
    fn add_description(&self) -> &'static str {
        "Claude Code 套餐额度"
    }
    fn detail_url(&self) -> Option<&'static str> {
        Some("https://claude.ai/settings/usage")
    }
    fn card_config(&self) -> presentation::CardConfig {
        let mut config = presentation::CardConfig::for_billing(self.billing_mode());
        config.detail_url = self.detail_url();
        config.primary_window = presentation::PrimaryWindowSelection::HighestNonCurrency;
        config.currency_window_role = presentation::CurrencyWindowRole::RemainingBalance {
            label_suffix: " 余额",
        };
        config
    }
    fn plan_tier(&self, plan_name: &str) -> Option<u8> {
        let name = plan_name.to_lowercase();
        if name.contains("free") {
            Some(0)
        } else if name.contains("20x") {
            Some(5)
        } else if name.contains("5x") || name.contains("team premium") {
            Some(4)
        } else if name.contains("pro") || name.contains("team standard") {
            Some(2)
        } else {
            None
        }
    }
    fn auth_spec(&self) -> AuthSpec {
        AuthSpec::OAuth {
            authorize_url: "https://claude.ai/login",
            token_url: TOKEN_URL,
            client_id: CLIENT_ID,
            scopes: &["user:profile", "user:inference"],
            pkce: true,
        }
    }

    async fn detect_local(&self) -> Option<Credential> {
        #[cfg(target_os = "macos")]
        if let Some(root) = read_keychain_payload().await {
            if let Some(credential) = credential_from_root(root, "keychain", None) {
                return Some(credential);
            }
        }

        for path in Self::candidate_paths() {
            let Ok(source) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(root) = serde_json::from_str::<Value>(&source) else {
                continue;
            };
            if let Some(credential) = credential_from_root(root, "file", Some(&path)) {
                return Some(credential);
            }
        }
        None
    }

    async fn refresh(&self, cred: &Credential) -> anyhow::Result<Option<Credential>> {
        let Some(refresh_token) = cred
            .data
            .get("refresh_token")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
        else {
            return Ok(None);
        };
        let response = super::http_client()
            .post(TOKEN_URL)
            .header("Content-Type", "application/json")
            .json(&json!({
                "grant_type": "refresh_token",
                "refresh_token": refresh_token,
                "client_id": CLIENT_ID,
                "scope": "user:profile user:inference",
            }))
            .send()
            .await?;
        if !response.status().is_success() {
            anyhow::bail!("Claude 凭证续期失败 HTTP {}", response.status());
        }
        let value = response.json::<Value>().await?;
        let Some(access_token) = value.get("access_token").and_then(Value::as_str) else {
            return Ok(None);
        };
        let new_refresh_token = value
            .get("refresh_token")
            .and_then(Value::as_str)
            .unwrap_or(refresh_token);
        let expires_in = value
            .get("expires_in")
            .and_then(Value::as_i64)
            .unwrap_or(3600);
        let refreshed =
            merged_refreshed_credential(cred, access_token, new_refresh_token, expires_in);

        if let Err(error) = sync_refreshed_file(&refreshed) {
            log::warn!("同步 Claude CLI 文件凭证失败: {error}");
        }
        #[cfg(target_os = "macos")]
        if let Err(error) = sync_refreshed_keychain(&refreshed).await {
            // TokenMeter 已拿到可用的新 token，Keychain 同步失败不应丢弃它。
            log::warn!("同步 Claude Keychain 凭证失败: {error}");
        }
        Ok(Some(refreshed))
    }

    async fn fetch(&self, cred: &Credential) -> anyhow::Result<ProviderSnapshot> {
        let access_token = cred
            .data
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .ok_or_else(|| anyhow::anyhow!("缺少 access_token"))?;
        let client = super::http_client();
        let usage_request = client
            .get(USAGE_URL)
            .bearer_auth(access_token)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .header("anthropic-beta", "oauth-2025-04-20")
            .header("User-Agent", "claude-code/2.1.0")
            .send();
        let profile_request = client
            .get(PROFILE_URL)
            .bearer_auth(access_token)
            .header("Accept", "application/json")
            .header("anthropic-beta", "oauth-2025-04-20")
            .send();
        let (usage_response, profile_response) = tokio::join!(usage_request, profile_request);
        let usage_response = usage_response?;
        if matches!(usage_response.status().as_u16(), 401 | 403) {
            return Ok(auth_expired_snapshot(self));
        }
        let usage = usage_response
            .error_for_status()?
            .json::<UsageResponse>()
            .await?;
        let profile = match profile_response {
            Ok(response) if response.status().is_success() => response.json::<Value>().await.ok(),
            _ => None,
        };
        let account_label = profile.as_ref().and_then(profile_label);
        let current_plan_name = profile
            .as_ref()
            .and_then(profile_plan_name)
            .or_else(|| plan_name(cred));
        let windows = build_windows(&usage);
        let main_used = usage
            .seven_day
            .as_ref()
            .and_then(|window| window.utilization)
            .or_else(|| {
                usage
                    .five_hour
                    .as_ref()
                    .and_then(|window| window.utilization)
            });

        Ok(ProviderSnapshot {
            account_id: String::new(),
            account_label,
            provider_id: self.id().into(),
            display_name: self.display_name().into(),
            plan_name: current_plan_name,
            billing: BillingMode::Subscription,
            balance: None,
            windows,
            fidelity: Fidelity::Estimated,
            status: if main_used.is_some_and(|used| used >= 100.0) {
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
    fn parses_cli_credentials_and_plan_tier() {
        let credential = credential_from_root(
            json!({
                "claudeAiOauth": {
                    "accessToken": "access",
                    "refreshToken": "refresh",
                    "expiresAt": 1234,
                    "scopes": ["user:profile"],
                    "rateLimitTier": "default_claude_max_5x",
                    "subscriptionType": "max"
                },
                "future": { "keep": true }
            }),
            "file",
            Some(Path::new("/tmp/.credentials.json")),
        )
        .unwrap();
        assert_eq!(credential.data["access_token"], "access");
        assert_eq!(credential.data[SOURCE_PAYLOAD]["future"]["keep"], true);
        assert_eq!(plan_name(&credential).as_deref(), Some("Max 5X"));
    }

    #[test]
    fn maps_usage_windows_and_converts_extra_usage_cents() {
        let usage: UsageResponse = serde_json::from_value(json!({
            "five_hour": { "utilization": 12.4, "resets_at": "2026-08-09T10:00:00Z" },
            "seven_day": { "utilization": 34.6, "resets_at": "2026-08-12T10:00:00Z" },
            "limits": [{
                "kind": "weekly_scoped",
                "group": "weekly",
                "percent": 20,
                "scope": { "model": { "id": "claude-opus", "display_name": "Opus" } },
                "is_active": true
            }],
            "extra_usage": {
                "is_enabled": true,
                "monthly_limit": 5000,
                "used_credits": 1250,
                "utilization": 25,
                "currency": "usd"
            }
        }))
        .unwrap();
        let windows = build_windows(&usage);
        assert!(windows.iter().any(|window| window.label == "5 小时额度"));
        assert!(windows.iter().any(|window| window.label == "7 天额度"));
        let extra = windows
            .iter()
            .find(|window| window.label == "Extra Usage")
            .unwrap();
        assert_eq!(extra.limit, Some(50.0));
        assert_eq!(extra.used_raw, Some(12.5));
        assert_eq!(extra.remaining, Some(37.5));
    }

    #[test]
    fn profile_refreshes_identity_and_plan_tier() {
        let profile = json!({
            "account": { "display_name": "Ada", "email": "ada@example.com" },
            "organization": {
                "rate_limit_tier": "default_claude_max_20x",
                "organization_type": "personal"
            }
        });
        assert_eq!(profile_label(&profile).as_deref(), Some("Ada"));
        assert_eq!(profile_plan_name(&profile).as_deref(), Some("Max 20X"));

        let provider = ClaudeProvider::new();
        assert_eq!(provider.plan_tier("Pro"), Some(2));
        assert_eq!(provider.plan_tier("Team Standard"), Some(2));
        assert_eq!(provider.plan_tier("Max 5X"), Some(4));
        assert_eq!(provider.plan_tier("Team Premium"), Some(4));
        assert_eq!(provider.plan_tier("Max 20X"), Some(5));
        assert_eq!(provider.plan_tier("Enterprise Contract"), None);
    }
}
