#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod oauth_codex;
mod oauth_device;
mod providers;
mod scheduler;
mod scheduler_ctl;
mod settings;
mod store;
mod tray;

use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;
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
        .plugin(tauri_plugin_store::Builder::new().build())
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
            commands::open_add_provider,
            commands::save_api_key_provider,
            commands::import_local_credential,
            commands::kimi_device_start,
            commands::kimi_device_poll,
            commands::codex_oauth_start,
            commands::resize_popover,
            commands::remove_provider,
            commands::has_configured_providers,
            commands::quit_app,
        ])
        .setup(|app| {
            // 纯菜单栏：运行时也强制 Accessory 策略（不显示 Dock / Cmd+Tab），
            // 连 `tauri dev` 调试模式也生效，Info.plist 的 LSUIElement 只管打包后。
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // 读取设置，初始化调度控制（间隔广播 + 立即刷新信号）
            let current = settings::load(app.handle());
            app.manage(scheduler_ctl::new_ctl(current.refresh_interval_secs));
            app.manage(scheduler::new_snapshots());
            tray::build_tray(app.handle())?;

            let handle = app.handle().clone();
            tokio::spawn(async move {
                scheduler::run(handle).await;
            });
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
