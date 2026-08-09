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
use anyhow::{anyhow, Context, Result};
use base64::{
    engine::general_purpose::{STANDARD as B64, URL_SAFE_NO_PAD},
    Engine as _,
};
use hkdf::Hkdf;
use rand::RngCore;
use serde_json::{json, Value};
use sha2::Sha256;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

const SERVICE: &str = "com.hangbits.tokenmeter";
const NONCE_LEN: usize = 12; // AES-GCM 标准 96-bit nonce
const KEY_FILE: &str = "credentials.key";
static STORE_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn store_write_lock() -> Result<MutexGuard<'static, ()>> {
    STORE_WRITE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| anyhow!("凭证存储写入锁已损坏，已停止写入"))
}

/// 一个已配置的账号实例。`account_id` 是凭证存储、缓存和 UI 操作的稳定主键；
/// `provider_id` 仍表示它属于哪个平台。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialAccount {
    pub account_id: String,
    pub provider_id: String,
}

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

/// 修正已有私有文件的权限。旧版本可能已经创建过 0644 文件，单靠
/// `OpenOptionsExt::mode` 不会改变它，因此每次读取前也要主动收紧权限。
pub(crate) fn ensure_private_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    if path.exists() {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// 原子写入私有文件：先在同目录写完并同步临时文件，再原子替换目标。
/// Unix 强制 0600；Windows 继承用户级 AppData 目录 ACL。
pub(crate) fn write_private(path: &PathBuf, data: &[u8]) -> Result<()> {
    use std::io::Write;

    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("文件路径缺少父目录: {}", path.display()))?;
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::Builder::new()
        .prefix(".tokenmeter-write-")
        .tempfile_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    temp.write_all(data)?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .map_err(|error| anyhow!("原子替换 {} 失败: {}", path.display(), error.error))?;
    ensure_private_permissions(path)?;
    #[cfg(unix)]
    if let Err(error) = std::fs::File::open(parent).and_then(|directory| directory.sync_all()) {
        // 文件本身已经完整同步并原子替换；目录同步失败只影响极端断电场景，
        // 不能在提交完成后向调用方谎报“保存失败”并触发状态回滚。
        log::warn!("目录 {} 同步失败: {error}", parent.display());
    }
    Ok(())
}

