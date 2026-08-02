//! 平台壳层（三层架构的 Platform Shell）。
//!
//! 所有平台差异集中在这里：
//! - `tray`：托盘/菜单栏（macOS 顶部菜单栏、Windows 任务栏托盘）
//! - `open_browser`：系统浏览器打开（macOS open / Windows start）
//! - `setup`：平台级启动配置（macOS Accessory 策略隐藏 Dock）
//!
//! 窗口失焦/退出守卫等平台细节见 `crate::main` 的 on_window_event / run 回调。

pub mod tray;

/// 平台相关的一次性启动配置。
pub fn setup(app: &mut tauri::App) {
    // 纯菜单栏：运行时强制 Accessory 策略（不显示 Dock / Cmd+Tab），
    // 连 `tauri dev` 调试模式也生效；Info.plist 的 LSUIElement 只管打包后。
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);
}

/// 用系统默认浏览器打开 URL。
pub fn open_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let (cmd, args) = ("open", vec![url]);
    #[cfg(target_os = "windows")]
    let (cmd, args) = ("cmd", vec!["/C", "start", "", url]);
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let (cmd, args) = ("xdg-open", vec![url]);
    std::process::Command::new(cmd).args(&args).spawn()?;
    Ok(())
}
