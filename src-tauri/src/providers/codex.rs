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
use crate::providers::Brand;
use async_trait::async_trait;
use chrono::Utc;
use reqwest::Client;
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
        let home = crate::providers::home_dir();
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
    fn auth_spec(&self) -> AuthSpec {
        AuthSpec::OAuth {
            authorize_url: "https://auth.openai.com/oauth/authorize",
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
                    let account_id = tokens.get("account_id").and_then(|x| x.as_str()).unwrap_or("");
                    return Some(Credential {
                        data: json!({
                            "access_token": access,
                            "account_id": account_id,
                            "refresh_token": tokens.get("refresh_token").and_then(|x| x.as_str()).unwrap_or(""),
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
        let account_id = cred
            .data
            .get("account_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let client = Client::new();
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
        let new_rt = v.get("refresh_token").and_then(|x| x.as_str()).unwrap_or(rt);
        Ok(Some(Credential {
            data: json!({
                "access_token": at,
                "account_id": account_id,
                "refresh_token": new_rt,
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

        let client = Client::new();
        let resp = client
            .get(USAGE_URL)
            .bearer_auth(access)
            .header("ChatGPT-Account-Id", account_id)
            .header("Accept", "application/json")
            .send()
            .await?;

        if resp.status().as_u16() == 401 || resp.status().as_u16() == 403 {
            return Ok(ProviderSnapshot {
                provider_id: self.id().to_string(),
                display_name: self.display_name().to_string(),
                plan_name: None,
                billing: BillingMode::Subscription,
                balance: None,
                windows: vec![],
                fidelity: Fidelity::Exact,
                status: HealthStatus::AuthExpired,
                fetched_at: Utc::now().timestamp(),
            });
        }

        let body = resp.error_for_status()?.json::<WhamResp>().await?;

        // 套餐徽章：plan_type（如 "plus" → "Plus"）
        let plan_name = body.plan_type.as_deref().map(|p| {
            let mut c = p.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => p.to_string(),
            }
        });

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
        let balance = body.credits.as_ref().and_then(|c| {
            let unlimited = c.unlimited.unwrap_or(false);
            let bal = c.balance.as_ref().and_then(|b| b.as_f64()).unwrap_or(0.0);
            let has = c.has_credits.unwrap_or(false);
            if !has && !unlimited {
                return None;
            }
            Some(Balance {
                total: bal,
                granted: None,
                topped_up: Some(bal),
                currency: "USD".to_string(),
                available: unlimited || bal > 0.0,
            })
        });

        Ok(ProviderSnapshot {
            provider_id: self.id().to_string(),
            display_name: self.display_name().to_string(),
            plan_name,
            billing: BillingMode::Subscription,
            balance,
            windows,
            fidelity: Fidelity::Exact,
            status: HealthStatus::Ok,
            fetched_at: Utc::now().timestamp(),
        })
    }
}
