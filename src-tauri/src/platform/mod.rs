//! 平台壳层（三层架构的 Platform Shell）。
//!
//! 所有平台差异集中在这里：
//! - `tray`：托盘/菜单栏（macOS 顶部菜单栏、Windows 任务栏托盘）
//! - `open_browser`：系统浏览器打开（Windows 使用 ShellExecuteW）
//! - `setup`：平台级启动配置（macOS Accessory 策略隐藏 Dock）
//!
//! 窗口失焦/退出守卫等平台细节见 `crate::main` 的 on_window_event / run 回调。

pub mod floating_orb;
pub mod tray;

/// 平台相关的一次性启动配置。
pub fn setup(_app: &mut tauri::App) {
    // 纯菜单栏：运行时强制 Accessory 策略（不显示 Dock / Cmd+Tab），
    // 连 `tauri dev` 调试模式也生效；Info.plist 的 LSUIElement 只管打包后。
    #[cfg(target_os = "macos")]
    _app.set_activation_policy(tauri::ActivationPolicy::Accessory);
}

/// 用系统默认浏览器打开 URL。Windows 显式使用 ShellExecuteW，绕开 WebView2
/// 与前端 shell plugin 在无主窗口模式下偶发无法拉起默认浏览器的问题。
pub fn open_browser(url: &str) -> std::io::Result<()> {
    open::that_detached(url)
}
