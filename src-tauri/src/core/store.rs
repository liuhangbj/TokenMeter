//! 凭证加密存储（无系统 Keychain 依赖）
//!
//! 背景（2026-08-02）：用户无 Apple 开发者账户，macOS Keychain 对未签名/反复重编译
//! 的 debug 二进制有访问控制限制（写入后读取 NoEntry），开发期不可用。故统一用
//! **AES-256-GCM 加密文件**，跨 macOS / Windows，摆脱签名限制。
//!
//! 安全模型（2026-08-02 修订，替代"设备指纹派生密钥"）：
//! - 主密钥为**首次使用时随机生成**的 32 字节，独立存放于 `credentials.key`，
//!   Unix 上权限 0600（仅属主可读写），Windows 上位于用户级 AppData。
//! - 密文 `credentials.json` 同样以 0600 写入。
//! - 攻击者需要同时拿到密钥文件与密文文件才能解密；不再存在
//!   "hostname+username 公开可推导 → 拿到密文即解密"的漏洞。
//! - 旧版本（密钥由设备指纹派生）数据在首次读取时自动迁移重加密。
//!
//! ⚠️ 仍比系统 Keychain 弱：同一用户下的其他进程若可读这两个文件即可解密。
//! 这是无签名环境下的现实折中。
#![allow(dead_code)]

use crate::core::providers::Credential;
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use rand::RngCore;
use serde_json::{json, Value};
use sha2::Sha256;
use hkdf::Hkdf;
use std::io::Write;
use std::path::PathBuf;

const SERVICE: &str = "com.hangbits.tokenmeter";
const NONCE_LEN: usize = 12; // AES-GCM 标准 96-bit nonce
const KEY_FILE: &str = "credentials.key";

/// 数据目录：macOS ~/Library/Application Support/TokenMeter，Windows %APPDATA%\TokenMeter
/// 可用环境变量 TOKENMETER_DATA_DIR 覆盖（本机 dev 测试实例隔离数据用）。
pub(crate) fn data_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("TOKENMETER_DATA_DIR") {
        if !dir.trim().is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }
    let dir = if cfg!(target_os = "windows") {
        let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
        format!("{appdata}\\TokenMeter")
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        format!("{home}/Library/Application Support/TokenMeter")
    };
    Ok(PathBuf::from(dir))
}

/// 凭证密文路径：数据目录/credentials.json
fn cred_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("credentials.json"))
}

/// 主密钥路径：数据目录/credentials.key（随机 32 字节，0600）
fn key_path() -> Result<PathBuf> {
    Ok(data_dir()?.join(KEY_FILE))
}

/// 以 0600 权限写文件（Unix）；Windows 依赖用户级 AppData 的 ACL。
pub(crate) fn write_private(path: &PathBuf, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(data)?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, data)?;
    }
    Ok(())
}

/// 读取或生成主密钥（随机 32 字节）。
fn load_or_create_key() -> Result<[u8; 32]> {
    let p = key_path()?;
    if let Ok(bytes) = std::fs::read(&p) {
        if bytes.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            return Ok(key);
        }
        log::warn!("{} 长度异常（{} 字节），重新生成新密钥", p.display(), bytes.len());
    }
    let mut key = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut key);
    write_private(&p, &key)?;
    Ok(key)
}

/// 旧版本密钥派生（hostname + username + service，公开可推导）。
/// 仅用于迁移解密旧密文；新数据一律使用随机密钥。
fn legacy_master_key() -> [u8; 32] {
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

/// 读取整个凭证文件（解密前的密文 map）。
fn read_store() -> Result<Value> {
    let p = cred_path()?;
    if !p.exists() {
        return Ok(json!({}));
    }
    let s = std::fs::read_to_string(&p)?;
    Ok(serde_json::from_str(&s).unwrap_or_else(|_| json!({})))
}

/// 写回整个凭证文件（0600）。
fn write_store(store: &Value) -> Result<()> {
    let p = cred_path()?;
    let data = serde_json::to_string_pretty(store)?;
    write_private(&p, data.as_bytes())
}

/// 用指定密钥加密一段明文 → base64(nonce ‖ ciphertext)。
fn encrypt_with(key: &[u8; 32], plain: &str) -> Result<String> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| anyhow!("密钥错误: {e}"))?;
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

/// 用指定密钥解密 base64(nonce ‖ ciphertext) → 明文。
fn decrypt_with(key: &[u8; 32], b64: &str) -> Result<String> {
    let blob = B64.decode(b64.trim()).map_err(|e| anyhow!("base64 解码失败: {e}"))?;
    if blob.len() < NONCE_LEN {
        return Err(anyhow!("密文过短"));
    }
    let (nonce_bytes, ct) = blob.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| anyhow!("密钥错误: {e}"))?;
    let plain = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ct)
        .map_err(|_| anyhow!("解密失败（密钥不匹配或数据损坏）"))?;
    Ok(String::from_utf8(plain)?)
}

pub fn save_credential(provider_id: &str, cred: &Credential) -> Result<()> {
    let key = load_or_create_key()?;
    let json = serde_json::to_string(&cred.data)?;
    let sealed = encrypt_with(&key, &json)?;
    let mut store = read_store()?;
    store[provider_id] = json!(sealed);
    write_store(&store)?;
    log::info!("凭证已加密写入 {}（{} 字节密文）", provider_id, sealed.len());
    Ok(())
}

pub fn load_credential(provider_id: &str) -> Option<Credential> {
    let store = read_store().ok()?;
    let sealed = store.get(provider_id)?.as_str()?;
    let key = load_or_create_key().ok()?;
    match decrypt_with(&key, sealed) {
        Ok(json) => {
            let data = serde_json::from_str(&json).ok()?;
            Some(Credential { data })
        }
        Err(_) => {
            // 迁移：旧版本用设备指纹派生密钥。若旧密钥能解开，则用新密钥重写该条。
            let legacy = legacy_master_key();
            match decrypt_with(&legacy, sealed) {
                Ok(json) => {
                    log::info!("{provider_id}: 检测到旧版加密，迁移到随机密钥");
                    let data = serde_json::from_str(&json).ok()?;
                    if let Ok(new_sealed) = encrypt_with(&key, &json) {
                        let mut store = read_store().ok()?;
                        store[provider_id] = json!(new_sealed);
                        let _ = write_store(&store);
                    }
                    Some(Credential { data })
                }
                Err(e) => {
                    log::warn!("凭证 {} 解密失败: {e}", provider_id);
                    None
                }
            }
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

/// 列出已配置凭证的 provider_id（前端判断"还没有添加供应商"与首屏状态用）。
pub fn configured_provider_ids() -> Vec<String> {
    read_store()
        .ok()
        .and_then(|s| s.as_object().cloned())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}
