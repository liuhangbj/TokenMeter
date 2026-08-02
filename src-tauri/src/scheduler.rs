//! 调度器 —— 零数据库版 + 智能刷新。
//!
//! 用户 2026-08-02 决定：砍掉日/周/月消耗统计，不落 SQLite；抓取结果只进内存缓存。
//! 刷新逻辑（同日定）：
//!   - 点开面板 → 立即刷新一次（前端 command 触发 `ctl.trigger_refresh()`）
//!   - 否则按设置的后台间隔刷新（`ctl.interval_tx` 广播，改设置即时生效）
//! 用 `tokio::select!` 同时等待「间隔到点」与「立即刷新」两个信号。
//! 仅对已配置凭证的 provider 抓取；无凭证的跳过。
//!
//! 2026-08-02 修订：各 provider 并发抓取（独立 task，互不阻塞）；单个 provider
//! 失败时保留缓存旧数据并标记 NetworkError + last_error，UI 显示"数据已过期"，
//! 而不是静默展示越来越旧的数据。

use crate::providers::{self, HealthStatus, ProviderSnapshot};
use crate::scheduler_ctl::SchedulerCtl;
use crate::store::keychain;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tauri::{Emitter, Manager};

/// 内存态最新快照表：provider_id → 最近一次抓取结果。仅存活于进程内，不写文件。
pub type Snapshots = Arc<RwLock<HashMap<String, ProviderSnapshot>>>;

pub fn new_snapshots() -> Snapshots {
    Arc::new(RwLock::new(HashMap::new()))
}

/// 抓取单个 provider，遇 AuthExpired 自动 refresh 后重试一次。
/// 返回 Err 表示抓取失败（调用方把缓存旧数据标记为 NetworkError）。
async fn fetch_one(p: &Arc<dyn providers::Provider>) -> Result<ProviderSnapshot, String> {
    let cred = keychain::load_credential(p.id())
        .ok_or_else(|| format!("{} 无凭证", p.id()))?;
    match p.fetch(&cred).await {
        Ok(snap) if snap.status == HealthStatus::AuthExpired => {
            log::info!("{} 凭证过期，尝试刷新", p.id());
            match p.refresh(&cred).await {
                Ok(Some(new_cred)) => {
                    // 刷新成功：存新凭证，重新抓取
                    if let Err(e) = keychain::save_credential(p.id(), &new_cred) {
                        log::warn!("{} 刷新后凭证保存失败: {e}", p.id());
                    }
                    match p.fetch(&new_cred).await {
                        Ok(snap2) => {
                            log::info!("{} 刷新后抓取成功", p.id());
                            Ok(snap2)
                        }
                        Err(e) => {
                            log::warn!("{} 刷新后仍失败: {e}", p.id());
                            Err(format!("刷新后抓取失败: {e}"))
                        }
                    }
                }
                _ => {
                    log::warn!("{} 刷新失败，需重新授权", p.id());
                    Ok(snap) // 保留 AuthExpired 快照，让前端提示重新授权
                }
            }
        }
        Ok(snap) => {
            log::info!("{} 抓取成功", p.id());
            Ok(snap)
        }
        Err(e) => {
            log::warn!("{} 抓取失败: {e}", p.id());
            Err(e.to_string())
        }
    }
}

/// 对所有已配置凭证的 provider 做一次全量抓取（并发），写入内存缓存，并通知前端。
/// 单个 provider 失败不影响其他；失败时保留旧数据并标记 NetworkError + last_error。
async fn fetch_all(handle: &tauri::AppHandle, cache: &Snapshots) {
    let providers = providers::registry();
    let mut set = tokio::task::JoinSet::new();
    for p in providers {
        if keychain::load_credential(p.id()).is_some() {
            let p2 = p.clone();
            set.spawn(async move {
                let id = p2.id();
                let result = fetch_one(&p2).await;
                (id, result)
            });
        }
    }
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((id, Ok(snap))) => {
                cache.write().unwrap().insert(id.to_string(), snap);
            }
            Ok((id, Err(err))) => {
                log::warn!("{id} 刷新失败: {err}");
                let mut cache = cache.write().unwrap();
                if let Some(old) = cache.get_mut(id) {
                    old.status = HealthStatus::NetworkError;
                    old.last_error = Some(err);
                }
            }
            Err(e) => log::error!("provider 任务 panic: {e}"),
        }
    }
    // 通知前端快照已更新（面板若开着则自动刷新显示）
    let _ = handle.emit("snapshots-updated", ());
}

pub async fn run(handle: tauri::AppHandle) {
    let cache = handle.state::<Snapshots>().inner().clone();
    let ctl = handle.state::<SchedulerCtl>().inner().clone();
    let mut interval_rx = ctl.interval_tx.subscribe();

    // 启动即先抓一次，避免首屏空等一个周期
    fetch_all(&handle, &cache).await;

    loop {
        let interval_secs = *interval_rx.borrow();
        let sleep = tokio::time::sleep(Duration::from_secs(interval_secs.max(30)));
        tokio::pin!(sleep);

        tokio::select! {
            // 后台间隔到点
            _ = &mut sleep => {
                fetch_all(&handle, &cache).await;
            }
            // 面板触发立即刷新
            _ = ctl.refresh_now.notified() => {
                log::info!("面板触发立即刷新");
                fetch_all(&handle, &cache).await;
            }
            // 间隔被修改：立即采用新值进入下一轮（不强制抓取）
            _ = interval_rx.changed() => {
                log::info!("刷新间隔已更新为 {} 秒", *interval_rx.borrow());
            }
        }
    }
}
