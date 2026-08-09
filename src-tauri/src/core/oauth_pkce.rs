//! Claude 与 OpenRouter 的 PKCE 浏览器授权流程。
//!
//! Claude 使用官方的“浏览器显示 code#state，用户粘贴回应用”模式；
//! OpenRouter 支持 localhost 任意端口，使用一次性 loopback listener 自动接收回调。

use crate::core::providers::{self, Credential};
use anyhow::{anyhow, Context};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const CLAUDE_AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const CLAUDE_TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLAUDE_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const CLAUDE_REDIRECT_URI: &str = "https://platform.claude.com/oauth/code/callback";
const CLAUDE_SCOPES: &str = "user:inference user:profile";
const OPENROUTER_AUTHORIZE_URL: &str = "https://openrouter.ai/auth";
const OPENROUTER_TOKEN_URL: &str = "https://openrouter.ai/api/v1/auth/keys";
const SESSION_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Serialize)]
pub struct PkceStart {
    pub session_id: String,
    pub authorize_url: String,
}

struct PkceSession {
    verifier: String,
    state: String,
    created_at: Instant,
}

struct OpenRouterSession {
    pkce: PkceSession,
    listener: TcpListener,
}

static CLAUDE_PENDING: OnceLock<Mutex<HashMap<String, PkceSession>>> = OnceLock::new();
static OPENROUTER_PENDING: OnceLock<Mutex<HashMap<String, OpenRouterSession>>> = OnceLock::new();

fn claude_pending() -> &'static Mutex<HashMap<String, PkceSession>> {
    CLAUDE_PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

fn openrouter_pending() -> &'static Mutex<HashMap<String, OpenRouterSession>> {
    OPENROUTER_PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock<T>(mutex: &'static Mutex<T>) -> anyhow::Result<MutexGuard<'static, T>> {
    mutex.lock().map_err(|_| anyhow!("OAuth 会话锁已损坏"))
}

fn random_urlsafe(byte_len: usize) -> String {
    let mut bytes = vec![0u8; byte_len];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn pkce_session() -> PkceSession {
    PkceSession {
        verifier: random_urlsafe(32),
        state: random_urlsafe(24),
        created_at: Instant::now(),
    }
}

fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn session_id() -> String {
    random_urlsafe(18)
}

fn remove_expired<T>(sessions: &mut HashMap<String, T>, created_at: impl Fn(&T) -> Instant) {
    sessions.retain(|_, session| created_at(session).elapsed() < SESSION_TTL);
}

pub fn start_claude() -> anyhow::Result<PkceStart> {
    let session = pkce_session();
    let id = session_id();
    let mut url = reqwest::Url::parse(CLAUDE_AUTHORIZE_URL)?;
    url.query_pairs_mut()
        .append_pair("code", "true")
        .append_pair("client_id", CLAUDE_CLIENT_ID)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", CLAUDE_REDIRECT_URI)
        .append_pair("scope", CLAUDE_SCOPES)
        .append_pair("code_challenge", &challenge(&session.verifier))
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &session.state);

    let mut sessions = lock(claude_pending())?;
    remove_expired(&mut sessions, |session| session.created_at);
    sessions.insert(id.clone(), session);
    Ok(PkceStart {
        session_id: id,
        authorize_url: url.to_string(),
    })
}

fn split_claude_code(input: &str) -> anyhow::Result<(&str, Option<&str>)> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        anyhow::bail!("授权码不能为空");
    }
    Ok(match trimmed.split_once('#') {
        Some((code, state)) => (code.trim(), Some(state.trim())),
        None => (trimmed, None),
    })
}

