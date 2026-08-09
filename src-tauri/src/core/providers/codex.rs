//! OpenAI Codex（ChatGPT 订阅侧，OAuth，里程碑 M3）
//!
//! ✅ 2026-08-02 真实账号实测确认（本机 ~/.codex/auth.json 打 wham/usage，HTTP 200）：
//! - 认证：`Authorization: Bearer <access_token>` + `ChatGPT-Account-Id: <account_id>` 头
//! - 端点：`GET https://chatgpt.com/backend-api/wham/usage`
//! - 返回（全部实测字段）：
//!   - `plan_type` 套餐名（实测 "plus"）
//!   - `rate_limit.primary_window` 主窗口（used_percent 整数 + limit_window_seconds + reset_at）
//!   - `rate_limit.secondary_window` 次窗口 —— 可为 null（单窗口账号）
//!   - `code_review_rate_limit` 可为 null
//!   - `credits.balance` 字符串（实测 "0"），`has_credits`/`unlimited` 布尔
//!   - `spend_control` / `rate_limit_upsell`（升级 CTA）/ `rate_limit_reset_credits`
//! - 窗口语义：limit_window_seconds=18000 → 5h；=604800 → 7d。used_percent 为整数百分比。
//! - ⚠️ credits.balance 是字符串，需 trim + parse
//!
//! 本机凭证探测：`$CODEX_HOME/auth.json` / `~/.config/codex/auth.json` / `~/.codex/auth.json`。

use super::*;
use crate::core::oauth_codex;
use crate::core::providers::Brand;
use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::path::PathBuf;

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

pub struct CodexProvider;

impl CodexProvider {
    pub fn new() -> Self {
        Self
    }

    fn candidate_paths() -> Vec<PathBuf> {
        let home = crate::core::providers::home_dir();
        let codex_home = std::env::var("CODEX_HOME").map(PathBuf::from).ok();
        let mut v = vec![];
        if let Some(c) = codex_home {
            v.push(c.join("auth.json"));
        }
        v.push(home.join(".config/codex/auth.json"));
        v.push(home.join(".codex/auth.json"));
        v
    }
}

// ---------- wham/usage 响应结构 ----------

#[derive(Debug, Deserialize)]
struct WhamResp {
    #[serde(default)]
    plan_type: Option<String>,
    #[serde(default)]
    rate_limit: Option<RateLimit>,
    #[serde(default)]
    credits: Option<Credits>,
}

#[derive(Debug, Deserialize)]
struct RateLimit {
    #[serde(default)]
    primary_window: Option<WindowInfo>,
    #[serde(default)]
    secondary_window: Option<WindowInfo>,
}

#[derive(Debug, Deserialize)]
struct WindowInfo {
    #[serde(default)]
    used_percent: Option<f64>,
    #[serde(default)]
    limit_window_seconds: Option<i64>,
    #[serde(default)]
    reset_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct Credits {
    #[serde(default)]
    has_credits: Option<bool>,
    #[serde(default)]
    unlimited: Option<bool>,
    #[serde(default)]
    balance: Option<NumStr>,
}

/// 兼容数字 / 数字字符串（credits.balance 实测为字符串 "0"）。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum NumStr {
    N(f64),
    S(String),
}

impl NumStr {
    fn as_f64(&self) -> Option<f64> {
        match self {
            NumStr::N(n) if !n.is_nan() => Some(*n),
            NumStr::S(s) => s.trim().parse::<f64>().ok(),
            _ => None,
        }
    }
}

fn credit_balance(credits: Option<&Credits>) -> Option<Balance> {
    let credits = credits?;
    let unlimited = credits.unlimited.unwrap_or(false);
    let balance = credits
        .balance
        .as_ref()
        .and_then(NumStr::as_f64)
        .unwrap_or(0.0);

    // wham/usage 即使在余额为 0 时也可能返回 has_credits=false；
    // 只要 credits 对象存在，就保留这一项供前端明确显示 $0，而不是整行消失。
    Some(Balance {
        total: balance,
        granted: None,
        topped_up: Some(balance),
        currency: "USD".to_string(),
        available: unlimited || balance > 0.0,
    })
}

fn account_label_from_credential(cred: &Credential) -> Option<String> {
    let account_id = cred
        .data
        .get("account_id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty());

    let token_label = cred
        .data
        .get("id_token")
        .and_then(|value| value.as_str())
        .and_then(oauth_codex::extract_identity)
        .and_then(|identity| identity.account_label);
    if token_label.is_some() {
        return token_label;
    }

    // 兼容旧版已保存凭证：旧凭证没有 id_token/account_label。若它与本机
    // Codex CLI 当前账号相同，可从 CLI 的 id_token 补回用户名或邮箱。
    if let Some(account_id) = account_id {
        for path in CodexProvider::candidate_paths() {
            let Ok(source) = fs::read_to_string(path) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&source) else {
                continue;
            };
            let Some(identity) = value
                .get("tokens")
                .and_then(|tokens| tokens.get("id_token"))
                .and_then(|token| token.as_str())
                .and_then(oauth_codex::extract_identity)
            else {
                continue;
            };
            if identity.account_id == account_id && identity.account_label.is_some() {
                return identity.account_label;
            }
        }
    }

    cred.data
        .get("account_label")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| account_id.map(str::to_string))
}

