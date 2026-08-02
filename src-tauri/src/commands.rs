//! Tauri commands —— 暴露给前端（托盘面板 / 添加向导 / 设置）的接口。

use crate::oauth_codex;
use crate::oauth_device;
use crate::providers::{self, AuthSpec, Credential, ProviderSnapshot};
use crate::scheduler::Snapshots;
use crate::scheduler_ctl::SchedulerCtl;
use crate::settings::{self, Settings};
use crate::store::keychain;
use serde::Serialize;
use std::collections::HashMap;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

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
    providers::addable_registry()
        .into_iter()
        .map(|p| AddableProvider {
            id: p.id().to_string(),
            display_name: p.display_name().to_string(),
            auth_spec: p.auth_spec(),
        })
        .collect()
}

/// 面板被打开：立即触发一次后台刷新，并返回当前快照（先返旧数据，后台刷新完成后前端再拉）。
#[tauri::command]
pub fn on_panel_open(ctl: State<SchedulerCtl>) {
    ctl.trigger_refresh();
}

/// 自适应面板高度：前端量出内容实际高度后调用，把窗口缩放到该高度（封顶 800px）。
/// 内容短则不滚动、窗口跟着内容走；超过 800 才由 CSS overflow 出滚动条。
#[tauri::command]
pub fn resize_popover(app: AppHandle, height: f64) -> Result<(), String> {
    let Some(w) = app.get_webview_window("popover") else {
        return Err("popover 窗口不存在".to_string());
    };
    let h = height.clamp(120.0, 800.0);
    let size = tauri::LogicalSize::new(380.0, h);
    w.set_size(tauri::Size::Logical(size)).map_err(|e| e.to_string())
}

/// 读取设置。
#[tauri::command]
pub fn get_settings(app: AppHandle) -> Settings {
    settings::load(&app)
}

/// 保存设置（开机启动即时生效；刷新间隔广播给 scheduler 即时采用）。
#[tauri::command]
pub fn set_settings(app: AppHandle, ctl: State<SchedulerCtl>, settings: Settings) -> Result<(), String> {
    ctl.set_interval(settings.refresh_interval_secs);
    settings::save(&app, &settings).map_err(|e| e.to_string())
}

/// 可选刷新间隔档位（秒），供前端下拉。
#[tauri::command]
pub fn interval_options() -> Vec<u64> {
    settings::INTERVAL_OPTIONS.to_vec()
}

// ---------- 添加供应商向导 ----------

/// 打开（或聚焦）添加供应商窗口。
#[tauri::command]
pub fn open_add_provider(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("add-provider") {
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    WebviewWindowBuilder::new(&app, "add-provider", WebviewUrl::App("index.html?add=1".into()))
        .title("添加供应商")
        .inner_size(480.0, 620.0)
        .resizable(false)
        .center()
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// 保存 API Key / CloudSecret 类凭证：组装 Credential → 写 Keychain → 立即 fetch 验证。
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

    // 先验证凭证可用（真实抓取一次），失败则不落 Keychain
    p.fetch(&cred).await.map_err(|e| format!("凭证验证失败：{e}"))?;

    keychain::save_credential(&provider_id, &cred).map_err(|e| e.to_string())?;
    ctl.trigger_refresh(); // 立即刷新面板数据
    let _ = app; // 保留句柄（未来可用于关窗）
    Ok(())
}

/// 探测本机 CLI 凭证（Codex ~/.codex/auth.json、Kimi ~/.kimi/...），命中则一键导入。
/// 返回是否成功导入。
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
            // 验证可用再存
            p.fetch(&cred).await.map_err(|e| format!("本机凭证已失效：{e}"))?;
            keychain::save_credential(&provider_id, &cred).map_err(|e| e.to_string())?;
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

/// Kimi 设备码授权：第二步，轮询直到用户授权完成，存 Keychain 并刷新。
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
    keychain::save_credential("kimi_code", &cred).map_err(|e| e.to_string())?;
    ctl.trigger_refresh();
    Ok(())
}

/// Codex PKCE 浏览器授权：后台跑完整流程（开浏览器→接回调→换 token→存凭证），
/// command 立即返回，授权完成后 emit "codex-oauth-done" 事件通知前端。
/// 避免前端 await 一个几十秒的 command 导致 Promise 不 resolve、界面卡"等待授权"。
#[tauri::command]
pub fn codex_oauth_start(app: AppHandle, ctl: State<'_, SchedulerCtl>) -> Result<(), String> {
    let ctl = ctl.inner().clone();
    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = oauth_codex::run_flow(|url| async move {
            let _ = open_url_in_browser(&url);
        })
        .await;

        match result {
            Ok(data) => {
                let cred = Credential { data };
                match keychain::save_credential("codex", &cred) {
                    Ok(_) => {
                        ctl.trigger_refresh();
                        let _ = app2.emit("codex-oauth-done", serde_json::json!({"ok": true}));
                    }
                    Err(e) => {
                        let _ = app2.emit("codex-oauth-done", serde_json::json!({"ok": false, "error": e.to_string()}));
                    }
                }
            }
            Err(e) => {
                let _ = app2.emit("codex-oauth-done", serde_json::json!({"ok": false, "error": e.to_string()}));
            }
        }
    });
    Ok(())
}

/// 用系统默认浏览器打开 URL（macOS open / Windows start / Linux xdg-open）。
fn open_url_in_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let (cmd, args) = ("open", vec![url]);
    #[cfg(target_os = "windows")]
    let (cmd, args) = ("cmd", vec!["/C", "start", "", url]);
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let (cmd, args) = ("xdg-open", vec![url]);
    std::process::Command::new(cmd).args(&args).spawn()?;
    Ok(())
}
