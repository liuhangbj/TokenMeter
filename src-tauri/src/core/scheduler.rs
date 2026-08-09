//! 调度器 —— 零数据库版 + 智能刷新（Core 层，不依赖 Tauri）。
//!
//! 用户 2026-08-02 决定：砍掉日/周/月消耗统计，不落 SQLite；抓取结果只进内存缓存。
//! 刷新逻辑（同日定）：
//!   - 点开面板 → 立即刷新一次（前端 command 触发 `ctl.trigger_refresh()`）
//!   - 否则按设置的后台间隔刷新（`ctl.interval_tx` 广播，改设置即时生效）
//!
//! 用 `tokio::select!` 同时等待「间隔到点」与「立即刷新」两个信号。
//! 仅对已配置凭证的 provider 抓取；无凭证的跳过。
//!
//! 2026-08-02 修订：各 provider 并发抓取（独立 task，互不阻塞）；单个 provider
//! 失败时保留缓存旧数据并标记 NetworkError + last_error，UI 显示"数据已过期"，
//! 而不是静默展示越来越旧的数据。
//!
//! 平台解耦：抓取完成通过 `notify` 回调通知平台层（由 platform/main 注入
//! "snapshots-updated" 事件），Core 不感知 Tauri。

use crate::core::providers::{self, Credential, HealthStatus, ProviderSnapshot};
use crate::core::scheduler_ctl::SchedulerCtl;
use crate::core::store;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// 内存态最新快照表：account_id → 最近一次抓取结果。仅存活于进程内，不写文件。
pub type Snapshots = Arc<RwLock<HashMap<String, ProviderSnapshot>>>;

pub fn new_snapshots() -> Snapshots {
    Arc::new(RwLock::new(HashMap::new()))
}

/// 从凭证中提取不泄露秘密的账号识别。平台响应没有昵称时，仍可区分不同 API Key / 账号。
fn account_label_from_credential(cred: &Credential) -> Option<String> {
    let data = &cred.data;
    string_field(data, "account_label")
        .map(str::to_string)
        .or_else(|| string_field(data, "account_id").map(str::to_string))
        .or_else(|| string_field(data, "secret_id").map(|id| format!("SecretId {id}")))
        .or_else(|| string_field(data, "api_key").map(|key| compact_identifier("API Key", key, 4)))
}

fn string_field<'a>(data: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    data.get(key)
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
}

fn compact_identifier(kind: &str, value: &str, suffix_len: usize) -> String {
    let suffix: String = value
        .chars()
        .rev()
        .take(suffix_len)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{kind} · …{suffix}")
}

const IDENTITY_REFRESH_RETRY_SECS: i64 = 24 * 60 * 60;

fn needs_identity_refresh(provider_id: &str, cred: &Credential, now: i64) -> bool {
    if provider_id != "codex" {
        return false;
    }
    let data = &cred.data;
    let has_identity =
        string_field(data, "id_token").is_some() || string_field(data, "account_label").is_some();
    let has_refresh_token = string_field(data, "refresh_token").is_some();
    let last_attempt = data
        .get("identity_refresh_attempted_at")
        .and_then(|value| value.as_i64())
        .unwrap_or(0);
    !has_identity
        && has_refresh_token
        && now.saturating_sub(last_attempt) >= IDENTITY_REFRESH_RETRY_SECS
}

fn mark_identity_refresh_attempt(cred: &mut Credential, now: i64) {
    if let Some(data) = cred.data.as_object_mut() {
        data.insert(
            "identity_refresh_attempted_at".to_string(),
            serde_json::json!(now),
        );
    }
}

