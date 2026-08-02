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

use tauri::Manager;
use tauri_plugin_autostart::MacosLauncher;

/// popover 是否曾获得焦点（失焦关闭守卫：未获焦前的失焦不关窗口，防"刚弹就关"）
static POPOVER_HAS_FOCUS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

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

    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
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
        ])
        .setup(|app| {
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
            // 面板失焦自动关闭（销毁窗口 + 回收 WebView，纯菜单栏不留痕迹）。
            // 🔑 关键守卫：只有"获得过焦点"后再失焦才关闭。
            //    Windows 上无装饰+置顶窗口 show 后可能未真正获焦，会立刻收到
            //    Focused(false)，若直接 close → 窗口刚弹出就被销毁（观感"闪退"）。
            //    因此用 AtomicBool 记录"曾获焦"，未获焦前的失焦不处理。
            if window.label() == "popover" {
                match event {
                    tauri::WindowEvent::Focused(true) => {
                        POPOVER_HAS_FOCUS.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    tauri::WindowEvent::Focused(false)
                        if POPOVER_HAS_FOCUS.load(std::sync::atomic::Ordering::Relaxed) =>
                    {
                        POPOVER_HAS_FOCUS.store(false, std::sync::atomic::Ordering::Relaxed);
                        let _ = window.close();
                    }
                    _ => {}
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
