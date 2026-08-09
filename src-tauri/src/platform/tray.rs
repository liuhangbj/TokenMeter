//! 菜单栏 / 托盘
//!
//! 图标：专用菜单栏模板图（纯黑剪影 + 透明底），`icon_as_template(true)` 让
//! macOS 按菜单栏明暗自动渲染为黑/白 —— 原生感关键。图标字节编译期嵌入。
//!
//! 弹窗：左键点托盘图标 → 把无装饰面板定位到托盘图标旁边；失焦自动隐藏。
//! 纯菜单栏 App：启动时创建隐藏面板（前端后台完成测量/定型），
//! 首次点击托盘时直接以最终尺寸定位显示；之后隐藏复用；
//! 所有窗口 skipTaskbar，macOS 由 LSUIElement + Accessory 策略隐藏 Dock。

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
/// 面板默认宽度：由 506px 收窄约 25%，适合额度概览的紧凑信息密度。
/// 添加供应商向导也会在此宽度内响应式排版。
pub(crate) const PANEL_W: i32 = 380;
/// 面板的最大初始高度。前端会在内容不足时自动缩短窗口；超过该高度时面板内滚动。
pub(crate) const PANEL_H: i32 = 800;
/// Windows 外层窗口固定高度。WebView2 可见窗口不做运行时 resize，内容在内部滚动。
#[cfg(target_os = "windows")]
const WINDOWS_PANEL_H: i32 = 560;
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
/// 启动时由 main 预先创建（隐藏），前端在后台完成测量与尺寸定型；
/// 之后隐藏复用（不销毁，避免 Windows 上"最后窗口关闭 → 进程退出"类问题）。
pub(crate) fn get_or_create_panel(app: &tauri::AppHandle) -> Option<tauri::WebviewWindow> {
    if let Some(w) = app.get_webview_window("popover") {
        return Some(w);
    }
    #[cfg(target_os = "windows")]
    let initial_height = WINDOWS_PANEL_H;
    #[cfg(not(target_os = "windows"))]
    let initial_height = PANEL_H;

    WebviewWindowBuilder::new(app, "popover", WebviewUrl::App("index.html".into()))
        .title("TokenMeter")
        .inner_size(PANEL_W as f64, initial_height as f64)
        .resizable(false)
        .decorations(false)
        // macOS：透明窗口 + 圆角外透明；Windows：透明不可靠且实色面板不需要
        .transparent(cfg!(target_os = "macos"))
        .always_on_top(true)
        .skip_taskbar(true)
        // 无边框透明窗口的原生阴影仍按宿主矩形绘制，无法可靠贴合
        // WebView 内的 CSS 圆角，底部会露出尖角。所有平台统一由 CSS
        // 面板负责边界与阴影，避免两套窗口轮廓叠加。
        .shadow(false)
        .visible(false) // 创建后由 toggle_panel 定位再显示
        .build()
        .map_err(|e| log::error!("创建面板窗口失败: {e}"))
        .ok()
}

/// 切换面板显示/隐藏并定位。
/// - Windows：固定锚定【工作区右下角】（工作区自动扣除任务栏），
///   面板底部始终贴在任务栏上方，首次/再次弹出位置完全一致，
///   不依赖托盘 rect 的准确性，也不随面板高度变化而跳动。
/// - macOS：菜单栏在顶部 → 面板在图标正下方弹出。
fn toggle_panel(app: &tauri::AppHandle, cursor: tauri::PhysicalPosition<f64>) {
    let Some(window) = get_or_create_panel(app) else {
        log::warn!("toggle_panel: 面板窗口创建失败");
        return;
    };
    if window.is_visible().unwrap_or(false) {
        // 面板已打开 → 点击收起（hide 复用窗口，不销毁）
        crate::mark_panel_hidden();
        let _ = window.hide();
        return;
    }

    log::info!("toggle_panel: 就绪，直接定位显示");
    position_and_show(app, cursor);
}