pub async fn complete_claude(session_id: &str, pasted_code: &str) -> anyhow::Result<Credential> {
    let session = {
        let mut sessions = lock(claude_pending())?;
        let session = sessions
            .remove(session_id)
            .ok_or_else(|| anyhow!("Claude OAuth 会话不存在或已过期，请重新授权"))?;
        if session.created_at.elapsed() >= SESSION_TTL {
            anyhow::bail!("Claude OAuth 会话已过期，请重新授权");
        }
        session
    };
    let (code, returned_state) = split_claude_code(pasted_code)?;
    if let Some(returned_state) = returned_state {
        if returned_state != session.state {
            anyhow::bail!("Claude OAuth state 不匹配，请重新授权");
        }
    }

    let body = json!({
        "grant_type": "authorization_code",
        "client_id": CLAUDE_CLIENT_ID,
        "code": code,
        "redirect_uri": CLAUDE_REDIRECT_URI,
        "code_verifier": session.verifier,
        "state": returned_state.unwrap_or(&session.state),
    });
    let response = providers::http_client()
        .post(CLAUDE_TOKEN_URL)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        anyhow::bail!(
            "Claude 授权码交换失败 HTTP {status}: {}",
            safe_error(&detail)
        );
    }
    let value = response.json::<Value>().await?;
    let access_token = value
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| anyhow!("Claude 授权响应缺少 access_token"))?;
    let refresh_token = value
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| anyhow!("Claude 授权响应缺少 refresh_token"))?;
    let expires_in = value
        .get("expires_in")
        .and_then(Value::as_i64)
        .unwrap_or(3600);
    Ok(crate::core::providers::claude::oauth_credential(
        access_token,
        refresh_token,
        expires_in,
        CLAUDE_SCOPES.split_whitespace(),
    ))
}

pub async fn start_openrouter() -> anyhow::Result<PkceStart> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("无法启动 OpenRouter 本地 OAuth 回调")?;
    let port = listener.local_addr()?.port();
    let callback_url = format!("http://localhost:{port}/callback");
    let session = pkce_session();
    let id = session_id();
    let mut url = reqwest::Url::parse(OPENROUTER_AUTHORIZE_URL)?;
    url.query_pairs_mut()
        .append_pair("callback_url", &callback_url)
        .append_pair("state", &session.state)
        .append_pair("code_challenge", &challenge(&session.verifier))
        .append_pair("code_challenge_method", "S256")
        .append_pair("key_label", "TokenMeter");

    let mut sessions = lock(openrouter_pending())?;
    remove_expired(&mut sessions, |session| session.pkce.created_at);
    sessions.insert(
        id.clone(),
        OpenRouterSession {
            pkce: session,
            listener,
        },
    );
    Ok(PkceStart {
        session_id: id,
        authorize_url: url.to_string(),
    })
}

async fn receive_openrouter_code(session: OpenRouterSession) -> anyhow::Result<String> {
    let (mut stream, _) = tokio::time::timeout(Duration::from_secs(180), session.listener.accept())
        .await
        .map_err(|_| anyhow!("OpenRouter 浏览器授权等待超时，请重试"))??;
    let mut buffer = vec![0u8; 16 * 1024];
    let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buffer))
        .await
        .map_err(|_| anyhow!("OpenRouter 回调读取超时"))??;
    let request = std::str::from_utf8(&buffer[..read]).context("OpenRouter 回调不是有效 HTTP")?;
    let target = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| anyhow!("OpenRouter 回调请求格式无效"))?;
    let url = reqwest::Url::parse(&format!("http://localhost{target}"))?;
    let params = url.query_pairs().collect::<HashMap<_, _>>();
    let error = params.get("error").map(|value| value.as_ref());
    if let Some(error) = error {
        let message = params
            .get("error_description")
            .map(|value| value.as_ref())
            .unwrap_or(error);
        write_callback_html(&mut stream, false).await;
        anyhow::bail!("OpenRouter 授权被拒绝: {message}");
    }
    let state = params
        .get("state")
        .map(|value| value.as_ref())
        .ok_or_else(|| anyhow!("OpenRouter 回调缺少 state"))?;
    if state != session.pkce.state {
        write_callback_html(&mut stream, false).await;
        anyhow::bail!("OpenRouter OAuth state 不匹配，请重新授权");
    }
    let code = params
        .get("code")
        .map(|value| value.to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("OpenRouter 回调缺少授权码"))?;
    write_callback_html(&mut stream, true).await;
    Ok(code)
}

