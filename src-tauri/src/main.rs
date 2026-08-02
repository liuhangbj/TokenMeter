#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! 组装根（三层架构）：
//! - `core`     平台无关核心（provider / 存储 / 调度 / 设置 / OAuth）
//! - `platform` 平台壳（托盘 / 浏览器 / macOS Accessory 策略）
//! - `commands` IPC 薄层；本文件只负责插件注册、窗口事件与退出守卫。

mod commands;
mod core;
mod platform;

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::MacosLauncher;

/// 用户是否明确选择了退出（托盘菜单「退出」）。
/// `ExitRequested` 只在标志置位时放行，防止 Windows 上窗口隐藏/关闭等
/// 任意事件路径把纯菜单栏进程带退出（"单击托盘图标就退出"）。
pub static QUITTING: AtomicBool = AtomicBool::new(false);

/// popover 是否曾获得焦点（失焦隐藏守卫：未获焦前的失焦不隐藏，防"刚弹就关"）。
/// Windows 上无装饰+置顶窗口 show 后可能未真正获焦就收到 Focused(false)，
/// 直接 hide 会让面板刚弹出就消失（观感"闪退"）。
static POPOVER_HAS_FOCUS: AtomicBool = AtomicBool::new(false);

#[tokio::main]
async fn main() {
    // 崩溃落盘：任何线程 panic 都写日志文件（若设置了 TOKENMETER_LOG_FILE），
    // 否则只走默认 stderr。这样 Windows 上闪退也能拿到根因。
    let log_file = std::env::var("TOKENMETER_LOG_FILE").ok();
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(path) = &log_file {
            let msg = format!("PANIC: {info}\n");
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                use std::io::Write;
                let _ = f.write_all(msg.as_bytes());
            }
        }
        default_hook(info);
    }));

    // 日志：默认输出到 stderr（GUI 子系统下不可见，无副作用）。
    // 设 TOKENMETER_LOG_FILE=<path> 时同时写文件——仅 CI/排查用，正常使用零文件。
    if let Ok(path) = std::env::var("TOKENMETER_LOG_FILE") {
        let file = std::fs::File::create(&path).expect("无法创建日志文件");
        let target = env_logger::Target::Pipe(Box::new(file));
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
            .target(target)
            .init();
    } else {
        env_logger::init();
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        // 单实例锁：纯菜单栏 App 必须单实例。重复启动时聚焦已有实例的
        // 面板（如果开着），防止多实例托盘图标互相干扰（Windows 实测有
        // 两个 tokenmeter.exe 同时在跑，点击事件混乱 → 观感"闪退"）。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 二次启动：若面板窗口存在则显示并聚焦（用户可能以为没打开）
            if let Some(w) = app.get_webview_window("popover") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshots,
            commands::list_addable_providers,
            commands::on_panel_open,
            commands::get_settings,
            commands::set_settings,
            commands::interval_options,
            commands::save_api_key_provider,
            commands::import_local_credential,
            commands::kimi_device_start,
            commands::kimi_device_poll,
            commands::codex_oauth_start,
            commands::resize_popover,
            commands::remove_provider,
            commands::has_configured_providers,
            commands::quit_app,
            commands::log_frontend_error,
        ])
        .setup(|app| {
            // 平台启动配置（macOS Accessory 策略隐藏 Dock）
            platform::setup(app);

            // 读取设置（core 文件存储），初始化调度控制（间隔广播 + 立即刷新信号）
            let current = core::settings::load();
            app.manage(core::scheduler_ctl::new_ctl(current.refresh_interval_secs));
            app.manage(core::scheduler::new_snapshots());
            platform::tray::build_tray(app.handle())?;

            // 启动即创建隐藏面板：前端在后台完成测量与尺寸定型，
            // 首次点托盘时直接以最终尺寸定位显示，杜绝"先弹再重定位"闪切。
            let _ = platform::tray::get_or_create_panel(app.handle());

            // 调度器运行：core 层不感知 Tauri，抓取完成通过闭包回发前端事件
            let cache = app.state::<core::scheduler::Snapshots>().inner().clone();
            let ctl = app.state::<core::scheduler_ctl::SchedulerCtl>().inner().clone();
            let notify = {
                let handle = app.handle().clone();
                move || {
                    let _ = handle.emit("snapshots-updated", ());
                }
            };
            tokio::spawn(async move {
                core::scheduler::run(cache, ctl, notify).await;
            });

            // 调试钩子：TOKENMETER_AUTO_PANEL=1 时启动即创建并显示面板，
            // 并通知前端进入"添加供应商"视图（UI 抓屏/布局调试用）。
            if std::env::var("TOKENMETER_AUTO_PANEL").as_deref() == Ok("1") {
                let handle = app.handle().clone();
                let h2 = handle.clone();
                // Windows 上 WebviewWindow 必须在主线程创建（异步任务会拿到
                // 无效窗口句柄导致白屏/窗口缺失），因此用 run_on_main_thread。
                let _ = handle.run_on_main_thread(move || {
                    if let Some(w) = platform::tray::get_or_create_panel(&h2) {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                    let _ = h2.emit("debug-auto-panel", ());
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // popover 面板：只有"获得过焦点"后再失焦才隐藏（防 Windows 刚弹就关）；
            // 任何关闭请求一律转成隐藏，避免"最后窗口关闭"把进程带退出。
            if window.label() == "popover" {
                match event {
                    tauri::WindowEvent::Focused(true) => {
                        POPOVER_HAS_FOCUS.store(true, Ordering::Relaxed);
                    }
                    tauri::WindowEvent::Focused(false)
                        if POPOVER_HAS_FOCUS.load(Ordering::Relaxed) =>
                    {
                        POPOVER_HAS_FOCUS.store(false, Ordering::Relaxed);
                        let _ = window.hide();
                    }
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        POPOVER_HAS_FOCUS.store(false, Ordering::Relaxed);
                        let _ = window.hide();
                    }
                    _ => {}
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_app_handle, event| {
        // 硬保险：不是用户明确点"退出"（QUITTING）或应用更新重启（RESTART_EXIT_CODE）时，
        // 任何 ExitRequested 都拦下，防止 Windows 上窗口隐藏/关闭等路径把进程带退出。
        if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
            let restarting = code == Some(tauri::RESTART_EXIT_CODE);
            if !QUITTING.load(Ordering::Relaxed) && !restarting {
                api.prevent_exit();
            }
        }
    });
}