/// 定位并显示面板：定位必须在 show 之前完成，显示后不再移动窗口。
fn position_and_show(app: &tauri::AppHandle, _cursor: tauri::PhysicalPosition<f64>) {
    let Some(window) = get_or_create_panel(app) else {
        log::warn!("position_and_show: 面板窗口创建失败");
        return;
    };

    #[cfg(target_os = "windows")]
    {
        // 用点击光标位置反查托盘所在显示器（比 tray_rect 可靠）
        let monitor = app.monitor_from_point(_cursor.x, _cursor.y).ok().flatten();
        if let Some(m) = monitor {
            let wa = m.work_area(); // 物理坐标，已扣除任务栏
            let margin = 8.0_f64;
            // 右下角锚定使用窗口当前真实物理尺寸；不再用逻辑常量猜测，
            // 避免 DPI 或未来尺寸策略变化后出现离任务栏的大段空隙。
            let scale = m.scale_factor();
            let size = window.outer_size().ok();
            let panel_w = size
                .map(|value| value.width as f64)
                .unwrap_or(PANEL_W as f64 * scale);
            let panel_h = size
                .map(|value| value.height as f64)
                .unwrap_or(WINDOWS_PANEL_H as f64 * scale);
            let x = (wa.position.x as f64 + wa.size.width as f64 - panel_w - margin)
                .max(wa.position.x as f64 + margin);
            let y = (wa.position.y as f64 + wa.size.height as f64 - panel_h - margin)
                .max(wa.position.y as f64 + margin);
            let _ = window.set_position(PhysicalPosition::new(x as i32, y as i32));
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let panel_w = PANEL_W as f64;
        let panel_h = PANEL_H as f64;

        // macOS：菜单栏在顶部，点击托盘时的光标位置就是图标位置。
        // 直接以点击光标定位（tray_rect 在部分系统上为 0/不准）：
        // 水平以光标为中心，垂直落在菜单栏下方约 10px。
        let cx = _cursor.x;
        let cy = _cursor.y;
        let monitor = app.monitor_from_point(cx, cy).ok().flatten();

        let mut x = cx - panel_w / 2.0;
        let mut y = cy + 10.0;
        if let Some(m) = monitor {
            let wa = m.work_area(); // 物理坐标，已扣除菜单栏/Dock
            let min_x = wa.position.x as f64 + 8.0;
            let max_x = wa.position.x as f64 + wa.size.width as f64 - panel_w - 8.0;
            x = x.clamp(min_x, max_x.max(min_x));
            let min_y = wa.position.y as f64 + 8.0;
            let max_y = wa.position.y as f64 + wa.size.height as f64 - panel_h - 8.0;
            y = y.clamp(min_y, max_y.max(min_y));
        }

        let _ = window.set_position(PhysicalPosition::new(x as i32, y as i32));
    }

    crate::mark_panel_shown();
    let _ = window.show();
    let _ = window.set_focus();
}

/// 无托盘点击坐标时的显示入口（二次启动、自动化截图）：Windows 用主显示器
/// 右下角走与托盘点击相同的定位逻辑；其他平台只负责显示。
pub(crate) fn show_panel_on_primary_monitor(app: &tauri::AppHandle) {
    #[cfg(target_os = "windows")]
    {
        if let Ok(Some(monitor)) = app.primary_monitor() {
            let work_area = monitor.work_area();
            let cursor = PhysicalPosition::new(
                (work_area.position.x as f64 + work_area.size.width as f64 - 1.0)
                    .max(work_area.position.x as f64),
                (work_area.position.y as f64 + work_area.size.height as f64 - 1.0)
                    .max(work_area.position.y as f64),
            );
            position_and_show(app, cursor);
            return;
        }
    }

    if let Some(window) = get_or_create_panel(app) {
        crate::mark_panel_shown();
        let _ = window.show();
        let _ = window.set_focus();
    }
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
                position,
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

                toggle_panel(tray.app_handle(), position);
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
