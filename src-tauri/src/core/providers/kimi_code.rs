//! Kimi Code（coding plan 订阅侧，OAuth，里程碑 M3）
//!
//! ✅ 2026-08-02 真实账号实测确认（设备码 OAuth 打 api.kimi.com/coding/v1/usages）：
//! - 认证：设备码流程（auth.kimi.com），CLIENT_ID = 17e5f671-d194-4dfb-9706-5516cb48c098
//! - 抓取：`GET /coding/v1/usages`，Bearer token，scope=FEATURE_CODING
//! - 返回（全部实测字段）：
//!   - `usage`     本周额度（limit/used/remaining/resetTime，Weekly，7 天）
//!   - `limits[]`  5 小时滚动窗口（window.duration=300min + detail.limit/used/remaining/resetTime）
//!   - `totalQuota` 总订阅积分池 —— 条件性存在（实测账号为空 {}），前端按字段存在性渲染
//!   - `boosterWallet` Extra Usage 货币钱包（monthlyUsed/monthlyChargeLimit，proto Money 字符串分）
//!   - `user.membership.level` 会员档位 slug（实测 LEVEL_ADVANCED / LEVEL_INTERMEDIATE）
//!   - `user.region` 区域（实测 REGION_CN）—— 与 level 独立，佐证统一 level + 区域屏蔽
//! - ⚠️ 所有数值字段是字符串（"100"/"10000000000"），解析须 string→number
//! - 月限额/续费时间走另一套 MembershipService（web 会话，coding token 401）→ 不采集
//!
//! 本机凭证探测：读 `~/.kimi/credentials/kimi-code.json`。

use super::*;
use crate::core::providers::Brand;
use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use std::fs;

const USAGES_URL: &str = "https://api.kimi.com/coding/v1/usages";
const TOKEN_URL: &str = "https://auth.kimi.com/api/oauth/token";
const CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";

pub struct KimiCodeProvider;

impl KimiCodeProvider {
    pub fn new() -> Self {
        Self
    }
}

// ---------- /usages 响应结构（字段名有版本漂移，用别名兼容） ----------

#[derive(Debug, Deserialize)]
struct UsagesResp {
    #[serde(default)]
    usage: Option<QuotaDetail>,
    #[serde(default)]
    limits: Vec<LimitEntry>,
    #[serde(default)]
    user: Option<UserInfo>,
    #[serde(default, rename = "boosterWallet")]
    booster_wallet: Option<BoosterWallet>,
}

