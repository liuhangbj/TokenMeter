//! 菜单栏 / 托盘
//!
//! 图标：专用菜单栏模板图（纯黑剪影 + 透明底），`icon_as_template(true)` 让
//! macOS 按菜单栏明暗自动渲染为黑/白 —— 原生感关键。图标字节编译期嵌入。
//!
//! 弹窗：左键点托盘图标 → 把无装饰面板定位到图标正下方显示；失焦自动隐藏。

use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconEvent},
    tray::TrayIconBuilder,
    Manager, PhysicalPosition,
};

/// 菜单栏模板图标（编译期嵌入，RGBA 原始字节，64×64）
/// macOS：纯黑剪影模板图，系统按菜单栏明暗自动变色
#[cfg(target_os = "macos")]
const MENUBAR_RGBA: &[u8] = include_bytes!("../icons/menubar.rgba");
/// Windows：彩色图标（模板剪影在深色任务栏看不清）
#[cfg(not(target_os = "macos"))]
const TRAY_COLOR_RGBA: &[u8] = include_bytes!("../icons/tray_color.rgba");
const MENUBAR_SIZE: u32 = 64;
/// 面板宽度（与 tauri.conf.json 的 popover 窗口一致）
const PANEL_W: i32 = 380;

/// 托盘图标与"是否模板图"按平台选择。
/// macOS 用模板剪影 + icon_as_template(true)；Windows 用彩色图 + 非模板。
fn tray_icon() -> (Image<'static>, bool) {
    #[cfg(target_os = "macos")]
    {
        (
            Image::new_owned(MENUBAR_RGBA.to_vec(), MENUBAR_SIZE, MENUBAR_SIZE),
            true,
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        (
            Image::new_owned(TRAY_COLOR_RGBA.to_vec(), MENUBAR_SIZE, MENUBAR_SIZE),
            false,
        )
    }
}

/// 切换面板显示/隐藏，并把面板定位到托盘图标正下方。
fn toggle_panel(app: &tauri::AppHandle, tray_rect: tauri::Rect) {
    let Some(window) = app.get_webview_window("popover") else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }

    // tray_rect.position/size 是 Position/Size 枚举（逻辑或物理），
    // 必须用【显示器的真实 scale_factor】转物理坐标——Retina 是 2.0，
    // 之前误传 1.0 导致坐标少算一半、面板跑偏到屏幕中间（用户反馈）。
    let scale = window
        .current_monitor()
        .ok()
        .flatten()
        .map(|m| m.scale_factor())
        .unwrap_or(1.0);

    let icon_pos = tray_rect.position.to_physical::<f64>(scale);
    let icon_size = tray_rect.size.to_physical::<f64>(scale);

    // 面板实际尺寸（用于向上弹出与越界计算）
    let win_size = window
        .outer_size()
        .map(|s| (s.width as f64, s.height as f64))
        .unwrap_or((PANEL_W as f64, 600.0));
    let panel_w = win_size.0;
    let panel_h = win_size.1;

    // 水平：居中于图标
    let mut x = icon_pos.x + (icon_size.width / 2.0) - (panel_w / 2.0);

    // 垂直：跨平台自适应弹出方向。
    // macOS 菜单栏在屏幕【顶部】→ 面板向【下】展开；
    // Windows 托盘在任务栏【底部】→ 面板向【上】展开。
    // 判定：图标中心在屏幕垂直中线以上则向下弹，否则向上弹。
    let mut y = icon_pos.y + icon_size.height; // 默认向下
    if let Ok(Some(monitor)) = window.current_monitor() {
        let screen = monitor.size();
        let spos = monitor.position();
        let screen_top = spos.y as f64;
        let screen_h = screen.height as f64;
        let icon_center_y = icon_pos.y + icon_size.height / 2.0;

        if icon_center_y > screen_top + screen_h / 2.0 {
            // 图标在下半屏（Windows 任务栏）→ 向上弹出
            y = icon_pos.y - panel_h;
        } else {
            // 图标在上半屏（macOS 菜单栏）→ 向下弹出
            y = icon_pos.y + icon_size.height;
        }

        // 水平防越界
        let max_x = spos.x as f64 + screen.width as f64 - panel_w - 8.0;
        let min_x = spos.x as f64 + 8.0;
        x = x.clamp(min_x, max_x.max(min_x));

        // 垂直防越界（向上弹出时顶部不出屏；向下弹出时底部不出屏则上收）
        let max_y = screen_top + screen_h - panel_h - 8.0;
        let min_y = screen_top + 8.0;
        y = y.clamp(min_y, max_y.max(min_y));
    }

    let _ = window.set_position(PhysicalPosition::new(x as i32, y as i32));
    let _ = window.show();
    let _ = window.set_focus();
}

pub fn build_tray(app: &tauri::AppHandle) -> anyhow::Result<()> {
    let refresh = MenuItem::with_id(app, "refresh", "刷新额度", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&refresh, &quit])?;

    let (icon, is_template) = tray_icon();

    let _tray = TrayIconBuilder::new()
        .icon(icon)
        .icon_as_template(is_template) // macOS 模板图随菜单栏变色；Windows 用彩色图
        .menu(&menu)
        .show_menu_on_left_click(false) // 左键用于弹面板，菜单走右键
        .tooltip("TokenMeter")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "quit" => app.exit(0),
            "refresh" => log::info!("用户触发刷新"),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } = event
            {
                toggle_panel(tray.app_handle(), rect);
            }
        })
        .build(app)?;

    Ok(())
}
