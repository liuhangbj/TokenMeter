//! Tauri commands —— IPC 薄层（三层架构最外层）。
//!
//! 只做参数组装与平台副作用（开机启动、窗口操作），
//! 业务逻辑全部在 `crate::core`，平台差异在 `crate::platform`。

use crate::core::oauth_codex;
use crate::core::oauth_device;
use crate::core::providers::{self, AuthSpec, Credential, ProviderSnapshot};
use crate::core::scheduler::Snapshots;
use crate::core::scheduler_ctl::SchedulerCtl;
use crate::core::settings::{self, Settings};
use crate::core::store;
use serde::Serialize;
use std::collections::HashMap;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_autostart::ManagerExt as _;

/// 一个可添加供应商的视图（驱动添加向导 UI）
#[derive(Serialize)]
pub struct AddableProvider {
    pub id: String,
    pub display_name: String,
    pub auth_spec: AuthSpec,
}

/// 拉取所有已抓取 provider 的最新内存快照（App 运行期数据，关闭即丢）。
#[tauri::command]
pub fn get_snapshots(cache: State<Snapshots>) -> Vec<ProviderSnapshot> {
    cache.read().unwrap().values().cloned().collect()
}

/// 列出「添加供应商」入口可见的 provider（含 auth_spec，驱动动态表单）。
#[tauri::command]
pub fn list_addable_providers() -> Vec<AddableProvider> {
    let list = providers::addable_registry()
        .into_iter()
        .map(|p| AddableProvider {
            id: p.id().to_string(),
            display_name: p.display_name().to_string(),
            auth_spec: p.auth_spec(),
        })
        .collect::<Vec<_>>();
    log::info!("list_addable_providers 返回 {} 个 provider", list.len());
    list
}

/// 面板被打开：立即触发一次后台刷新（先返旧数据，后台刷新完成后前端再拉）。
#[tauri::command]
pub fn on_panel_open(ctl: State<SchedulerCtl>) {
    ctl.trigger_refresh();
}

/// 读取设置（core 层文件存储）。
#[tauri::command]
pub fn get_settings() -> Settings {
    settings::load()
}

/// 保存设置：刷新间隔广播给 scheduler；开机启动即时生效（平台副作用）。
#[tauri::command]
pub fn set_settings(
    app: AppHandle,
    ctl: State<SchedulerCtl>,
    settings: Settings,
) -> Result<(), String> {
    ctl.set_interval(settings.refresh_interval_secs);
    settings::save(&settings).map_err(|e| e.to_string())?;
    apply_autostart(&app, settings.launch_at_login)?;
    Ok(())
}

/// 可选刷新间隔档位（秒），供前端下拉。
#[tauri::command]
pub fn interval_options() -> Vec<u64> {
    settings::INTERVAL_OPTIONS.to_vec()
}

/// 应用开机启动设置到系统（登录项/注册表）。
fn apply_autostart(app: &AppHandle, enable: bool) -> Result<(), String> {
    let mgr = app.autolaunch();
    let res = if enable { mgr.enable() } else { mgr.disable() };
    res.map_err(|e| e.to_string())
}

/// 保存 API Key / CloudSecret 类凭证：组装 Credential → 加密存储 → 立即 fetch 验证。
#[tauri::command]
pub async fn save_api_key_provider(
    app: AppHandle,
    ctl: State<'_, SchedulerCtl>,
    provider_id: String,
    fields: HashMap<String, String>,
) -> Result<(), String> {
    let providers = providers::registry();
    let p = providers
        .iter()
        .find(|p| p.id() == provider_id)
        .ok_or_else(|| format!("未知 provider: {provider_id}"))?;

    let data = serde_json::to_value(&fields).map_err(|e| e.to_string())?;
    let cred = Credential { data };

    // 先验证凭证可用（真实抓取一次），失败则不落盘
    p.fetch(&cred).await.map_err(|e| format!("凭证验证失败：{e}"))?;

    store::save_credential(&provider_id, &cred).map_err(|e| e.to_string())?;
    ctl.trigger_refresh(); // 立即刷新面板数据
    let _ = app; // 保留句柄（未来可用于其他副作用）
    Ok(())
}

