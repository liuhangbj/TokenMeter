//! 菜单栏 / 托盘
//!
//! 图标：专用菜单栏模板图（纯黑剪影 + 透明底），`icon_as_template(true)` 让
//! macOS 按菜单栏明暗自动渲染为黑/白 —— 原生感关键。图标字节编译期嵌入。
//!
//! 弹窗：左键点托盘图标 → 把无装饰面板定位到托盘图标旁边；失焦自动隐藏。
//! 纯菜单栏 App：启动时创建隐藏面板（前端后台完成测量/定型），
//! 首次点击托盘时直接以最终尺寸定位显示；之后隐藏复用；
//! 所有窗口 skipTaskbar，macOS 由 LSUIElement + Accessory 策略隐藏 Dock。

use std::sync::{
    atomic::{AtomicU64, Ordering as AtomicOrdering},
    Mutex, OnceLock,
};

use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder,
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
/// Windows 隐藏预创建时的初始高度；前端完成内容测量后会自适应收缩或扩展。
#[cfg(target_os = "windows")]
const WINDOWS_PANEL_H: i32 = 560;
/// 托盘单击防抖间隔（毫秒）：双击的第二击在此窗口内被忽略
const CLICK_DEBOUNCE_MS: u64 = 300;
/// 上次托盘点击的毫秒时间戳（双击防抖用）
static LAST_CLICK_MS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Default)]
enum PanelAnchor {
    /// 隐藏窗口完成初次测量、尚未选择显示入口。
    #[default]
    Unset,
    /// 菜单栏点击入口：保存定位完成后的目标坐标，而不是临时读取当前坐标。
    FixedPosition(PhysicalPosition<i32>),
    /// 悬浮组件入口：始终贴在目标显示器工作区右上角。
    RightTop { monitor_name: Option<String> },
    /// Windows 托盘入口：动态改变高度时保持右边和底边贴近任务栏工作区。
    #[cfg(target_os = "windows")]
    RightBottom { monitor_name: Option<String> },
}

static PANEL_ANCHOR: OnceLock<Mutex<PanelAnchor>> = OnceLock::new();

fn set_panel_anchor(anchor: PanelAnchor) {
    match PANEL_ANCHOR
        .get_or_init(|| Mutex::new(PanelAnchor::default()))
        .lock()
    {
        Ok(mut current) => *current = anchor,
        Err(error) => log::error!("面板锚点状态损坏: {error}"),
    }
}

fn panel_anchor() -> PanelAnchor {
    PANEL_ANCHOR
        .get_or_init(|| Mutex::new(PanelAnchor::default()))
        .lock()
        .map(|anchor| anchor.clone())
        .unwrap_or_default()
}

fn monitor_by_name(app: &tauri::AppHandle, name: Option<&str>) -> Option<tauri::Monitor> {
    let monitors = app.available_monitors().ok()?;
    name.and_then(|expected| {
        monitors
            .iter()
            .find(|monitor| {
                monitor
                    .name()
                    .is_some_and(|candidate| candidate == expected)
            })
            .cloned()
    })
    .or_else(|| app.primary_monitor().ok().flatten())
    .or_else(|| monitors.into_iter().next())
}

fn position_panel_right_top(
    panel: &tauri::WebviewWindow,
    monitor: &tauri::Monitor,
) -> Result<(), String> {
    let work = monitor.work_area();
    let scale = monitor.scale_factor();
    let panel_w = PANEL_W as f64 * scale;
    let margin = 8.0 * scale;
    let x = (work.position.x as f64 + work.size.width as f64 - panel_w - margin)
        .max(work.position.x as f64 + margin);
    let y = work.position.y as f64 + margin;
    set_panel_position_if_needed(
        panel,
        PhysicalPosition::new(x.round() as i32, y.round() as i32),
    )?;
    log::info!("主面板锚定右上角: x={}, y={}", x.round(), y.round());
    Ok(())
}

#[cfg(target_os = "windows")]
fn position_panel_right_bottom(
    panel: &tauri::WebviewWindow,
    monitor: &tauri::Monitor,
) -> Result<(), String> {
    let size = panel.outer_size().map_err(|error| error.to_string())?;
    position_panel_right_bottom_for_size(panel, monitor, size.width, size.height)
}

