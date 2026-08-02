//! Codex OAuth 2.0 + PKCE 授权码流程（浏览器授权）。
//!
//! 流程（参数经多来源交叉验证 + 官方 oauth.rs 实现核对）：
//!   1. 本地起 1455 端口临时 HTTP 服务器（OpenAI 注册的固定回调端口）
//!   2. 生成 PKCE code_verifier + code_challenge(S256) + state(CSRF)
//!   3. 构造授权 URL 并打开浏览器
//!   4. 用户登录后浏览器重定向回 localhost:1455/auth/callback?code=...&state=...
//!   5. 校验 state，用 code + code_verifier 换 token
//!   6. 从 id_token (JWT) 解出 chatgpt_account_id
//!   7. 存凭证（access_token / refresh_token / account_id）
//! 回调端口固定 1455（Codex CLI 同款，OpenAI 已注册）。

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const REDIRECT_PORT: u16 = 1455;
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const SCOPE: &str = "openid profile email offline_access";

/// 生成 PKCE code_verifier（86 字符 URL-safe 随机串）。
fn gen_verifier() -> String {
    let mut bytes = [0u8; 64];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// code_challenge = base64url-no-pad(SHA256(verifier))。
fn challenge_of(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

/// 构造授权 URL。
fn authorize_url(challenge: &str, state: &str) -> String {
    format!(
        "{AUTH}?response_type=code&client_id={cid}&redirect_uri={ru}&scope={sc}\
         &code_challenge={cc}&code_challenge_method=S256&state={st}\
         &id_token_add_organizations=true&codex_cli_simplified_flow=true&originator=codex_cli_rs",
        AUTH = AUTHORIZE_URL,
        cid = CLIENT_ID,
        ru = urlencoding(REDIRECT_URI),
        sc = urlencoding(SCOPE),
        cc = challenge,
        st = state,
    )
}

/// 极简 URL 编码（仅需处理 : / 空格等少数字符）。
fn urlencoding(s: &str) -> String {
    s.replace(':', "%3A")
        .replace('/', "%2F")
        .replace(' ', "%20")
}

/// 从 id_token (JWT) 的 payload 解出 chatgpt_account_id。
fn extract_account_id(id_token: &str) -> Option<String> {
    let payload_b64 = id_token.split('.').nth(1)?;
    let payload = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&payload).ok()?;
    // OpenAI 把 account_id 放在自定义 claim 里（多个可能位置，逐一试）
    v.get("chatgpt_account_id")
        .or_else(|| v.get("https://api.openai.com/auth").and_then(|a| a.get("chatgpt_account_id")))
        .or_else(|| v.get("account_id"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
}

#[derive(Debug, Deserialize)]
struct TokenResp {
    access_token: Option<String>,
    refresh_token: Option<String>,
    id_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// 在 1455 端口监听一次回调，返回 (code, state)。
async fn wait_for_callback(listener: TcpListener) -> Result<(String, String)> {
    // 10 分钟超时
    let accept = tokio::time::timeout(std::time::Duration::from_secs(600), listener.accept())
        .await
        .map_err(|_| anyhow!("等待授权回调超时（10 分钟）"))??;

    let (mut socket, _) = accept;
    let mut buf = vec![0u8; 4096];
    let n = socket.read(&mut buf).await?;
    let req = String::from_utf8_lossy(&buf[..n]);

    // 解析请求行：GET /auth/callback?code=...&state=... HTTP/1.1
    let line = req.lines().next().unwrap_or("");
    let path = line.split_whitespace().nth(1).unwrap_or("");
    let (mut code, mut state) = (String::new(), String::new());
    if let Some(q) = path.split('?').nth(1) {
        for pair in q.split('&') {
            let mut kv = pair.splitn(2, '=');
            match (kv.next(), kv.next()) {
                (Some("code"), Some(v)) => code = v.to_string(),
                (Some("state"), Some(v)) => state = v.to_string(),
                _ => {}
            }
        }
    }

    // 回一个友好页面让用户知道可以关闭了
    let ok = !code.is_empty();
    let body = if ok {
        "<html><body style='font-family:sans-serif;text-align:center;padding:60px'>\
         <h2>✅ 授权成功</h2><p>可以关闭此页面，回到 TokenMeter。</p></body></html>"
    } else {
        "<html><body style='font-family:sans-serif;text-align:center;padding:60px'>\
         <h2>⚠️ 授权失败</h2><p>未收到授权码，请重试。</p></body></html>"
    };
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = socket.write_all(resp.as_bytes()).await;
    let _ = socket.shutdown().await;

    if code.is_empty() {
        return Err(anyhow!("回调中未收到授权码"));
    }
    Ok((code, state))
}

/// 完整流程：打开浏览器 → 等回调 → 换 token → 返回凭证 JSON。
/// `open_browser` 由调用方注入（前端 plugin-shell 的 open）。
pub async fn run_flow<F, Fut>(open_browser: F) -> Result<serde_json::Value>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let verifier = gen_verifier();
    let challenge = challenge_of(&verifier);
    let mut state_bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut state_bytes);
    let state = URL_SAFE_NO_PAD.encode(state_bytes);

    let url = authorize_url(&challenge, &state);

    // 先绑定端口再开浏览器：绑定失败立即报错（不开浏览器、不空等 10 分钟），
    // 也消除了"回调先于 bind 到达"的竞态。
    let listener = TcpListener::bind(("127.0.0.1", REDIRECT_PORT))
        .await
        .map_err(|e| anyhow!("无法监听回调端口 {REDIRECT_PORT}（可能已被 Codex CLI 占用）: {e}"))?;
    let callback_task = tokio::spawn(wait_for_callback(listener));
    open_browser(url).await;

    let (code, cb_state) = callback_task
        .await
        .map_err(|e| anyhow!("回调任务失败: {e}"))??;

    if cb_state != state {
        return Err(anyhow!("state 校验失败（可能的 CSRF，已中止）"));
    }

    // 换 token
    let client = crate::core::providers::http_client();
    let resp = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", code.as_str()),
            ("redirect_uri", REDIRECT_URI),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await?
        .json::<TokenResp>()
        .await?;

    let Some(access_token) = resp.access_token else {
        return Err(anyhow!(
            "换 token 失败: {} {}",
            resp.error.unwrap_or_default(),
            resp.error_description.unwrap_or_default()
        ));
    };
    let account_id = resp
        .id_token
        .as_deref()
        .and_then(extract_account_id)
        .unwrap_or_default();

    Ok(json!({
        "access_token": access_token,
        "refresh_token": resp.refresh_token.unwrap_or_default(),
        "account_id": account_id,
    }))
}
