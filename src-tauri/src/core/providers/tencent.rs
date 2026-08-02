//! 腾讯云 TC3-HMAC-SHA256 签名 + 请求发送助手
//!
//! 腾讯云所有 API 共用同一套签名算法，Token Plan / TokenHub 按量 / Coding Plan
//! 都通过本模块发送请求。参考官方签名文档实现。

use anyhow::anyhow;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use serde_json::Value;

type HmacSha256 = Hmac<Sha256>;

fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex(&h.finalize())
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut m = HmacSha256::new_from_slice(key).expect("hmac key");
    m.update(data);
    m.finalize().into_bytes().to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 腾讯云接口数值字段可能是数字也可能是字符串，统一解析。
pub fn value_num(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
}

/// 发送腾讯云 API 请求（TC3-HMAC-SHA256 签名）
///
/// - `service` 服务名（如 `tokenhub` / `billing`）
/// - `host`    完整域名（如 `tokenhub.tencentcloudapi.com`）
/// - `action`  X-TC-Action
/// - `version` X-TC-Version
pub async fn tencent_post(
    client: &reqwest::Client,
    service: &str,
    host: &str,
    action: &str,
    version: &str,
    secret_id: &str,
    secret_key: &str,
    region: Option<&str>,
    payload: &Value,
) -> anyhow::Result<Value> {
    let now = chrono::Utc::now();
    let timestamp = now.timestamp();
    let date = now.format("%Y-%m-%d").to_string();
    let payload_str = serde_json::to_string(payload)?;
    let hashed_payload = sha256_hex(payload_str.as_bytes());

    let action_lower = action.to_lowercase();
    let canonical_headers =
        format!("content-type:application/json\nhost:{host}\nx-tc-action:{action_lower}\n");
    let signed_headers = "content-type;host;x-tc-action";
    let canonical_request =
        format!("POST\n/\n\n{canonical_headers}\n{signed_headers}\n{hashed_payload}");

    let credential_scope = format!("{date}/{service}/tc3_request");
    let hashed_canonical = sha256_hex(canonical_request.as_bytes());
    let string_to_sign =
        format!("TC3-HMAC-SHA256\n{timestamp}\n{credential_scope}\n{hashed_canonical}");

    let secret_date = hmac(format!("TC3{secret_key}").as_bytes(), date.as_bytes());
    let secret_service = hmac(&secret_date, service.as_bytes());
    let secret_signing = hmac(&secret_service, b"tc3_request");
    let signature = hex(&hmac(&secret_signing, string_to_sign.as_bytes()));

    let authorization = format!(
        "TC3-HMAC-SHA256 Credential={secret_id}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}"
    );

    let mut req = client
        .post(format!("https://{host}"))
        .header("Authorization", authorization)
        .header("Content-Type", "application/json")
        .header("Host", host)
        .header("X-TC-Action", action)
        .header("X-TC-Version", version)
        .header("X-TC-Timestamp", timestamp.to_string())
        .body(payload_str);
    if let Some(r) = region {
        req = req.header("X-TC-Region", r);
    }

    let resp = req.send().await?;
    let status = resp.status();
    let body: Value = resp.json().await?;
    if !status.is_success() {
        return Err(anyhow!("tencent {action} HTTP {status}: {body}"));
    }
    // 腾讯云业务错误放在 Response.Error
    if let Some(err) = body.get("Response").and_then(|r| r.get("Error")) {
        return Err(anyhow!("tencent {action} API error: {err}"));
    }
    Ok(body)
}