/// 探测本机 CLI 凭证（Codex ~/.codex/auth.json、Kimi ~/.kimi/...），命中则一键导入。
#[tauri::command]
pub async fn import_local_credential(
    ctl: State<'_, SchedulerCtl>,
    provider_id: String,
) -> Result<bool, String> {
    let providers = providers::registry();
    let p = providers
        .iter()
        .find(|p| p.id() == provider_id)
        .ok_or_else(|| format!("未知 provider: {provider_id}"))?;

    match p.detect_local().await {
        Some(cred) => {
            p.fetch(&cred).await.map_err(|e| format!("本机凭证已失效：{e}"))?;
            store::save_credential(&provider_id, &cred).map_err(|e| e.to_string())?;
            ctl.trigger_refresh();
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Kimi 设备码授权：第一步，请求设备码（返回 user_code + verify_url 给前端展示）。
#[tauri::command]
pub async fn kimi_device_start() -> Result<oauth_device::DeviceAuthStart, String> {
    oauth_device::start().await.map_err(|e| e.to_string())
}

/// Kimi 设备码授权：第二步，轮询直到用户授权完成，存凭证并刷新。
#[tauri::command]
pub async fn kimi_device_poll(
    ctl: State<'_, SchedulerCtl>,
    device_code: String,
    interval_secs: u64,
) -> Result<(), String> {
    let data = oauth_device::poll_until_authorized(&device_code, interval_secs)
        .await
        .map_err(|e| e.to_string())?;
    let cred = Credential { data };
    store::save_credential("kimi_code", &cred).map_err(|e| e.to_string())?;
    ctl.trigger_refresh();
    Ok(())
}

/// Codex 设备码授权：第一步，请求设备码（返回 user_code + verify_url 给前端展示）。
#[tauri::command]
pub async fn codex_device_start() -> Result<oauth_codex::CodexDeviceStart, String> {
    oauth_codex::start().await.map_err(|e| e.to_string())
}

/// Codex 设备码授权：第二步，轮询直到授权完成，存凭证并刷新。
#[tauri::command]
pub async fn codex_device_poll(
    ctl: State<'_, SchedulerCtl>,
    device_auth_id: String,
    user_code: String,
    interval_secs: u64,
) -> Result<(), String> {
    let data = oauth_codex::poll_until_authorized(&device_auth_id, &user_code, interval_secs)
        .await
        .map_err(|e| e.to_string())?;
    let cred = Credential { data };
    store::save_credential("codex", &cred).map_err(|e| e.to_string())?;
    ctl.trigger_refresh();
    Ok(())
}

/// 移除一个已配置的 provider：删除加密凭证 + 清掉内存快照。
#[tauri::command]
pub fn remove_provider(
    app: AppHandle,
    cache: State<'_, Snapshots>,
    provider_id: String,
) -> Result<(), String> {
    store::delete_credential(&provider_id).map_err(|e| e.to_string())?;
    cache.write().unwrap().remove(&provider_id);
    let _ = app.emit("snapshots-updated", ());
    log::info!("已移除 provider: {provider_id}");
    Ok(())
}

/// 是否已配置过任何 provider（前端区分"加载中"和"还没有添加供应商"）。
#[tauri::command]
pub fn has_configured_providers() -> bool {
    !store::configured_provider_ids().is_empty()
}

/// 前端"退出应用"：先置 QUITTING 标志再退出，放行 main.rs 的 ExitRequested 守卫。
#[tauri::command]
pub fn quit_app(app: AppHandle) {
    crate::QUITTING.store(true, std::sync::atomic::Ordering::Relaxed);
    app.exit(0);
}

/// 前端 JS 错误上报（写入后端日志，便于远程排查空白页等）。
#[tauri::command]
pub fn log_frontend_error(msg: String) {
    log::error!("[frontend] {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addable_providers_are_nonempty_and_serializable() {
        let list = list_addable_providers();
        assert_eq!(list.len(), 6, "应返回 6 个可添加 provider");
        let json = serde_json::to_string(&list).expect("AddableProvider 序列化失败");
        assert!(json.contains("\"kind\":\"oauth\""));
        assert!(json.contains("\"kind\":\"api_key\""));
    }
}
