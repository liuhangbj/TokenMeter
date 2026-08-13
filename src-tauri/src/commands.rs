//! Tauri commands —— IPC 薄层（三层架构最外层）。
//!
//! 只做参数组装与平台副作用（开机启动、窗口操作），
//! 业务逻辑全部在 `crate::core`，平台差异在 `crate::platform`。

use crate::core::oauth_codex;
use crate::core::oauth_device;
use crate::core::oauth_google;
use crate::core::oauth_pkce;
use crate::core::providers::{
    self, presentation::AccountCardModel, presentation::BrandStyle, AddAccountType, AuthSpec,
    Credential, HealthStatus, Provider,
};
use crate::core::scheduler::Snapshots;
use crate::core::scheduler_ctl::SchedulerCtl;
use crate::core::settings::{self, Appearance, FloatingOrbSettings, Settings, Theme};
use crate::core::store;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt as _;

static SETTINGS_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn settings_write_lock() -> Result<MutexGuard<'static, ()>, String> {
    SETTINGS_WRITE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "设置写入锁已损坏，已停止写入".to_string())
}

/// 一个可添加供应商的视图（驱动添加向导 UI）
#[derive(Serialize)]
pub struct AddableProvider {
    pub id: String,
    pub product_name: String,
    pub description: String,
    pub account_type: AddAccountType,
    pub vendor: AddableVendor,
    pub brand: BrandStyle,
    pub auth_spec: AuthSpec,
    pub supports_local_import: bool,
}

#[derive(Serialize)]
pub struct AddableVendor {
    pub id: String,
    pub display_name: String,
    pub brand: BrandStyle,
}

/// 将最新内存快照通过各 Provider 的字段映射转换为标准卡片契约。
/// 前端只消费 AccountCardModel，不再判断具体供应商。
#[tauri::command]
pub fn get_account_cards(cache: State<Snapshots>) -> Vec<AccountCardModel> {
    let registry = providers::registry();
    let snapshots = cache.read().unwrap().values().cloned().collect::<Vec<_>>();
    snapshots
        .iter()
        .filter_map(|snapshot| {
            let provider = registry
                .iter()
                .find(|provider| provider.id() == snapshot.provider_id);
            match provider {
                Some(provider) => Some(provider.present(snapshot)),
                None => {
                    log::warn!("{} 没有对应的卡片映射，跳过", snapshot.provider_id);
                    None
                }
            }
        })
        .collect()
}