/// 读取或生成主密钥（随机 32 字节）。
fn load_or_create_key() -> Result<[u8; 32]> {
    let p = key_path()?;
    match std::fs::read(&p) {
        Ok(bytes) => {
            ensure_private_permissions(&p)?;
            if bytes.len() != 32 {
                return Err(anyhow!(
                    "主密钥 {} 长度异常（{} 字节），已停止操作以保护现有凭证",
                    p.display(),
                    bytes.len()
                ));
            }
            let mut key = [0u8; 32];
            key.copy_from_slice(&bytes);
            return Ok(key);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("读取主密钥 {} 失败", p.display()))
        }
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

fn parse_store(contents: &str) -> Result<Value> {
    let store: Value = serde_json::from_str(contents).context("凭证文件 JSON 已损坏")?;
    if !store.is_object() {
        return Err(anyhow!("凭证文件顶层结构必须是对象"));
    }
    Ok(store)
}

/// 读取整个凭证文件（解密前的密文 map）。损坏时明确报错，绝不把它当成
/// 空数据继续写回，避免一次局部保存覆盖全部账号。
fn read_store() -> Result<Value> {
    let p = cred_path()?;
    if !p.exists() {
        return Ok(json!({}));
    }
    ensure_private_permissions(&p)?;
    let s = std::fs::read_to_string(&p)?;
    parse_store(&s).with_context(|| format!("无法读取 {}，原文件保持不变", p.display()))
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
    let blob = B64
        .decode(b64.trim())
        .map_err(|e| anyhow!("base64 解码失败: {e}"))?;
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
    let _guard = store_write_lock()?;
    let key = load_or_create_key()?;
    let json = serde_json::to_string(&cred.data)?;
    let sealed = encrypt_with(&key, &json)?;
    let mut store = read_store()?;
    store[provider_id] = json!(sealed);
    write_store(&store)?;
    log::info!(
        "凭证已加密写入 {}（{} 字节密文）",
        provider_id,
        sealed.len()
    );
    Ok(())
}

/// 为平台新增一个账号实例并保存凭证。
///
/// 旧版只有一个账号时，凭证键直接是 `codex` 之类的平台 ID；新版以
/// `codex#1`、`codex#2` 的形式追加。两种键都会被读取，因此旧账号不需要重登。
pub fn create_credential(provider_id: &str, cred: &Credential) -> Result<String> {
    let _guard = store_write_lock()?;
    let key = load_or_create_key()?;
    let json = serde_json::to_string(&cred.data)?;
    let sealed = encrypt_with(&key, &json)?;
    let mut store = read_store()?;
    let account_id = next_account_id(&store, provider_id);
    store[&account_id] = json!(sealed);
    write_store(&store)?;
    log::info!(
        "凭证已加密写入账号 {}（平台 {}，{} 字节密文）",
        account_id,
        provider_id,
        sealed.len()
    );
    Ok(account_id)
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
                        if let Ok(_guard) = store_write_lock() {
                            if let Ok(mut store) = read_store() {
                                store[provider_id] = json!(new_sealed);
                                let _ = write_store(&store);
                            }
                        }
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

pub fn delete_credential(account_id: &str) -> Result<()> {
    let _guard = store_write_lock()?;
    let mut store = read_store()?;
    if let Some(map) = store.as_object_mut() {
        if map.remove(account_id).is_some() {
            write_store(&store)?;
        }
    }
    Ok(())
}

/// 列出已配置账号。旧版 `provider_id` 键会兼容地视为该平台的第一个账号。
pub fn configured_accounts() -> Vec<CredentialAccount> {
    let store = match read_store() {
        Ok(store) => store,
        Err(error) => {
            log::error!("读取凭证账号失败: {error:#}");
            return Vec::new();
        }
    };
    store
        .as_object()
        .cloned()
        .map(|m| {
            m.into_iter()
                // 凭证存储的值始终是密文字符串；跳过无效/未来扩展字段。
                .filter(|(_, value)| value.is_string())
                .map(|(account_id, _)| CredentialAccount {
                    provider_id: provider_id_from_account_id(&account_id).to_string(),
                    account_id,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn provider_id_from_account_id(account_id: &str) -> &str {
    account_id
        .split_once('#')
        .map_or(account_id, |(provider_id, _)| provider_id)
}

fn next_account_id(store: &Value, provider_id: &str) -> String {
    let highest_number = store
        .as_object()
        .into_iter()
        .flat_map(|accounts| accounts.keys())
        .filter_map(|account_id| {
            if account_id == provider_id {
                Some(1)
            } else {
                account_id
                    .strip_prefix(&format!("{provider_id}#"))
                    .and_then(|suffix| suffix.parse::<u32>().ok())
            }
        })
        .max()
        .unwrap_or(0);

    // 正常情况下按序编号；极端情况下编号溢出时退回随机后缀，仍保证不覆盖已有凭证。
    if let Some(next) = highest_number.checked_add(1) {
        format!("{provider_id}#{next}")
    } else {
        let mut bytes = [0_u8; 9];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        format!("{provider_id}#{}", URL_SAFE_NO_PAD.encode(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_ids_keep_legacy_credentials_and_increment_new_accounts() {
        let legacy = json!({ "codex": "sealed" });
        assert_eq!(next_account_id(&legacy, "codex"), "codex#2");
        assert_eq!(provider_id_from_account_id("codex#2"), "codex");

        let empty = json!({});
        assert_eq!(next_account_id(&empty, "kimi_code"), "kimi_code#1");
    }

    #[test]
    fn corrupted_store_is_rejected_instead_of_treated_as_empty() {
        assert!(parse_store("{broken").is_err());
        assert!(parse_store("[]").is_err());
        assert!(parse_store(r#"{"codex":"sealed"}"#).is_ok());
    }

    #[test]
    fn private_write_replaces_contents_atomically() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("settings.json");
        write_private(&path, b"first").expect("first write");
        write_private(&path, b"second").expect("second write");
        assert_eq!(std::fs::read(&path).expect("read"), b"second");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