/// 由 limit_window_seconds 推断窗口周期与中文标签。
fn window_meta(secs: i64) -> (WindowPeriod, &'static str) {
    match secs {
        s if s <= 5 * 3600 + 600 => (WindowPeriod::Hours5, "5 小时"),
        s if s <= 7 * 86_400 + 3600 => (WindowPeriod::Week, "本周"),
        s => (WindowPeriod::Custom(s), "自定义窗口"),
    }
}

fn to_window(w: &WindowInfo) -> Option<QuotaWindow> {
    let secs = w.limit_window_seconds?;
    let (period, label) = window_meta(secs);
    let pct = w.used_percent.map(|p| p.clamp(0.0, 100.0));
    Some(QuotaWindow {
        period,
        label: label.to_string(),
        used: pct,
        used_raw: None,
        limit: Some(100.0),
        remaining: pct.map(|p| (100.0 - p).max(0.0)),
        unit: QuotaUnit::Percent,
        reset_at: w.reset_at,
    })
}

#[async_trait]
impl Provider for CodexProvider {
    fn id(&self) -> &'static str {
        "codex"
    }
    fn display_name(&self) -> &'static str {
        "OpenAI Codex"
    }
    fn brand(&self) -> Brand {
        Brand::OpenAI
    }
    fn billing_mode(&self) -> BillingMode {
        BillingMode::Subscription
    }
    fn add_product_name(&self) -> &'static str {
        "Codex 套餐"
    }
    fn add_description(&self) -> &'static str {
        "ChatGPT 套餐额度"
    }
    fn detail_url(&self) -> Option<&'static str> {
        Some("https://chatgpt.com/codex/settings/usage")
    }
    fn card_config(&self) -> presentation::CardConfig {
        let mut config = presentation::CardConfig::for_billing(self.billing_mode());
        config.detail_url = self.detail_url();
        config.balance_role = presentation::BalanceRole::Supplemental {
            label: "Credit 余额",
        };
        config
    }
    fn plan_tier(&self, plan_name: &str) -> Option<u8> {
        let name = plan_name.to_lowercase();
        if name.contains("free") {
            Some(0)
        } else if name.contains("go") {
            Some(1)
        } else if name.contains("plus") || name.contains("business") || name.contains("team") {
            Some(2)
        } else if name.contains("pro 20x") {
            Some(5)
        } else if name.contains("pro 5x") || name.contains("prolite") {
            Some(4)
        } else {
            None
        }
    }
    fn auth_spec(&self) -> AuthSpec {
        AuthSpec::OAuth {
            // 新版设备码授权页（旧 authorize 页已废弃）
            authorize_url: "https://auth.openai.com/codex/device",
            token_url: TOKEN_URL,
            client_id: CLIENT_ID,
            scopes: &["codex"],
            pkce: true,
        }
    }

    async fn detect_local(&self) -> Option<Credential> {
        for p in Self::candidate_paths() {
            if let Ok(s) = fs::read_to_string(&p) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                    let tokens = v.get("tokens")?;
                    let access = tokens.get("access_token")?.as_str()?;
                    let identity = tokens
                        .get("id_token")
                        .and_then(|token| token.as_str())
                        .and_then(oauth_codex::extract_identity)
                        .unwrap_or_default();
                    let account_id = tokens
                        .get("account_id")
                        .and_then(|value| value.as_str())
                        .filter(|value| !value.is_empty())
                        .unwrap_or(&identity.account_id);
                    return Some(Credential {
                        data: json!({
                            "access_token": access,
                            "id_token": tokens.get("id_token").and_then(|x| x.as_str()).unwrap_or(""),
                            "account_id": account_id,
                            "refresh_token": tokens.get("refresh_token").and_then(|x| x.as_str()).unwrap_or(""),
                            "account_label": identity.account_label,
                        }),
                    });
                }
            }
        }
        None
    }

    /// 到期前刷新（refresh_token grant）。
    async fn refresh(&self, cred: &Credential) -> anyhow::Result<Option<Credential>> {
        let rt = cred
            .data
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        let Some(rt) = rt else {
            return Ok(None);
        };
        let previous_account_id = cred
            .data
            .get("account_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let previous_account_label = cred
            .data
            .get("account_label")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let previous_id_token = cred
            .data
            .get("id_token")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string();
        let client = super::http_client();
        let resp = client
            .post(TOKEN_URL)
            .form(&[
                ("client_id", CLIENT_ID),
                ("grant_type", "refresh_token"),
                ("refresh_token", rt),
            ])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Ok(None);
        }
        let v = resp.json::<serde_json::Value>().await?;
        let Some(at) = v.get("access_token").and_then(|x| x.as_str()) else {
            return Ok(None);
        };
        let new_rt = v
            .get("refresh_token")
            .and_then(|x| x.as_str())
            .unwrap_or(rt);
        let id_token = v
            .get("id_token")
            .and_then(|value| value.as_str())
            .unwrap_or(&previous_id_token);
        let refreshed_identity = oauth_codex::extract_identity(id_token).unwrap_or_default();
        let account_id = if refreshed_identity.account_id.is_empty() {
            previous_account_id
        } else {
            refreshed_identity.account_id
        };
        let account_label = refreshed_identity.account_label.or(previous_account_label);
        Ok(Some(Credential {
            data: json!({
                "access_token": at,
                "id_token": id_token,
                "account_id": account_id,
                "refresh_token": new_rt,
                "account_label": account_label,
            }),
        }))
    }

    async fn fetch(&self, cred: &Credential) -> anyhow::Result<ProviderSnapshot> {
        let access = cred
            .data
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 access_token"))?;
        let account_id = cred
            .data
            .get("account_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let client = super::http_client();
        let resp = client
            .get(USAGE_URL)
            .bearer_auth(access)
            .header("ChatGPT-Account-Id", account_id)
            .header("Accept", "application/json")
            .send()
            .await?;

        if resp.status().as_u16() == 401 || resp.status().as_u16() == 403 {
            return Ok(ProviderSnapshot {
                account_id: String::new(),
                account_label: None,
                provider_id: self.id().to_string(),
                display_name: self.display_name().to_string(),
                plan_name: None,
                billing: BillingMode::Subscription,
                balance: None,
                windows: vec![],
                fidelity: Fidelity::Exact,
                status: HealthStatus::AuthExpired,
                fetched_at: Utc::now().timestamp(),
                last_error: None,
            });
        }

        let body = resp.error_for_status()?.json::<WhamResp>().await?;

        // 套餐徽章：把接口内部名标准化为用户可读名称
        // （如 prolite / pro_5x → Pro 5X）。
        // 只有返回明确 5x / 20x 时，前端才按对应价格色阶显示。
        let plan_name = body.plan_type.as_deref().map(display_plan_name);

        // 额度窗口：primary + secondary（可为 null 则跳过）
        let mut windows: Vec<QuotaWindow> = Vec::new();
        if let Some(rl) = &body.rate_limit {
            if let Some(pw) = &rl.primary_window {
                if let Some(w) = to_window(pw) {
                    windows.push(w);
                }
            }
            if let Some(sw) = &rl.secondary_window {
                if let Some(w) = to_window(sw) {
                    windows.push(w);
                }
            }
        }

        // credits → 余额型展示（USD）
        let balance = credit_balance(body.credits.as_ref());

        Ok(ProviderSnapshot {
            account_id: String::new(),
            account_label: account_label_from_credential(cred),
            provider_id: self.id().to_string(),
            display_name: self.display_name().to_string(),
            plan_name,
            billing: BillingMode::Subscription,
            balance,
            windows,
            fidelity: Fidelity::Exact,
            status: HealthStatus::Ok,
            fetched_at: Utc::now().timestamp(),
            last_error: None,
        })
    }
}