/// 抓取单个 provider，遇 AuthExpired 自动 refresh 后重试一次。
/// 返回 Err 表示抓取失败（调用方把缓存旧数据标记为 NetworkError）。
async fn fetch_one(
    p: &Arc<dyn providers::Provider>,
    account_id: &str,
) -> Result<ProviderSnapshot, String> {
    let mut cred =
        store::load_credential(account_id).ok_or_else(|| format!("{} 无凭证", account_id))?;
    let now = chrono::Utc::now().timestamp();
    if needs_identity_refresh(p.id(), &cred, now) {
        log::info!("{} ({}) 补全账号身份字段", p.id(), account_id);
        match p.refresh(&cred).await {
            Ok(Some(mut refreshed)) => {
                mark_identity_refresh_attempt(&mut refreshed, now);
                if let Err(error) = store::save_credential(account_id, &refreshed) {
                    log::warn!("{} ({}) 身份补全凭证保存失败: {error}", p.id(), account_id);
                }
                cred = refreshed;
            }
            Ok(None) => {
                mark_identity_refresh_attempt(&mut cred, now);
                let _ = store::save_credential(account_id, &cred);
                log::warn!("{} ({}) 身份补全未返回新凭证，稍后重试", p.id(), account_id);
            }
            Err(error) => {
                mark_identity_refresh_attempt(&mut cred, now);
                let _ = store::save_credential(account_id, &cred);
                log::warn!("{} ({}) 身份补全失败: {error}", p.id(), account_id);
            }
        }
    }
    let mut snapshot = match p.fetch(&cred).await {
        Ok(snap) if snap.status == HealthStatus::AuthExpired => {
            log::info!("{} ({}) 凭证过期，尝试刷新", p.id(), account_id);
            match p.refresh(&cred).await {
                Ok(Some(new_cred)) => {
                    // 刷新成功：存新凭证，重新抓取
                    if let Err(e) = store::save_credential(account_id, &new_cred) {
                        log::warn!("{} ({}) 刷新后凭证保存失败: {e}", p.id(), account_id);
                    }
                    match p.fetch(&new_cred).await {
                        Ok(snap2) => {
                            log::info!("{} ({}) 刷新后抓取成功", p.id(), account_id);
                            snap2
                        }
                        Err(e) => {
                            log::warn!("{} ({}) 刷新后仍失败: {e}", p.id(), account_id);
                            return Err(format!("刷新后抓取失败: {e}"));
                        }
                    }
                }
                _ => {
                    log::warn!("{} ({}) 刷新失败，需重新授权", p.id(), account_id);
                    snap // 保留 AuthExpired 快照，让前端提示重新授权
                }
            }
        }
        Ok(snap) => {
            log::info!("{} ({}) 抓取成功", p.id(), account_id);
            snap
        }
        Err(e) => {
            log::warn!("{} ({}) 抓取失败: {e}", p.id(), account_id);
            return Err(e.to_string());
        }
    };
    snapshot.account_id = account_id.to_string();
    if snapshot.account_label.is_none() {
        snapshot.account_label = account_label_from_credential(&cred);
    }
    Ok(snapshot)
}

/// 对所有已配置凭证的 provider 做一次全量抓取（并发），写入内存缓存。
/// 单个 provider 失败不影响其他；失败时保留旧数据并标记 NetworkError + last_error。
/// 完成后调用 `notify` 通知平台层刷新前端。
async fn fetch_all(cache: &Snapshots, notify: &(dyn Fn() + Send + Sync)) {
    let providers = providers::registry();
    let mut set = tokio::task::JoinSet::new();
    for account in store::configured_accounts() {
        let Some(p) = providers
            .iter()
            .find(|p| p.id() == account.provider_id)
            .cloned()
        else {
            log::warn!("{}: 未知 provider，跳过抓取", account.account_id);
            continue;
        };
        set.spawn(async move {
            let account_id = account.account_id;
            let result = fetch_one(&p, &account_id).await;
            (account_id, result)
        });
    }
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((id, Ok(snap))) => {
                cache.write().unwrap().insert(id.to_string(), snap);
            }
            Ok((id, Err(err))) => {
                log::warn!("{id} 刷新失败: {err}");
                let mut cache = cache.write().unwrap();
                if let Some(old) = cache.get_mut(&id) {
                    old.status = HealthStatus::NetworkError;
                    old.last_error = Some(err);
                }
            }
            Err(e) => log::error!("provider 任务 panic: {e}"),
        }
    }
    notify();
}

/// 启动调度循环：先抓一次，然后按间隔 / 立即刷新信号持续工作。
pub async fn run(cache: Snapshots, ctl: SchedulerCtl, notify: impl Fn() + Send + Sync + 'static) {
    let mut interval_rx = ctl.interval_tx.subscribe();
    let notify_ref: &(dyn Fn() + Send + Sync) = &notify;

    // 启动即先抓一次，避免首屏空等一个周期
    fetch_all(&cache, notify_ref).await;

    loop {
        let interval_secs = *interval_rx.borrow();
        let sleep = tokio::time::sleep(Duration::from_secs(interval_secs.max(30)));
        tokio::pin!(sleep);

        tokio::select! {
            // 后台间隔到点
            _ = &mut sleep => {
                fetch_all(&cache, notify_ref).await;
            }
            // 面板触发立即刷新
            _ = ctl.refresh_now.notified() => {
                log::info!("面板触发立即刷新");
                fetch_all(&cache, notify_ref).await;
            }
            // 间隔被修改：立即采用新值进入下一轮（不强制抓取）
            _ = interval_rx.changed() => {
                log::info!("刷新间隔已更新为 {} 秒", *interval_rx.borrow());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credential(data: serde_json::Value) -> Credential {
        Credential { data }
    }

    #[test]
    fn codex_identity_refresh_is_one_time_with_daily_retry() {
        let now = 2 * IDENTITY_REFRESH_RETRY_SECS;
        let old = credential(serde_json::json!({
            "refresh_token": "refresh",
            "account_id": "acct_123"
        }));
        assert!(needs_identity_refresh("codex", &old, now));
        assert!(!needs_identity_refresh("kimi_code", &old, now));

        let mut attempted = old.clone();
        mark_identity_refresh_attempt(&mut attempted, now);
        assert!(!needs_identity_refresh("codex", &attempted, now + 60));
        assert!(needs_identity_refresh(
            "codex",
            &attempted,
            now + IDENTITY_REFRESH_RETRY_SECS
        ));

        let enriched = credential(serde_json::json!({
            "refresh_token": "refresh",
            "account_label": "Ada"
        }));
        assert!(!needs_identity_refresh("codex", &enriched, now));
    }
}