#[derive(Debug, Deserialize)]
struct QuotaDetail {
    #[serde(default)]
    limit: Option<NumStr>,
    #[serde(default)]
    used: Option<NumStr>,
    #[serde(default)]
    remaining: Option<NumStr>,
    #[serde(default, rename = "resetTime", alias = "reset_at")]
    reset_time: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LimitEntry {
    #[serde(default)]
    window: Option<Window>,
    #[serde(default)]
    detail: Option<QuotaDetail>,
}

#[derive(Debug, Deserialize)]
struct Window {
    #[serde(default)]
    duration: Option<i64>,
    #[serde(default, rename = "timeUnit")]
    time_unit: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserInfo {
    #[serde(default)]
    membership: Option<Membership>,
}

#[derive(Debug, Deserialize)]
struct Membership {
    #[serde(default)]
    level: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BoosterWallet {
    #[serde(default, rename = "monthlyChargeLimit")]
    monthly_charge_limit: Option<Money>,
    #[serde(default, rename = "monthlyUsed")]
    monthly_used: Option<Money>,
}

#[derive(Debug, Deserialize)]
struct Money {
    #[serde(default, rename = "priceInCents")]
    price_in_cents: Option<NumStr>,
    #[serde(default)]
    currency: Option<String>,
}

/// 兼容数字 / 数字字符串两种形态（实测均为字符串，但保留数字兜底）。
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

/// 解析 ISO8601 / RFC3339 时间为 Unix 秒；失败返回 None。
fn parse_reset(s: &Option<String>) -> Option<i64> {
    s.as_deref()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|dt| dt.timestamp())
}

/// 由 limit/used/remaining 三元组构造 QuotaWindow。
fn make_window(period: WindowPeriod, label: &str, unit: QuotaUnit, d: &QuotaDetail) -> Option<QuotaWindow> {
    let limit = d.limit.as_ref().and_then(|x| x.as_f64());
    let used = d
        .used
        .as_ref()
        .and_then(|x| x.as_f64())
        .or_else(|| match (limit, d.remaining.as_ref().and_then(|x| x.as_f64())) {
            (Some(l), Some(r)) => Some((l - r).max(0.0)),
            _ => None,
        });
    let remaining = d.remaining.as_ref().and_then(|x| x.as_f64());
    // 三层全空视为无此窗口
    if limit.is_none() && used.is_none() && remaining.is_none() {
        return None;
    }
    let used_pct = match (used, limit) {
        (Some(u), Some(l)) if l > 0.0 => Some((u / l * 100.0).min(100.0)),
        _ => None,
    };
    Some(QuotaWindow {
        period,
        label: label.to_string(),
        used: used_pct,
        used_raw: used,
        limit,
        remaining,
        unit,
        reset_at: parse_reset(&d.reset_time),
    })
}

#[async_trait]
impl Provider for KimiCodeProvider {
    fn id(&self) -> &'static str {
        "kimi_code"
    }
    fn display_name(&self) -> &'static str {
        "Kimi Code"
    }
    fn brand(&self) -> Brand {
        Brand::Kimi
    }
    fn billing_mode(&self) -> BillingMode {
        BillingMode::Subscription
    }
    fn auth_spec(&self) -> AuthSpec {
        AuthSpec::OAuth {
            authorize_url: "https://www.kimi.com/code/authorize_device",
            token_url: TOKEN_URL,
            client_id: CLIENT_ID,
            scopes: &["kimi-code"],
            pkce: false, // 设备码流程无 PKCE
        }
    }

    async fn detect_local(&self) -> Option<Credential> {
        let p = crate::core::providers::home_dir().join(".kimi/credentials/kimi-code.json");
        let s = fs::read_to_string(p).ok()?;
        let v = serde_json::from_str::<serde_json::Value>(&s).ok()?;
        let access = v.get("access_token")?.as_str()?;
        let refresh = v.get("refresh_token").and_then(|x| x.as_str()).unwrap_or("");
        Some(Credential {
            data: json!({
                "access_token": access,
                "refresh_token": refresh,
            }),
        })
    }