fn display_plan_name(plan_type: &str) -> String {
    let normalized = plan_type
        .replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    match normalized.as_str() {
        // wham/usage 对 99 美元档可能返回内部名 prolite；产品名称统一显示 Pro 5X。
        "prolite" | "pro lite" | "pro 5x" => "Pro 5X".to_string(),
        "pro 20x" => "Pro 20X".to_string(),
        _ => normalized
            .split_whitespace()
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

#[cfg(test)]
mod tests {
    use super::{credit_balance, display_plan_name, CodexProvider, Credits, NumStr};
    use crate::core::providers::Provider;

    #[test]
    fn preserves_codex_pro_tier_suffixes() {
        assert_eq!(display_plan_name("prolite"), "Pro 5X");
        assert_eq!(display_plan_name("pro_lite"), "Pro 5X");
        assert_eq!(display_plan_name("pro_5x"), "Pro 5X");
        assert_eq!(display_plan_name("pro-20x"), "Pro 20X");
        let provider = CodexProvider::new();
        assert_eq!(provider.plan_tier("Pro 5X"), Some(4));
        assert_eq!(provider.plan_tier("Pro 20X"), Some(5));
    }

    #[test]
    fn keeps_zero_credit_balance_visible() {
        let credits = Credits {
            has_credits: Some(false),
            unlimited: Some(false),
            balance: Some(NumStr::S("0".to_string())),
        };

        let balance = credit_balance(Some(&credits)).expect("credits object should stay visible");
        assert_eq!(balance.total, 0.0);
        assert!(!balance.available);
    }
}
