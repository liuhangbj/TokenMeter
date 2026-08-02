//! 凭证加密存储（替代 Keychain）
//!
//! 背景（2026-08-02）：用户无 Apple 开发者账户，macOS Keychain 对未签名/反复重编译
//! 的 debug 二进制有访问控制限制（写入后读取 NoEntry），开发期不可用。故改用
//! **AES-256-GCM 加密文件**，统一所有构建、摆脱签名限制。
//!
//! 安全模型：
//! - 主密钥由设备特征（hostname + username + service 名）经 HKDF-SHA256 派生，
//!   不落盘、不需用户密码；换设备/重装系统即失效（凭证天然绑定本机）。
//! - 每个凭证独立随机 nonce，AES-256-GCM 认证加密（机密性 + 完整性）。
//! - 密文 base64 后写入应用配置目录 `credentials.json`（与 settings.json 同目录，
//!   非散落临时文件）。⚠️ 该文件仅含密文，无密钥、无明文。
#![allow(dead_code)]

use crate::providers::Credential;
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use hkdf::Hkdf;
use rand::RngCore;
use serde_json::{json, Value};
use sha2::Sha256;
use std::path::PathBuf;

const SERVICE: &str = "com.hangbits.tokenmeter";
const NONCE_LEN: usize = 12; // AES-GCM 标准 96-bit nonce

/// 派生设备绑定的主密钥（32 字节）。
fn master_key() -> [u8; 32] {
    let host = hostname::get()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown-host".to_string());
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown-user".to_string());
    let ikm = format!("{SERVICE}|{host}|{user}");
    let hk = Hkdf::<Sha256>::new(Some(SERVICE.as_bytes()), ikm.as_bytes());
    let mut okm = [0u8; 32];
    hk.expand(b"tokenmeter-credential-key", &mut okm)
        .expect("HKDF expand 不会失败（输出长度合法）");
    okm
}

/// 凭证文件路径：应用配置目录/credentials.json
fn cred_path() -> Result<PathBuf> {
    let dir = if cfg!(target_os = "windows") {
        let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
        format!("{appdata}\\TokenMeter")
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        format!("{home}/Library/Application Support/TokenMeter")
    };
    Ok(PathBuf::from(dir).join("credentials.json"))
}

/// 读取整个凭证文件（解密前的密文 map）。
fn read_store() -> Result<Value> {
    let p = cred_path()?;
    if !p.exists() {
        return Ok(json!({}));
    }
    let s = std::fs::read_to_string(&p)?;
    Ok(serde_json::from_str(&s).unwrap_or_else(|_| json!({})))
}

/// 写回整个凭证文件。
fn write_store(store: &Value) -> Result<()> {
    let p = cred_path()?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&p, serde_json::to_string_pretty(store)?)?;
    Ok(())
}

/// 加密一段明文 → base64(nonce ‖ ciphertext)。
fn encrypt(plain: &str) -> Result<String> {
    let key = master_key();
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| anyhow!("密钥错误: {e}"))?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plain.as_bytes())
        .map_err(|e| anyhow!("加密失败: {e}"))?;
    let mut blob = nonce_bytes.to_vec();
    blob.extend_from_slice(&ct);
    Ok(B64.encode(blob))
}

/// 解密 base64(nonce ‖ ciphertext) → 明文。
fn decrypt(b64: &str) -> Result<String> {
    let blob = B64.decode(b64.trim()).map_err(|e| anyhow!("base64 解码失败: {e}"))?;
    if blob.len() < NONCE_LEN {
        return Err(anyhow!("密文过短"));
    }
    let (nonce_bytes, ct) = blob.split_at(NONCE_LEN);
    let key = master_key();
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| anyhow!("密钥错误: {e}"))?;
    let plain = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ct)
        .map_err(|_| anyhow!("解密失败（密钥不匹配或数据损坏）"))?;
    Ok(String::from_utf8(plain)?)
}

pub fn save_credential(provider_id: &str, cred: &Credential) -> Result<()> {
    let json = serde_json::to_string(&cred.data)?;
    let sealed = encrypt(&json)?;
    let mut store = read_store()?;
    store[provider_id] = json!(sealed);
    write_store(&store)?;
    log::info!("凭证已加密写入 {}（{} 字节密文）", provider_id, sealed.len());
    Ok(())
}

pub fn load_credential(provider_id: &str) -> Option<Credential> {
    let store = read_store().ok()?;
    let sealed = store.get(provider_id)?.as_str()?;
    match decrypt(sealed) {
        Ok(json) => {
            let data = serde_json::from_str(&json).ok()?;
            Some(Credential { data })
        }
        Err(e) => {
            log::warn!("凭证 {} 解密失败: {e}", provider_id);
            None
        }
    }
}

pub fn delete_credential(provider_id: &str) -> Result<()> {
    let mut store = read_store()?;
    if let Some(map) = store.as_object_mut() {
        if map.remove(provider_id).is_some() {
            write_store(&store)?;
        }
    }
    Ok(())
}