    /// 到期前刷新 access_token（refresh_token grant）。
    async fn refresh(&self, cred: &Credential) -> anyhow::Result<Option<Credential>> {
        let refresh_token = cred
            .data
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        let Some(rt) = refresh_token else {
            return Ok(None);
        };
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
            return Ok(None); // 刷新失败交由上层走重新授权
        }
        let v = resp.json::<serde_json::Value>().await?;
        let new_access = v.get("access_token").and_then(|x| x.as_str());
        let Some(at) = new_access else {
            return Ok(None);
        };
        let new_rt = v
            .get("refresh_token")
            .and_then(|x| x.as_str())
            .unwrap_or(rt);
        Ok(Some(Credential {
            data: json!({
                "access_token": at,
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

        let client = super::http_client();
        let resp = client
            .get(USAGES_URL)
            .bearer_auth(access)
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
                last_error: None,
            });
        }

        let body = resp.error_for_status()?.json::<UsagesResp>().await?;

        // ---- 套餐徽章：membership.level slug → 显示名 ----
        let plan_name = body
            .user
            .as_ref()
            .and_then(|u| u.membership.as_ref())
            .and_then(|m| m.level.as_deref())
            .map(kimi_tier_name);

        // ---- 额度窗口：5h（limits[]）+ 本周（usage）；totalQuota 条件层暂略（实测为空）----
        let mut windows: Vec<QuotaWindow> = Vec::new();

        // 5 小时滚动窗：window.duration=300 + timeUnit=MINUTE
        for entry in &body.limits {
            let is_5h = entry
                .window
                .as_ref()
                .map(|w| {
                    w.duration == Some(300)
                        && w.time_unit
                            .as_deref()
                            .map(|u| u.contains("MINUTE"))
                            .unwrap_or(false)
                })
                .unwrap_or(false);
            if is_5h {
                if let Some(d) = &entry.detail {
                    if let Some(w) = make_window(WindowPeriod::Hours5, "5 小时", QuotaUnit::Percent, d) {
                        windows.push(w);
                    }
                }
            }
        }

        // 本周（usage，Weekly）
        if let Some(u) = &body.usage {
            if let Some(w) = make_window(WindowPeriod::Week, "本周", QuotaUnit::Percent, u) {
                windows.push(w);
            }
        }

        // ---- Extra Usage 货币钱包 → 货币单位额度条（不是余额 hero！）----
        // 误区修正（2026-08-02 用户反馈）：monthlyChargeLimit 是"月限额"，
        // 若放 Balance.total 会被前端当成"你有的钱"（显示 100）。实际是
        // "本月 Extra Usage 限额 100、已用 monthlyUsed"，应以额度条呈现
        // （used/limit，货币单位），与 5h/周窗口同一视觉语言。
        if let Some(bw) = &body.booster_wallet {
            let limit_cents = bw
                .monthly_charge_limit
                .as_ref()
                .and_then(|m| m.price_in_cents.as_ref())
                .and_then(|x| x.as_f64());
            let used = bw
                .monthly_used
                .as_ref()
                .and_then(|m| m.price_in_cents.as_ref())
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0);
            let currency = bw
                .monthly_charge_limit
                .as_ref()
                .and_then(|m| m.currency.clone())
                .unwrap_or_else(|| "CNY".to_string());
            let limit_yuan = limit_cents.map(|c| c / 100.0);
            let used_yuan = used / 100.0;
            let used_pct = match limit_yuan {
                Some(l) if l > 0.0 => Some((used_yuan / l * 100.0).min(100.0)),
                _ => None,
            };
            if limit_yuan.is_some() || used_yuan > 0.0 {
                windows.push(QuotaWindow {
                    period: WindowPeriod::Month,
                    label: "Extra Usage".to_string(),
                    used: used_pct,
                    used_raw: Some(used_yuan),
                    limit: limit_yuan,
                    remaining: limit_yuan.map(|l| (l - used_yuan).max(0.0)),
                    unit: QuotaUnit::Currency(currency),
                    reset_at: None,
                });
            }
        }

        Ok(ProviderSnapshot {
            provider_id: self.id().to_string(),
            display_name: self.display_name().to_string(),
            plan_name,
            billing: BillingMode::Subscription,
            balance: None, // Kimi 无"账户余额"概念，boosterWallet 已转为额度条
            windows,
            fidelity: Fidelity::Exact,
            status: HealthStatus::Ok,
            fetched_at: Utc::now().timestamp(),
            last_error: None,
        })
    }
}

/// membership.level slug → 显示名（单表，不按区域分支）。
/// 实测锚点：LEVEL_ADVANCED、LEVEL_INTERMEDIATE；其余为推断，待真实凭证校准。
fn kimi_tier_name(level: &str) -> String {
    match level {
        "LEVEL_BASIC" => "Andante".to_string(),
        "LEVEL_INTERMEDIATE" => "Moderato".to_string(),
        "LEVEL_ADVANCED" => "Allegretto".to_string(),
        "LEVEL_PRO" => "Allegro".to_string(),
        other => format!("Kimi 会员 {other}"),
    }
}