/// 列出「添加供应商」入口可见的 provider（含 auth_spec，驱动动态表单）。
#[tauri::command]
pub fn list_addable_providers() -> Vec<AddableProvider> {
    let mut entries = providers::addable_registry();
    entries.sort_by_key(|provider| {
        (
            provider.brand().vendor().order(),
            provider.add_account_type().order(),
        )
    });
    let list = entries
        .into_iter()
        .map(|provider| {
            let vendor = provider.brand().vendor();
            AddableProvider {
                id: provider.id().to_string(),
                product_name: provider.add_product_name().to_string(),
                description: provider.add_description().to_string(),
                account_type: provider.add_account_type(),
                vendor: AddableVendor {
                    id: vendor.id().to_string(),
                    display_name: vendor.display_name().to_string(),
                    brand: vendor.brand().style(),
                },
                brand: provider.brand().style(),
                auth_spec: provider.auth_spec(),
                supports_local_import: provider.supports_local_import(),
            }
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

/// 保存通用设置。只处理设置面板拥有的字段，避免昵称/排序携带旧副本
/// 覆盖新值。副作用与文件保存按可回滚顺序执行，成功后才更新运行时调度器。
#[tauri::command]
pub fn set_general_settings(
    app: AppHandle,
    ctl: State<SchedulerCtl>,
    launch_at_login: bool,
    refresh_interval_secs: u64,
    theme: Theme,
    appearance: Appearance,
    floating_orb_enabled: bool,
) -> Result<Settings, String> {
    let _guard = settings_write_lock()?;
    if !settings::INTERVAL_OPTIONS.contains(&refresh_interval_secs) {
        return Err(format!("不支持的刷新间隔: {refresh_interval_secs} 秒"));
    }

    let previous = settings::load_strict().map_err(|e| e.to_string())?;
    let mut next = previous.clone();
    next.launch_at_login = launch_at_login;
    next.refresh_interval_secs = refresh_interval_secs;
    next.theme = theme;
    next.appearance = appearance;
    next.floating_orb.enabled = floating_orb_enabled;
    let autostart_changed = previous.launch_at_login != next.launch_at_login;

    if autostart_changed {
        apply_autostart(&app, next.launch_at_login)?;
    }
    if let Err(error) = settings::save(&next) {
        if autostart_changed {
            if let Err(rollback_error) = apply_autostart(&app, previous.launch_at_login) {
                return Err(format!(
                    "保存设置失败: {error}；恢复开机启动状态也失败: {rollback_error}"
                ));
            }
        }
        return Err(error.to_string());
    }
    ctl.set_interval(next.refresh_interval_secs);
    // WebView 窗口的创建/显隐统一回主线程，避免 Windows 上从 IPC 工作线程
    // 创建透明窗口时出现无效句柄、白屏或短暂任务栏窗口。
    let floating_orb = next.floating_orb.clone();
    let floating_theme = next.theme;
    let floating_app = app.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        crate::platform::floating_orb::sync_visibility(
            &floating_app,
            &floating_orb,
            floating_theme,
        );
    }) {
        log::error!("同步悬浮球可见性失败: {error}");
    }
    let _ = app.emit("settings-updated", &next);
    Ok(next)
}

/// 保存悬浮球当前激活账号。它只影响球组切换，不改变卡片顺序。
#[tauri::command]
pub fn set_floating_orb_active_account(
    app: AppHandle,
    account_id: String,
) -> Result<Settings, String> {
    let _guard = settings_write_lock()?;
    let mut current = settings::load_strict().map_err(|error| error.to_string())?;
    current.floating_orb.active_account_id = if account_id.trim().is_empty() {
        None
    } else {
        Some(account_id)
    };
    settings::save(&current).map_err(|error| error.to_string())?;
    let _ = app.emit("settings-updated", &current);
    Ok(current)
}

/// 收起/展开悬浮球并记住状态。窗口尺寸由账号数量推导，前端不传像素值。
#[tauri::command]
pub fn set_floating_orb_collapsed(
    app: AppHandle,
    collapsed: bool,
    account_count: usize,
) -> Result<Settings, String> {
    let _guard = settings_write_lock()?;
    let mut current = settings::load_strict().map_err(|error| error.to_string())?;
    current.floating_orb.collapsed = collapsed;
    settings::save(&current).map_err(|error| error.to_string())?;

    let floating_orb = current.floating_orb.clone();
    let floating_theme = current.theme;
    let floating_app = app.clone();
    app.run_on_main_thread(move || {
        crate::platform::floating_orb::apply_layout(
            &floating_app,
            &floating_orb,
            account_count,
            floating_theme,
        );
    })
    .map_err(|error| error.to_string())?;
    let _ = app.emit("settings-updated", &current);
    Ok(current)
}

/// 账号增加或删除后，按当前收起状态和账号数量重新计算窗口高度。
#[tauri::command]
pub fn sync_floating_orb_layout(app: AppHandle, account_count: usize) -> Result<(), String> {
    let current = settings::load_strict().map_err(|error| error.to_string())?;
    let floating_orb = current.floating_orb;
    let floating_theme = current.theme;
    let floating_app = app.clone();
    app.run_on_main_thread(move || {
        crate::platform::floating_orb::apply_layout(
            &floating_app,
            &floating_orb,
            account_count,
            floating_theme,
        );
    })
    .map_err(|error| error.to_string())
}

/// 悬浮组件右键菜单的唯一动作：隐藏窗口并关闭设置开关。
/// 用户可从主面板设置中再次开启。
#[tauri::command]
pub fn hide_floating_orb(app: AppHandle) -> Result<Settings, String> {
    let _guard = settings_write_lock()?;
    let mut current = settings::load_strict().map_err(|error| error.to_string())?;
    current.floating_orb.enabled = false;
    settings::save(&current).map_err(|error| error.to_string())?;
    if let Some(window) = app.get_webview_window(crate::platform::floating_orb::WINDOW_LABEL) {
        window.hide().map_err(|error| error.to_string())?;
    }
    let _ = app.emit("settings-updated", &current);
    Ok(current)
}

/// 原生拖动结束后执行左右吸附，并把显示器、边缘和纵向比例持久化。
#[tauri::command]
pub fn snap_floating_orb(app: AppHandle) -> Result<FloatingOrbSettings, String> {
    let snapped = crate::platform::floating_orb::snap_and_describe(&app)?;
    let _guard = settings_write_lock()?;
    let mut current = settings::load_strict().map_err(|error| error.to_string())?;
    current.floating_orb.edge = snapped.edge;
    current.floating_orb.y_ratio = snapped.y_ratio;
    current.floating_orb.monitor_name = snapped.monitor_name;
    settings::save(&current).map_err(|error| error.to_string())?;
    let _ = app.emit("settings-updated", &current);
    Ok(current.floating_orb)
}

/// 大球点击后打开现有主面板，并通知它滚动定位到对应账号卡片。
#[tauri::command]
pub fn open_account_from_floating_orb(app: AppHandle, account_id: String) -> Result<(), String> {
    crate::platform::tray::show_panel_near_floating_orb(&app);
    app.emit("focus-account-card", account_id)
        .map_err(|error| error.to_string())
}

/// 面板自适应高度变化时，由原生层同时保持当前位置或重新应用右上角锚点。
#[tauri::command]
pub fn resize_panel(app: AppHandle, height: f64) -> Result<(), String> {
    crate::platform::tray::resize_panel(&app, height)
}

/// 保存账号昵称，只合并该字段，不触碰刷新间隔或开机启动。
#[tauri::command]
pub fn set_account_nickname(
    app: AppHandle,
    account_id: String,
    nickname: String,
) -> Result<Settings, String> {
    let _guard = settings_write_lock()?;
    let mut current = settings::load_strict().map_err(|e| e.to_string())?;
    let nickname = nickname.trim();
    if nickname.is_empty() {
        current.account_nicknames.remove(&account_id);
    } else {
        current
            .account_nicknames
            .insert(account_id, nickname.to_string());
    }
    settings::save(&current).map_err(|e| e.to_string())?;
    let _ = app.emit("settings-updated", &current);
    Ok(current)
}

/// 保存卡片顺序，只合并排序字段并去掉空值和重复账号。
#[tauri::command]
pub fn set_card_order(app: AppHandle, card_order: Vec<String>) -> Result<Settings, String> {
    let _guard = settings_write_lock()?;
    let mut seen = std::collections::HashSet::new();
    let normalized = card_order
        .into_iter()
        .filter(|account_id| !account_id.trim().is_empty())
        .filter(|account_id| seen.insert(account_id.clone()))
        .collect();
    let mut current = settings::load_strict().map_err(|e| e.to_string())?;
    current.card_order = normalized;
    settings::save(&current).map_err(|e| e.to_string())?;
    let _ = app.emit("settings-updated", &current);
    Ok(current)
}

/// 可选刷新间隔档位（秒），供前端下拉。
#[tauri::command]
pub fn interval_options() -> Vec<u64> {
    settings::INTERVAL_OPTIONS.to_vec()
}

fn validated_external_url(url: &str) -> Result<String, String> {
    let parsed = reqwest::Url::parse(url).map_err(|error| format!("无效链接: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("只允许打开 http/https 链接".to_string());
    }
    Ok(parsed.to_string())
}

/// 统一由后端调用系统默认浏览器，避免纯托盘 Windows App 的前端 shell
/// plugin 无法拉起浏览器。URL 先做协议白名单校验。
#[tauri::command]
pub fn open_external(url: String) -> Result<(), String> {
    let url = validated_external_url(&url)?;
    crate::platform::open_browser(&url).map_err(|error| format!("无法打开系统浏览器: {error}"))
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
    p.fetch(&cred)
        .await
        .map_err(|e| format!("凭证验证失败：{e}"))?;

    store::create_credential(&provider_id, &cred).map_err(|e| e.to_string())?;
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
            let cred = validate_or_refresh_local_credential(p.as_ref(), cred).await?;
            store::create_credential(&provider_id, &cred).map_err(|e| e.to_string())?;
            ctl.trigger_refresh();
            Ok(true)
        }
        None => Ok(false),
    }
}

