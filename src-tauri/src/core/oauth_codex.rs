//! OpenAI Codex 设备码授权（2026-08-03 按官方 openai/codex 源码逆向）。
//!
//! 新版 Codex CLI 已弃用旧的浏览器 OAuth authorize 流程，改为设备码：
//!   1. `POST {issuer}/api/accounts/deviceauth/usercode` 请求设备码
//!      （关键请求头 `originator: codex_cli_rs`，缺了会被 Cloudflare 拦截）
//!   2. 打开 `{issuer}/codex/device` 让用户输入 user_code 完成授权
//!   3. 轮询 `POST {issuer}/api/accounts/deviceauth/token`（403/404 表示未完成）
//!   4. 用返回的 authorization_code + code_verifier 在 `{issuer}/oauth/token` 换 token
//! 端点/字段来源：github.com/openai/codex codex-rs/login/src/device_code_auth.rs

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use std::time::{Duration, Instant};

const ISSUER: &str = "https://auth.openai.com";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const ORIGINATOR: &str = "codex_cli_rs";
/// 用户完成授权的页面（输入 user_code）
pub const VERIFY_URL: &str = "https://auth.openai.com/codex/device";
/// 轮询最长等待时间（官方 15 分钟）
const POLL_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// 设备码第一步返回（给前端展示 user_code + 打开 verify_url）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct CodexDeviceStart {
    pub user_code: String,
    pub verify_url: String,
    pub device_auth_id: String,
    pub interval_secs: u64,
}

#[derive(Debug, Deserialize)]
struct UserCodeResp {
    device_auth_id: String,
    #[serde(alias = "user_code", alias = "usercode")]
    user_code: String,
    #[serde(default)]
    interval: IntervalStr,
}

/// 兼容数字 / 数字字符串（实测 interval 是字符串 "5"）。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum IntervalStr {
    N(u64),
    S(String),
}

impl Default for IntervalStr {
    fn default() -> Self {
        IntervalStr::N(5)
    }
}

impl IntervalStr {
    fn as_u64(&self) -> u64 {
        match self {
            IntervalStr::N(n) => *n,
            IntervalStr::S(s) => s.trim().parse().unwrap_or(5),
        }
    }
}

/// 轮询成功后返回的授权码 + 服务端生成的 PKCE 参数。
#[derive(Debug, Deserialize)]
struct CodeSuccessResp {
    authorization_code: String,
    #[allow(dead_code)] // 服务端返回，协议完整性保留；换 token 只用 code_verifier
    code_challenge: String,
    code_verifier: String,
}

/// 第一步：请求设备码。
pub async fn start() -> Result<CodexDeviceStart> {
    let client = crate::core::providers::http_client();
    let resp = client
        .post(format!("{ISSUER}/api/accounts/deviceauth/usercode"))
        .header("Content-Type", "application/json")
        .header("originator", ORIGINATOR)
        .header("User-Agent", format!("{ORIGINATOR}/0.55.0 (TokenMeter)"))
        .json(&json!({ "client_id": CLIENT_ID }))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("请求 Codex 设备码失败 {status}: {body}"));
    }
    let d: UserCodeResp = resp.json().await?;
    Ok(CodexDeviceStart {
        user_code: d.user_code,
        verify_url: VERIFY_URL.to_string(),
        device_auth_id: d.device_auth_id,
        interval_secs: d.interval.as_u64().max(3),
    })
}

/// 第二步：轮询直到用户授权，换 token，返回凭证 JSON。
pub async fn poll_until_authorized(
    device_auth_id: &str,
    user_code: &str,
    interval_secs: u64,
) -> Result<serde_json::Value> {
    let client = crate::core::providers::http_client();
    let url = format!("{ISSUER}/api/accounts/deviceauth/token");
    let deadline = Instant::now() + POLL_TIMEOUT;

    loop {
        if Instant::now() >= deadline {
            return Err(anyhow!("Codex 授权超时（15 分钟未完成）"));
        }
        tokio::time::sleep(Duration::from_secs(interval_secs.max(3))).await;

        let resp = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("originator", ORIGINATOR)
            .header("User-Agent", format!("{ORIGINATOR}/0.55.0 (TokenMeter)"))
            .json(&json!({
                "device_auth_id": device_auth_id,
                "user_code": user_code,
            }))
            .send()
            .await?;

        let status = resp.status();
        if status.is_success() {
            let code: CodeSuccessResp = resp.json().await?;
            return exchange_tokens(&code).await;
        }
        if status.as_u16() == 403 || status.as_u16() == 404 {
            // 用户还没完成授权，继续轮询
            continue;
        }
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Codex 设备码轮询失败 HTTP {status}: {body}"));
    }
}

/// 用 authorization_code + code_verifier 换 access/refresh token。
async fn exchange_tokens(code: &CodeSuccessResp) -> Result<serde_json::Value> {
    let client = crate::core::providers::http_client();
    let redirect_uri = format!("{ISSUER}/deviceauth/callback");
    let body = format!(
        "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}",
        urlencode(&code.authorization_code),
        urlencode(&redirect_uri),
        urlencode(CLIENT_ID),
        urlencode(&code.code_verifier),
    );
    let resp = client
        .post(format!("{ISSUER}/oauth/token"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("originator", ORIGINATOR)
        .body(body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("Codex 换 token 失败 HTTP {status}: {body}"));
    }
    let v: serde_json::Value = resp.json().await?;
    let access = v
        .get("access_token")
        .and_then(|x| x.as_str())
        .ok_or_else(|| anyhow!("换 token 响应缺少 access_token"))?;
    let account_id = v
        .get("id_token")
        .and_then(|x| x.as_str())
        .and_then(extract_account_id)
        .unwrap_or_default();

    Ok(json!({
        "access_token": access,
        "refresh_token": v.get("refresh_token").and_then(|x| x.as_str()).unwrap_or(""),
        "account_id": account_id,
    }))
}

/// 从 id_token (JWT) 的 payload 解出 chatgpt_account_id。
fn extract_account_id(id_token: &str) -> Option<String> {
    let payload_b64 = id_token.split('.').nth(1)?;
    let payload = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&payload).ok()?;
    v.get("chatgpt_account_id")
        .or_else(|| v.get("https://api.openai.com/auth").and_then(|a| a.get("chatgpt_account_id")))
        .or_else(|| v.get("account_id"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
}

/// 极简 RFC3986 百分号编码（表单参数用）。
fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// 保留 Utc 引用避免误删（fetched_at 等时间字段在其他模块处理）
#[allow(dead_code)]
fn _keep_chrono() -> i64 {
    Utc::now().timestamp()
}