async fn write_callback_html(stream: &mut tokio::net::TcpStream, success: bool) {
    let (title, body) = if success {
        (
            "授权成功",
            "已连接 OpenRouter，可以关闭此页面并返回 TokenMeter。",
        )
    } else {
        (
            "授权失败",
            "OpenRouter 授权未完成，请返回 TokenMeter 重试。",
        )
    };
    let html = format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width'><title>{title}</title><style>body{{font-family:system-ui,-apple-system,sans-serif;display:grid;place-items:center;min-height:90vh;background:#f7f7fb;color:#17171c}}main{{padding:32px;text-align:center}}h1{{font-size:22px}}p{{color:#666}}</style><main><h1>{title}</h1><p>{body}</p></main>"
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

pub async fn complete_openrouter(session_id: &str) -> anyhow::Result<Credential> {
    let session = {
        let mut sessions = lock(openrouter_pending())?;
        let session = sessions
            .remove(session_id)
            .ok_or_else(|| anyhow!("OpenRouter OAuth 会话不存在或已过期，请重新授权"))?;
        if session.pkce.created_at.elapsed() >= SESSION_TTL {
            anyhow::bail!("OpenRouter OAuth 会话已过期，请重新授权");
        }
        session
    };
    let verifier = session.pkce.verifier.clone();
    let code = receive_openrouter_code(session).await?;
    let response = providers::http_client()
        .post(OPENROUTER_TOKEN_URL)
        .header("Content-Type", "application/json")
        .json(&json!({
            "code": code,
            "code_verifier": verifier,
            "code_challenge_method": "S256",
        }))
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        anyhow::bail!(
            "OpenRouter 授权码交换失败 HTTP {status}: {}",
            safe_error(&detail)
        );
    }
    let value = response.json::<Value>().await?;
    let api_key = value
        .get("key")
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| anyhow!("OpenRouter 授权响应缺少 API Key"))?;
    Ok(Credential {
        data: json!({
            "api_key": api_key,
            "account_id": value.get("user_id").and_then(Value::as_str),
            "source_kind": "oauth",
        }),
    })
}

fn safe_error(detail: &str) -> String {
    let normalized = detail.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() > 300 {
        format!("{}…", normalized.chars().take(300).collect::<String>())
    } else {
        normalized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_urlsafe_and_deterministic() {
        let first = challenge("test-verifier");
        assert_eq!(first, challenge("test-verifier"));
        assert!(!first.contains(['+', '/', '=']));
    }

    #[test]
    fn claude_code_accepts_code_with_or_without_state() {
        assert_eq!(
            split_claude_code("code#state").unwrap(),
            ("code", Some("state"))
        );
        assert_eq!(split_claude_code(" code ").unwrap(), ("code", None));
        assert!(split_claude_code(" ").is_err());
    }

    #[test]
    fn claude_authorize_url_contains_current_pkce_contract() {
        let start = start_claude().unwrap();
        let url = reqwest::Url::parse(&start.authorize_url).unwrap();
        let params = url.query_pairs().collect::<HashMap<_, _>>();
        assert_eq!(url.as_str().split('?').next(), Some(CLAUDE_AUTHORIZE_URL));
        assert_eq!(
            params.get("client_id").map(|value| value.as_ref()),
            Some(CLAUDE_CLIENT_ID)
        );
        assert_eq!(
            params
                .get("code_challenge_method")
                .map(|value| value.as_ref()),
            Some("S256")
        );
        assert_eq!(
            params.get("redirect_uri").map(|value| value.as_ref()),
            Some(CLAUDE_REDIRECT_URI)
        );
    }

    #[tokio::test]
    async fn openrouter_authorize_url_uses_loopback_pkce_and_state() {
        let start = start_openrouter().await.unwrap();
        let url = reqwest::Url::parse(&start.authorize_url).unwrap();
        let params = url.query_pairs().collect::<HashMap<_, _>>();
        assert_eq!(
            url.as_str().split('?').next(),
            Some(OPENROUTER_AUTHORIZE_URL)
        );
        assert!(params
            .get("callback_url")
            .is_some_and(|value| value.starts_with("http://localhost:")));
        assert_eq!(
            params
                .get("code_challenge_method")
                .map(|value| value.as_ref()),
            Some("S256")
        );
        assert!(params.contains_key("state"));
    }
}
