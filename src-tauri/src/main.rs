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

#[tokio::main]
async fn main() {
    env_logger::init();

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
            // 点别处 / 切走时，popover 面板自动隐藏
            if window.label() == "popover" {
                if let tauri::WindowEvent::Focused(false) = event {
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