#[cfg(target_os = "windows")]
fn position_panel_right_bottom_for_size(
    panel: &tauri::WebviewWindow,
    monitor: &tauri::Monitor,
    panel_width: u32,
    panel_height: u32,
) -> Result<(), String> {
    let work = monitor.work_area();
    let (x, y) = right_bottom_coordinates(
        work.position.x,
        work.position.y,
        work.size.width,
        work.size.height,
        panel_width,
        panel_height,
        monitor.scale_factor(),
    );
    set_panel_position_if_needed(panel, PhysicalPosition::new(x, y))?;
    log::info!("主面板锚定右下角: x={x}, y={y}");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[cfg(any(target_os = "windows", test))]
fn right_bottom_coordinates(
    work_x: i32,
    work_y: i32,
    work_width: u32,
    work_height: u32,
    panel_width: u32,
    panel_height: u32,
    scale: f64,
) -> (i32, i32) {
    let margin = 8.0 * scale;
    let min_x = work_x as f64 + margin;
    let min_y = work_y as f64 + margin;
    let x = (work_x as f64 + work_width as f64 - panel_width as f64 - margin).max(min_x);
    let y = (work_y as f64 + work_height as f64 - panel_height as f64 - margin).max(min_y);
    (x.round() as i32, y.round() as i32)
}

fn set_panel_position_if_needed(
    panel: &tauri::WebviewWindow,
    target: PhysicalPosition<i32>,
) -> Result<(), String> {
    if panel
        .outer_position()
        .is_ok_and(|current| current.x == target.x && current.y == target.y)
    {
        return Ok(());
    }
    panel
        .set_position(target)
        .map_err(|error| error.to_string())
}

/// 窗口系统可能在 WebView 尺寸提交后再发送一次 move/resize，并把无边框窗口
/// 放回默认居中位置。所有窗口生命周期事件都通过这里恢复最后一次显式锚点。
pub(crate) fn reapply_panel_anchor(app: &tauri::AppHandle) -> Result<(), String> {
    let panel = get_or_create_panel(app).ok_or_else(|| "主面板窗口创建失败".to_string())?;
    match panel_anchor() {
        PanelAnchor::Unset => Ok(()),
        PanelAnchor::FixedPosition(position) => set_panel_position_if_needed(&panel, position),
        PanelAnchor::RightTop { monitor_name } => {
            let monitor = monitor_by_name(app, monitor_name.as_deref())
                .ok_or_else(|| "无法获取主面板所在显示器".to_string())?;
            position_panel_right_top(&panel, &monitor)
        }
        #[cfg(target_os = "windows")]
        PanelAnchor::RightBottom { monitor_name } => {
            let monitor = monitor_by_name(app, monitor_name.as_deref())
                .ok_or_else(|| "无法获取主面板所在显示器".to_string())?;
            position_panel_right_bottom(&panel, &monitor)
        }
    }
}

/// 前端内容测量后的唯一尺寸更新入口。尺寸变化与锚点恢复在同一原生调用中
/// 完成，避免无边框窗口被系统重新放到屏幕中央。
pub(crate) fn resize_panel(app: &tauri::AppHandle, height: f64) -> Result<(), String> {
    let panel = get_or_create_panel(app).ok_or_else(|| "主面板窗口创建失败".to_string())?;
    let height = height.round().clamp(120.0, PANEL_H as f64);
    #[cfg(target_os = "windows")]
    let anchor = panel_anchor();
    panel
        .set_size(LogicalSize::new(PANEL_W as f64, height))
        .map_err(|error| error.to_string())?;

    #[cfg(target_os = "windows")]
    if let PanelAnchor::RightBottom { monitor_name } = anchor {
        let monitor = monitor_by_name(app, monitor_name.as_deref())
            .ok_or_else(|| "无法获取主面板所在显示器".to_string())?;
        let scale = monitor.scale_factor();
        return position_panel_right_bottom_for_size(
            &panel,
            &monitor,
            (PANEL_W as f64 * scale).round() as u32,
            (height * scale).round() as u32,
        );
    }

    reapply_panel_anchor(app)
}

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
    set_panel_anchor(PanelAnchor::Unset);

    #[cfg(target_os = "windows")]
    {
        // 用点击光标位置反查托盘所在显示器（比 tray_rect 可靠）
        let monitor = app.monitor_from_point(_cursor.x, _cursor.y).ok().flatten();
        if let Some(m) = monitor {
            set_panel_anchor(PanelAnchor::RightBottom {
                monitor_name: m.name().cloned(),
            });
            let _ = position_panel_right_bottom(&window, &m);
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

        let position = PhysicalPosition::new(x as i32, y as i32);
        set_panel_anchor(PanelAnchor::FixedPosition(position));
        let _ = set_panel_position_if_needed(&window, position);
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
        if let Some(monitor) = monitor_by_name(app, None) {
            set_panel_anchor(PanelAnchor::RightTop {
                monitor_name: monitor.name().cloned(),
            });
            let _ = position_panel_right_top(&window, &monitor);
        }
        crate::mark_panel_shown();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// 从悬浮组件打开主面板。主面板固定锚定在悬浮组件所在显示器的右上角，
/// 不再跟随组件的纵向位置，否则组件拖到屏幕下半部后主面板会出现在中央。
pub(crate) fn show_panel_near_floating_orb(app: &tauri::AppHandle) {
    let Some(orb) = app.get_webview_window(crate::platform::floating_orb::WINDOW_LABEL) else {
        show_panel_on_primary_monitor(app);
        return;
    };
    let Some(panel) = get_or_create_panel(app) else {
        return;
    };
    let Ok(orb_position) = orb.outer_position() else {
        show_panel_on_primary_monitor(app);
        return;
    };
    let Ok(orb_size) = orb.outer_size() else {
        show_panel_on_primary_monitor(app);
        return;
    };
    let center_x = orb_position.x as f64 + orb_size.width as f64 / 2.0;
    let center_y = orb_position.y as f64 + orb_size.height as f64 / 2.0;
    let Ok(Some(monitor)) = app.monitor_from_point(center_x, center_y) else {
        show_panel_on_primary_monitor(app);
        return;
    };
    set_panel_anchor(PanelAnchor::RightTop {
        monitor_name: monitor.name().cloned(),
    });
    let _ = position_panel_right_top(&panel, &monitor);
    crate::mark_panel_shown();
    let _ = panel.show();
    let _ = panel.set_focus();
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

#[cfg(test)]
mod tests {
    use super::right_bottom_coordinates;

    #[test]
    fn windows_panel_keeps_its_bottom_edge_when_height_shrinks() {
        let full = right_bottom_coordinates(0, 0, 1920, 1040, 380, 560, 1.0);
        let compact = right_bottom_coordinates(0, 0, 1920, 1040, 380, 320, 1.0);

        assert_eq!(full, (1532, 472));
        assert_eq!(compact, (1532, 712));
        assert_eq!(full.1 + 560, compact.1 + 320);
    }

    #[test]
    fn windows_panel_position_respects_monitor_origin_and_scale() {
        assert_eq!(
            right_bottom_coordinates(-2560, 0, 2560, 1400, 760, 640, 2.0),
            (-776, 744)
        );
    }
}