/// 本机 CLI 的 access token 可能已过期，但 refresh token 仍然有效。
/// 先验证；只有明确收到 AuthExpired 才续期并重试，网络错误不会被误报成过期。
async fn validate_or_refresh_local_credential(
    provider: &dyn Provider,
    credential: Credential,
) -> Result<Credential, String> {
    let first = provider
        .fetch(&credential)
        .await
        .map_err(|error| format!("验证本机凭证失败：{error}"))?;
    if first.status != HealthStatus::AuthExpired {
        return Ok(credential);
    }

    let refreshed = provider
        .refresh(&credential)
        .await
        .map_err(|error| format!("本机凭证自动续期失败：{error}"))?
        .ok_or_else(|| "本机凭证已过期且无法自动续期，请使用浏览器授权".to_string())?;
    let retried = provider
        .fetch(&refreshed)
        .await
        .map_err(|error| format!("续期后验证本机凭证失败：{error}"))?;
    if retried.status == HealthStatus::AuthExpired {
        return Err("本机凭证续期后仍被拒绝，请使用浏览器授权".to_string());
    }
    Ok(refreshed)
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
    store::create_credential("kimi_code", &cred).map_err(|e| e.to_string())?;
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
    store::create_credential("codex", &cred).map_err(|e| e.to_string())?;
    ctl.trigger_refresh();
    Ok(())
}

