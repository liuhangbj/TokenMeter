//! 调度器 —— 零数据库版 + 智能刷新。
//!
//! 用户 2026-08-02 决定：砍掉日/周/月消耗统计，不落 SQLite；抓取结果只进内存缓存。
//! 刷新逻辑（同日定）：
//!   - 点开面板 → 立即刷新一次（前端 command 触发 `ctl.trigger_refresh()`）
//!   - 否则按设置的后台间隔刷新（`ctl.interval_tx` 广播，改设置即时生效）
//! 用 `tokio::select!` 同时等待「间隔到点」与「立即刷新」两个信号。
//! 仅对已配置凭证的 provider 抓取；无凭证的跳过。

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
async fn fetch_one(p: &Arc<dyn providers::Provider>) -> Option<ProviderSnapshot> {
    let cred = keychain::load_credential(p.id())?;
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
                            Some(snap2)
                        }
                        Err(e) => {
                            log::warn!("{} 刷新后仍失败: {e}", p.id());
                            Some(snap) // 返回带 AuthExpired 的旧快照让前端提示重新授权
                        }
                    }
                }
                _ => {
                    log::warn!("{} 刷新失败，需重新授权", p.id());
                    Some(snap)
                }
            }
        }
        Ok(snap) => {
            log::info!("{} 抓取成功", p.id());
            Some(snap)
        }
        Err(e) => {
            log::warn!("{} 抓取失败: {e}", p.id());
            None
        }
    }
}

/// 对所有已配置凭证的 provider 做一次全量抓取，写入内存缓存，并通知前端。
async fn fetch_all(handle: &tauri::AppHandle, cache: &Snapshots) {
    let providers = providers::registry();
    for p in &providers {
        if keychain::load_credential(p.id()).is_some() {
            if let Some(snap) = fetch_one(p).await {
                cache.write().unwrap().insert(p.id().to_string(), snap);
            }
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
