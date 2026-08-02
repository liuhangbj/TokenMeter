//! 菜单栏 / 托盘
//!
//! 图标：专用菜单栏模板图（纯黑剪影 + 透明底），`icon_as_template(true)` 让
//! macOS 按菜单栏明暗自动渲染为黑/白 —— 原生感关键。图标字节编译期嵌入。
//!
//! 弹窗：左键点托盘图标 → 把无装饰面板定位到托盘图标旁边；失焦自动隐藏。
//! 纯菜单栏 App：启动零窗口（tauri.conf.json windows: []），面板首次点击才创建，
//! 之后隐藏复用；所有窗口 skipTaskbar，macOS 由 LSUIElement + Accessory 策略隐藏 Dock。

use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder,
};

/// 菜单栏模板图标（编译期嵌入，RGBA 原始字节，64×64）
/// macOS：纯黑剪影模板图，系统按菜单栏明暗自动变色
#[cfg(target_os = "macos")]
const MENUBAR_RGBA: &[u8] = include_bytes!("../../icons/menubar.rgba");
/// Windows：彩色图标（模板剪影在深色任务栏看不清）
#[cfg(not(target_os = "macos"))]
const TRAY_COLOR_RGBA: &[u8] = include_bytes!("../../icons/tray_color.rgba");
const MENUBAR_SIZE: u32 = 64;
/// 面板宽度（与前端 CSS 一致）
const PANEL_W: i32 = 380;
/// 托盘单击防抖间隔（毫秒）：双击的第二击在此窗口内被忽略
const CLICK_DEBOUNCE_MS: u64 = 300;
/// 上次托盘点击的毫秒时间戳（双击防抖用）
static LAST_CLICK_MS: AtomicU64 = AtomicU64::new(0);

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

/// 获取（不存在则创建）popover 面板窗口。
/// 纯菜单栏架构：启动时零窗口（tauri.conf.json windows: []），点托盘图标第一次才创建，
/// 之后隐藏复用（不销毁，避免 Windows 上"最后窗口关闭 → 进程退出"类问题）。
pub(crate) fn get_or_create_panel(app: &tauri::AppHandle) -> Option<tauri::WebviewWindow> {
    if let Some(w) = app.get_webview_window("popover") {
        return Some(w);
    }
    WebviewWindowBuilder::new(app, "popover", WebviewUrl::App("index.html".into()))
        .title("TokenMeter")
        .inner_size(PANEL_W as f64, 600.0)
        .resizable(false)
        .decorations(false)
        // macOS：透明窗口 + 圆角外透明；Windows：透明不可靠且实色面板不需要
        .transparent(cfg!(target_os = "macos"))
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false) // 创建后由 toggle_panel 定位再显示
        .build()
        .map_err(|e| log::error!("创建面板窗口失败: {e}"))
        .ok()
}

/// 切换面板显示/隐藏，并把面板定位到托盘图标旁边。
/// 跨平台：macOS 菜单栏在屏幕顶部 → 面板向下弹；
///          Windows 任务栏通常在底部 → 面板向上弹。
/// 用【托盘图标所在显示器】做计算（不依赖窗口 current_monitor，
/// 窗口未显示时 current_monitor 只给主屏，多显示器会错位）。
fn toggle_panel(app: &tauri::AppHandle, tray_rect: tauri::Rect) {
    let Some(window) = get_or_create_panel(app) else {
        log::warn!("toggle_panel: 面板窗口创建失败");
        return;
    };
    if window.is_visible().unwrap_or(false) {
        // 面板已打开 → 点击收起（hide 复用窗口，不销毁）
        let _ = window.hide();
        return;
    }

    // tray_rect 的 position/size 是 Position/Size 枚举（逻辑或物理）。
    // tray-icon 底层给的是物理像素；对 Physical 变体 to_physical 是 identity，
    // 对 Logical 变体会用 scale 换算。这里取物理坐标。
    let icon_x = tray_rect.position.to_physical::<f64>(1.0).x;
    let icon_y = tray_rect.position.to_physical::<f64>(1.0).y;
    let icon_w = tray_rect.size.to_physical::<u32>(1.0).width as f64;
    let icon_h = tray_rect.size.to_physical::<u32>(1.0).height as f64;

    // 用托盘坐标反查它所在的显示器（物理坐标）
    let monitor = app
        .monitor_from_point(icon_x, icon_y)
        .ok()
        .flatten();

    // 面板实际尺寸
    let win_size = window
        .outer_size()
        .map(|s| (s.width as f64, s.height as f64))
        .unwrap_or((PANEL_W as f64, 600.0));
    let panel_w = win_size.0;
    let panel_h = win_size.1;

    // 水平：居中于图标
    let mut x = icon_x + (icon_w / 2.0) - (panel_w / 2.0);

    // 垂直：图标在屏幕上半 → 向下弹；下半（Windows 任务栏）→ 向上弹
    let mut y = icon_y + icon_h; // 默认向下
    if let Some(m) = monitor {
        let spos = m.position(); // 物理
        let ssize = m.size();    // 物理
        let screen_top = spos.y as f64;
        let screen_h = ssize.height as f64;
        let icon_center_y = icon_y + icon_h / 2.0;

        if icon_center_y > screen_top + screen_h / 2.0 {
            y = icon_y - panel_h; // 下半屏 → 向上弹
        } else {
            y = icon_y + icon_h;  // 上半屏 → 向下弹
        }

        // 水平防越界（贴屏边缘收敛）
        let max_x = spos.x as f64 + ssize.width as f64 - panel_w - 8.0;
        let min_x = spos.x as f64 + 8.0;
        x = x.clamp(min_x, max_x.max(min_x));

        // 垂直防越界
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
            "quit" => {
                log::info!("用户退出");
                // 先置标志再请求退出：ExitRequested 守卫见 main.rs，
                // 只有用户明确退出才放行。
                crate::QUITTING.store(true, AtomicOrdering::Relaxed);
                app.exit(0);
                // 兜底：Windows 上偶发事件循环不退出（WebView2 子进程挂起），
                // 3 秒后强制结束进程，避免"只能任务管理器杀"。
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_secs(3));
                    std::process::exit(0);
                });
            }
            "refresh" => {
                log::info!("用户触发刷新");
                if let Some(ctl) = app.try_state::<crate::core::scheduler_ctl::SchedulerCtl>() {
                    ctl.trigger_refresh();
                }
            }
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
                // 双击防抖：Windows 双击会触发两次 Click(Up)，
                // 第一击弹出面板、第二击立刻收起 → 观感"闪退"。
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let last = LAST_CLICK_MS.load(AtomicOrdering::Relaxed);
                if now.saturating_sub(last) < CLICK_DEBOUNCE_MS {
                    return;
                }
                LAST_CLICK_MS.store(now, AtomicOrdering::Relaxed);

                toggle_panel(tray.app_handle(), rect);
            }
        })
        .build(app)?;

    // 保活：Tauri 2 文档说明 TrayIcon 最后一个实例被 drop 会移除托盘图标，
    // 注册进 app 托管状态确保存活（资源表也会持有一份，双保险）。
    app.manage(TrayHandle(_tray));

    Ok(())
}

/// 托盘句柄（保活用：字段不读，仅持有以防 TrayIcon 被 drop 移除托盘图标）
#[allow(dead_code)]
pub struct TrayHandle(pub tauri::tray::TrayIcon);