/// Claude 浏览器 OAuth：生成 PKCE 会话和授权地址。Claude 官方回调页会展示
/// `code#state`，由用户粘贴回 TokenMeter 完成交换。
#[tauri::command]
pub fn claude_oauth_start() -> Result<oauth_pkce::PkceStart, String> {
    oauth_pkce::start_claude().map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn claude_oauth_complete(
    ctl: State<'_, SchedulerCtl>,
    session_id: String,
    code: String,
) -> Result<(), String> {
    let credential = oauth_pkce::complete_claude(&session_id, &code)
        .await
        .map_err(|error| error.to_string())?;
    validate_and_store_oauth("claude", credential, &ctl).await
}

/// OpenRouter 浏览器 OAuth：启动一次性 localhost 回调监听器并生成 PKCE 地址。
#[tauri::command]
pub async fn openrouter_oauth_start() -> Result<oauth_pkce::PkceStart, String> {
    oauth_pkce::start_openrouter()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn openrouter_oauth_poll(
    ctl: State<'_, SchedulerCtl>,
    session_id: String,
) -> Result<(), String> {
    let credential = oauth_pkce::complete_openrouter(&session_id)
        .await
        .map_err(|error| error.to_string())?;
    validate_and_store_oauth("openrouter", credential, &ctl).await
}

/// Gemini Code Assist 浏览器 OAuth：loopback 回调 + PKCE。
#[tauri::command]
pub async fn gemini_oauth_start() -> Result<oauth_google::GoogleOAuthStart, String> {
    oauth_google::start()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn gemini_oauth_poll(
    ctl: State<'_, SchedulerCtl>,
    session_id: String,
) -> Result<(), String> {
    let credential = oauth_google::complete(&session_id)
        .await
        .map_err(|error| error.to_string())?;
    validate_and_store_oauth("gemini", credential, &ctl).await
}

async fn validate_and_store_oauth(
    provider_id: &str,
    credential: Credential,
    ctl: &SchedulerCtl,
) -> Result<(), String> {
    let registry = providers::registry();
    let provider = registry
        .iter()
        .find(|provider| provider.id() == provider_id)
        .ok_or_else(|| format!("未知 provider: {provider_id}"))?;
    let snapshot = provider
        .fetch(&credential)
        .await
        .map_err(|error| format!("授权成功，但账户信息验证失败：{error}"))?;
    if snapshot.status == HealthStatus::AuthExpired {
        return Err("授权凭证已被服务端拒绝，请重新授权".into());
    }
    store::create_credential(provider_id, &credential).map_err(|error| error.to_string())?;
    ctl.trigger_refresh();
    Ok(())
}

/// 移除一个已配置账号：删除该账号的加密凭证 + 清掉对应内存快照。
#[tauri::command]
pub fn remove_provider(
    app: AppHandle,
    cache: State<'_, Snapshots>,
    account_id: String,
) -> Result<(), String> {
    store::delete_credential(&account_id).map_err(|e| e.to_string())?;
    cache.write().unwrap().remove(&account_id);
    let _guard = match settings_write_lock() {
        Ok(guard) => guard,
        Err(error) => {
            log::warn!("账号 {account_id} 已删除，但无法锁定设置文件: {error}");
            let _ = app.emit("snapshots-updated", ());
            return Ok(());
        }
    };
    let mut current_settings = match settings::load_strict() {
        Ok(settings) => settings,
        Err(error) => {
            log::warn!("账号 {account_id} 已删除，但设置文件损坏，未覆盖原文件: {error}");
            let _ = app.emit("snapshots-updated", ());
            return Ok(());
        }
    };
    settings::remove_account_references(&mut current_settings, &account_id);
    if let Err(error) = settings::save(&current_settings) {
        // 凭证和缓存已经删除，设置清理失败不应让前端误判为“删除失败”。
        log::warn!("账号 {account_id} 已删除，但清理排序/昵称设置失败: {error}");
    }
    let _ = app.emit("snapshots-updated", ());
    log::info!("已移除账号: {account_id}");
    Ok(())
}

/// 是否已配置过任何 provider（前端区分"加载中"和"还没有添加供应商"）。
#[tauri::command]
pub fn has_configured_providers() -> bool {
    !store::configured_accounts().is_empty()
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
    use crate::core::providers::{BillingMode, Brand, Fidelity, ProviderSnapshot};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RefreshingProvider {
        fetches: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl Provider for RefreshingProvider {
        fn id(&self) -> &'static str {
            "mock"
        }

        fn display_name(&self) -> &'static str {
            "Mock"
        }

        fn brand(&self) -> Brand {
            Brand::OpenAI
        }

        fn billing_mode(&self) -> BillingMode {
            BillingMode::Subscription
        }

        fn auth_spec(&self) -> AuthSpec {
            AuthSpec::OAuth {
                authorize_url: "https://example.com/auth",
                token_url: "https://example.com/token",
                client_id: "mock",
                scopes: &[],
                pkce: false,
            }
        }

        async fn refresh(&self, _cred: &Credential) -> anyhow::Result<Option<Credential>> {
            Ok(Some(Credential {
                data: serde_json::json!({ "access_token": "fresh" }),
            }))
        }

        async fn fetch(&self, cred: &Credential) -> anyhow::Result<ProviderSnapshot> {
            self.fetches.fetch_add(1, Ordering::Relaxed);
            let fresh = cred
                .data
                .get("access_token")
                .and_then(|value| value.as_str())
                == Some("fresh");
            Ok(ProviderSnapshot {
                account_id: String::new(),
                account_label: None,
                provider_id: self.id().to_string(),
                display_name: self.display_name().to_string(),
                plan_name: None,
                billing: BillingMode::Subscription,
                balance: None,
                windows: vec![],
                fidelity: Fidelity::Exact,
                status: if fresh {
                    HealthStatus::Ok
                } else {
                    HealthStatus::AuthExpired
                },
                fetched_at: 0,
                last_error: None,
            })
        }
    }

    #[test]
    fn addable_providers_are_nonempty_and_serializable() {
        let list = list_addable_providers();
        assert_eq!(list.len(), 16, "应返回 16 个可添加 provider");
        assert_eq!(list[0].vendor.id, "openai");
        assert_eq!(list[0].account_type, AddAccountType::Plan);
        assert_eq!(list[0].product_name, "Codex 套餐");

        let mut vendor_counts = HashMap::new();
        for provider in &list {
            *vendor_counts
                .entry(provider.vendor.id.as_str())
                .or_insert(0) += 1;
        }
        assert_eq!(vendor_counts.get("openai"), Some(&2));
        assert_eq!(vendor_counts.get("anthropic"), Some(&2));
        assert_eq!(vendor_counts.get("openrouter"), Some(&1));
        assert_eq!(vendor_counts.get("moonshot"), Some(&2));
        assert_eq!(vendor_counts.get("deepseek"), Some(&1));
        assert_eq!(vendor_counts.get("glm"), Some(&2));
        assert_eq!(vendor_counts.get("minimax"), Some(&2));
        assert_eq!(vendor_counts.get("tencent"), Some(&1));
        assert_eq!(vendor_counts.get("google"), Some(&1));
        assert_eq!(vendor_counts.get("volcengine"), Some(&1));
        assert_eq!(vendor_counts.get("siliconflow"), Some(&1));
        for vendor_id in ["anthropic", "glm", "minimax"] {
            let account_types = list
                .iter()
                .filter(|provider| provider.vendor.id == vendor_id)
                .map(|provider| provider.account_type)
                .collect::<Vec<_>>();
            assert_eq!(
                account_types,
                vec![AddAccountType::Plan, AddAccountType::Api],
                "{vendor_id} 应按 Plan → API 顺序显示双产品入口"
            );
        }
        let tokenhub = list
            .iter()
            .find(|provider| provider.id == "tencent_tokenhub")
            .unwrap();
        assert_eq!(tokenhub.account_type, AddAccountType::Plan);
        assert!(matches!(tokenhub.auth_spec, AuthSpec::CloudSecret { .. }));

        let json = serde_json::to_string(&list).expect("AddableProvider 序列化失败");
        assert!(json.contains("\"kind\":\"oauth\""));
        assert!(json.contains("\"kind\":\"api_key\""));
        assert!(json.contains("\"account_type\":\"plan\""));
        assert!(json.contains("\"account_type\":\"api\""));
        assert!(json.contains("\"accent_dark\""));
    }

    #[test]
    fn external_links_only_allow_http_and_https() {
        assert!(validated_external_url("https://auth.openai.com/codex/device").is_ok());
        assert!(validated_external_url("http://localhost:1455/callback").is_ok());
        assert!(validated_external_url("file:///tmp/secret").is_err());
        assert!(validated_external_url("javascript:alert(1)").is_err());
    }

    #[tokio::test]
    async fn expired_local_credential_is_refreshed_and_verified_again() {
        let provider = RefreshingProvider {
            fetches: AtomicUsize::new(0),
        };
        let old = Credential {
            data: serde_json::json!({ "access_token": "expired", "refresh_token": "refresh" }),
        };

        let imported = validate_or_refresh_local_credential(&provider, old)
            .await
            .expect("refresh should recover expired local credential");
        assert_eq!(imported.data["access_token"], "fresh");
        assert_eq!(provider.fetches.load(Ordering::Relaxed), 2);
    }
}
