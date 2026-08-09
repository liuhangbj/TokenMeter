//! OAuth 设备码授权流程（Kimi Code）。
//!
//! ⚠️ 端点/参数/请求头均以 2026-08-02 实测跑通的脚本为准（勿凭 OAuth 标准猜）：
//!   - 设备码端点：`POST /api/oauth/device_authorization`（**不是** /device/code，后者 404）
//!   - 必带请求头：`X-Msh-Platform: kimi_code_cli`、`X-Msh-Device-Name/Model/Id`（缺了会 4xx）
//!   - 设备码请求体：仅 `client_id`（**不带 scope**）
//!   - 轮询令牌：`POST /api/oauth/token`，grant_type=device_code
//!   - 授权页字段：`verification_uri_complete`（fallback `verification_uri`）
//!
//! 凭证仅经内存 → 加密存储，不落盘明文。

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::Deserialize;
use serde_json::json;

const CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
const OAUTH_HOST: &str = "https://auth.kimi.com";
const DEVICE_AUTH_URL: &str = "https://auth.kimi.com/api/oauth/device_authorization";
const TOKEN_URL: &str = "https://auth.kimi.com/api/oauth/token";

/// 设备码流程第一步的返回（给前端展示）
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeviceAuthStart {
    pub user_code: String,
    pub verify_url: String,
    pub device_code: String,
    pub interval_secs: u64,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResp {
    device_code: String,
    user_code: String,
    // 返回同时含 verification_uri 和 verification_uri_complete，不能用 alias（会 duplicate field）。
    // complete 版自带 user_code 参数更完整，优先取它，缺了再退回 uri。
    verification_uri_complete: Option<String>,
    verification_uri: Option<String>,
    #[serde(default = "default_interval")]
    interval: u64,
}

fn default_interval() -> u64 {
    5
}

fn device_model() -> &'static str {
    if cfg!(target_os = "windows") {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else {
        "Linux"
    }
}

/// 实测脚本要求的公共请求头。
fn apply_common_headers(builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    let host = hostname::get()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".to_string());
    builder
        .header("X-Msh-Platform", "kimi_code_cli")
        .header("X-Msh-Device-Name", host)
        .header("X-Msh-Device-Model", device_model())
        .header("X-Msh-Device-Id", "tokenmeter")
}

/// 第一步：请求设备码。
pub async fn start() -> Result<DeviceAuthStart> {
    let client = crate::core::providers::http_client();
    let req = client
        .post(DEVICE_AUTH_URL)
        .form(&[("client_id", CLIENT_ID)]);
    let resp = apply_common_headers(req).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(anyhow!("请求设备码失败 {status}: {body}"));
    }
    let d = resp.json::<DeviceCodeResp>().await?;
    let verify_url = d
        .verification_uri_complete
        .or(d.verification_uri)
        .ok_or_else(|| anyhow!("响应缺少 verification_uri"))?;
    Ok(DeviceAuthStart {
        user_code: d.user_code,
        verify_url,
        device_code: d.device_code,
        interval_secs: d.interval.max(3),
    })
}

#[derive(Debug, Deserialize)]
struct TokenResp {
    access_token: Option<String>,
    refresh_token: Option<String>,
    error: Option<String>,
}

/// 第二步：轮询直到用户授权（或超时）。返回凭证 JSON。
pub async fn poll_until_authorized(
    device_code: &str,
    interval_secs: u64,
) -> Result<serde_json::Value> {
    let client = crate::core::providers::http_client();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(600);
    let mut wait = interval_secs.max(3);

    loop {
        if tokio::time::Instant::now() > deadline {
            return Err(anyhow!("授权超时（10 分钟未完成）"));
        }
        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;

        let req = client.post(TOKEN_URL).form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", device_code),
            ("client_id", CLIENT_ID),
        ]);
        let resp = apply_common_headers(req)
            .send()
            .await?
            .json::<TokenResp>()
            .await?;

        if let Some(at) = resp.access_token {
            let rt = resp.refresh_token.unwrap_or_default();
            return Ok(json!({
                "access_token": at,
                "refresh_token": rt,
                "account_label": account_label_from_access_token(&at),
            }));
        }
        match resp.error.as_deref() {
            Some("authorization_pending") => { /* 继续轮询 */ }
            Some("slow_down") => wait += 2,
            Some("expired_token") => return Err(anyhow!("设备码已过期，请重新发起授权")),
            Some(other) => return Err(anyhow!("授权失败：{other}")),
            None => return Err(anyhow!("授权响应异常")),
        }
    }
}

/// Kimi access token 有时是 JWT。若带有身份 claims，按用户名、邮箱、ID 选取；
/// token 为不透明字符串或没有身份 claim 时返回 None，由 usages.userId 兜底。
pub(crate) fn account_label_from_access_token(access_token: &str) -> Option<String> {
    let payload_b64 = access_token.split('.').nth(1)?;
    let payload = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&payload).ok()?;
    [
        "preferred_username",
        "username",
        "name",
        "email",
        "user_id",
        "sub",
    ]
    .into_iter()
    .find_map(|key| value.get(key).and_then(|field| field.as_str()))
    .filter(|label| !label.is_empty())
    .map(str::to_string)
}

// 保留常量避免未使用告警（OAUTH_HOST 供未来扩展）
#[allow(dead_code)]
const _: &str = OAUTH_HOST;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_label_prefers_username_then_email_then_id() {
        let payload = URL_SAFE_NO_PAD
            .encode(br#"{"username":"kimi-user","email":"kimi@example.com","user_id":"user_123"}"#);
        assert_eq!(
            account_label_from_access_token(&format!("header.{payload}.signature")),
            Some("kimi-user".to_string())
        );

        let email_payload =
            URL_SAFE_NO_PAD.encode(br#"{"email":"kimi@example.com","user_id":"user_123"}"#);
        assert_eq!(
            account_label_from_access_token(&format!("header.{email_payload}.signature")),
            Some("kimi@example.com".to_string())
        );

        let id_payload = URL_SAFE_NO_PAD.encode(br#"{"user_id":"user_123"}"#);
        assert_eq!(
            account_label_from_access_token(&format!("header.{id_payload}.signature")),
            Some("user_123".to_string())
        );
    }

    #[test]
    fn device_model_matches_build_platform() {
        #[cfg(target_os = "windows")]
        assert_eq!(device_model(), "Windows");
        #[cfg(target_os = "macos")]
        assert_eq!(device_model(), "macOS");
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        assert_eq!(device_model(), "Linux");
    }
}
