//! Gemini Code Assist 的 Google OAuth loopback 授权。
//!
//! OAuth 客户端与 scopes 取自 Google 官方 Gemini CLI。桌面客户端 secret 按
//! Google installed-app 约定是可公开嵌入值，不作为用户凭证处理。

use crate::core::providers::{self, Credential};
use anyhow::{anyhow, Context};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub const CLIENT_ID: &str =
    "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com";
pub const CLIENT_SECRET: &str = "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";
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const SCOPES: &str = "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile";
const SESSION_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Serialize)]
pub struct GoogleOAuthStart {
    pub session_id: String,
    pub authorize_url: String,
}

struct GoogleSession {
    verifier: String,
    state: String,
    redirect_uri: String,
    listener: TcpListener,
    created_at: Instant,
}

static PENDING: OnceLock<Mutex<HashMap<String, GoogleSession>>> = OnceLock::new();

fn pending() -> &'static Mutex<HashMap<String, GoogleSession>> {
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock() -> anyhow::Result<MutexGuard<'static, HashMap<String, GoogleSession>>> {
    pending()
        .lock()
        .map_err(|_| anyhow!("Google OAuth 会话锁已损坏"))
}

fn random_urlsafe(byte_len: usize) -> String {
    let mut bytes = vec![0u8; byte_len];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub async fn start() -> anyhow::Result<GoogleOAuthStart> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("无法启动 Gemini 本地 OAuth 回调")?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/oauth2callback");
    let verifier = random_urlsafe(32);
    let state = random_urlsafe(24);
    let id = random_urlsafe(18);
    let mut url = reqwest::Url::parse(AUTHORIZE_URL)?;
    url.query_pairs_mut()
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPES)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent")
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge(&verifier))
        .append_pair("code_challenge_method", "S256");
    let mut sessions = lock()?;
    sessions.retain(|_, session| session.created_at.elapsed() < SESSION_TTL);
    sessions.insert(
        id.clone(),
        GoogleSession {
            verifier,
            state,
            redirect_uri,
            listener,
            created_at: Instant::now(),
        },
    );
    Ok(GoogleOAuthStart {
        session_id: id,
        authorize_url: url.to_string(),
    })
}

async fn callback(session: GoogleSession) -> anyhow::Result<String> {
    let (mut stream, _) = tokio::time::timeout(Duration::from_secs(240), session.listener.accept())
        .await
        .map_err(|_| anyhow!("Gemini 浏览器授权等待超时，请重试"))??;
    let mut buffer = vec![0u8; 16 * 1024];
    let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer))
        .await
        .map_err(|_| anyhow!("Gemini OAuth 回调读取超时"))??;
    let request = std::str::from_utf8(&buffer[..read]).context("Gemini 回调不是有效 HTTP")?;
    let target = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| anyhow!("Gemini OAuth 回调格式无效"))?;
    let url = reqwest::Url::parse(&format!("http://127.0.0.1{target}"))?;
    let params = url.query_pairs().collect::<HashMap<_, _>>();
    if let Some(error) = params.get("error") {
        write_callback(&mut stream, false).await;
        anyhow::bail!("Gemini 授权被拒绝：{error}");
    }
    let returned_state = params
        .get("state")
        .map(|value| value.as_ref())
        .ok_or_else(|| anyhow!("Gemini OAuth 回调缺少 state"))?;
    if returned_state != session.state {
        write_callback(&mut stream, false).await;
        anyhow::bail!("Gemini OAuth state 不匹配，请重新授权");
    }
    let code = params
        .get("code")
        .map(|value| value.to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("Gemini OAuth 回调缺少授权码"))?;
    write_callback(&mut stream, true).await;
    Ok(code)
}

async fn write_callback(stream: &mut tokio::net::TcpStream, success: bool) {
    let (title, body) = if success {
        (
            "授权成功",
            "已连接 Gemini，可以关闭此页面并返回 TokenMeter。",
        )
    } else {
        ("授权失败", "Gemini 授权未完成，请返回 TokenMeter 重试。")
    };
    let html = format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width'><title>{title}</title><style>body{{font-family:system-ui,-apple-system,sans-serif;display:grid;place-items:center;min-height:90vh;background:#f4efe6;color:#25211d}}main{{padding:32px;text-align:center}}h1{{font-size:22px}}p{{color:#746d64}}</style><main><h1>{title}</h1><p>{body}</p></main>"
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

pub async fn complete(session_id: &str) -> anyhow::Result<Credential> {
    let session = {
        let mut sessions = lock()?;
        let session = sessions
            .remove(session_id)
            .ok_or_else(|| anyhow!("Gemini OAuth 会话不存在或已过期，请重新授权"))?;
        if session.created_at.elapsed() >= SESSION_TTL {
            anyhow::bail!("Gemini OAuth 会话已过期，请重新授权");
        }
        session
    };
    let verifier = session.verifier.clone();
    let redirect_uri = session.redirect_uri.clone();
    let code = callback(session).await?;
    let response = providers::http_client()
        .post(TOKEN_URL)
        .form(&[
            ("client_id", CLIENT_ID),
            ("client_secret", CLIENT_SECRET),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri.as_str()),
        ])
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        anyhow::bail!(
            "Gemini 授权码交换失败 HTTP {}：{}",
            status.as_u16(),
            body.trim()
        );
    }
    let value = serde_json::from_str::<Value>(&body)?;
    let access_token = value
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("Gemini 授权响应缺少 access_token"))?;
    let refresh_token = value
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("Gemini 授权响应缺少 refresh_token"))?;
    let expires_in = value
        .get("expires_in")
        .and_then(Value::as_i64)
        .unwrap_or(3600);
    Ok(Credential {
        data: serde_json::json!({
            "access_token": access_token,
            "refresh_token": refresh_token,
            "expiry_date": chrono::Utc::now().timestamp_millis() + expires_in * 1000,
            "scope": value.get("scope").and_then(Value::as_str).unwrap_or(SCOPES),
            "token_type": value.get("token_type").and_then(Value::as_str).unwrap_or("Bearer"),
            "source_kind": "browser_oauth",
        }),
    })
}
