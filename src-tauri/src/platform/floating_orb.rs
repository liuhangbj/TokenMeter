//! 单窗口桌面悬浮球。
//!
//! 大球与所有小球共享一个透明 WebView，避免按账号创建多个原生窗口后
//! 在 Windows 上出现任务栏残留、焦点闪烁和位置竞争。

use crate::core::settings::{FloatingOrbEdge, FloatingOrbSettings, Theme};
use tauri::{LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

pub const WINDOW_LABEL: &str = "floating-orb";
const EXPANDED_W: f64 = 156.0;
const EXPANDED_MAX_H: f64 = 536.0;
const EDGE_MARGIN: f64 = 10.0;

fn layout_size(collapsed: bool, account_count: usize, theme: Theme) -> (f64, f64) {
    if collapsed {
        // 收起后直接复用展开列表中的 small 账号组件。这里仅按各主题既有
        // small 尺寸加上舞台留白，不再维护另一套“收起球”视觉。
        return if account_count == 0 {
            (78.0, 76.0)
        } else {
            match theme {
                Theme::Cyberpunk => (69.0, 67.0),
                Theme::Parchment => (96.0, 66.0),
                Theme::Classic => (104.0, 58.0),
            }
        };
    }
    let height = match account_count {
        0 => 150.0,
        1 => 233.0,
        count => (244.0 + (count - 1) as f64 * 62.0).min(EXPANDED_MAX_H),
    };
    (EXPANDED_W, height)
}

pub fn get_or_create(app: &tauri::AppHandle) -> Option<tauri::WebviewWindow> {
    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        return Some(window);
    }

    let current = crate::core::settings::load();
    let account_count = crate::core::store::configured_accounts().len();
    let (width, height) = layout_size(current.floating_orb.collapsed, account_count, current.theme);

    WebviewWindowBuilder::new(app, WINDOW_LABEL, WebviewUrl::App("index.html".into()))
        .title("TokenMeter Floating Orb")
        .inner_size(width, height)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        // Windows 桌面挂件不应在启动、重新开启或刷新布局时抢走当前应用焦点。
        // WS_EX_NOACTIVATE 仍允许鼠标点击和拖动，只禁止把悬浮窗变成前台窗口；
        // 点击大球后由主面板显式 set_focus()，因此不会影响正常打开详情。
        .focusable(!cfg!(target_os = "windows"))
        .focused(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .shadow(false)
        .visible_on_all_workspaces(true)
        .visible(false)
        .build()
        .map_err(|error| log::error!("创建悬浮球窗口失败: {error}"))
        .ok()
}

fn selected_monitor(
    app: &tauri::AppHandle,
    settings: &FloatingOrbSettings,
) -> Option<tauri::Monitor> {
    let monitors = app.available_monitors().ok()?;
    settings
        .monitor_name
        .as_deref()
        .and_then(|name| {
            monitors
                .iter()
                .find(|monitor| monitor.name().is_some_and(|candidate| candidate == name))
                .cloned()
        })
        .or_else(|| app.primary_monitor().ok().flatten())
        .or_else(|| monitors.into_iter().next())
}

fn position_window(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    settings: &FloatingOrbSettings,
    logical_size: Option<(f64, f64)>,
) {
    let Some(monitor) = selected_monitor(app, settings) else {
        return;
    };
    let work = monitor.work_area();
    let scale = monitor.scale_factor();
    let (width, height) = logical_size
        .map(|(width, height)| (width * scale, height * scale))
        .or_else(|| {
            window
                .outer_size()
                .ok()
                .map(|size| (size.width as f64, size.height as f64))
        })
        .unwrap_or((EXPANDED_W * scale, EXPANDED_MAX_H * scale));
    let margin = if settings.collapsed { 4.0 } else { EDGE_MARGIN } * scale;
    let min_x = work.position.x as f64 + margin;
    let max_x = (work.position.x as f64 + work.size.width as f64 - width - margin).max(min_x);
    let x = match settings.edge {
        FloatingOrbEdge::Left => min_x,
        FloatingOrbEdge::Right => max_x,
    };
    let min_y = work.position.y as f64 + margin;
    let max_y = (work.position.y as f64 + work.size.height as f64 - height - margin).max(min_y);
    let y = min_y + (max_y - min_y) * settings.y_ratio.clamp(0.0, 1.0);
    if let Err(error) =
        window.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))
    {
        log::error!("定位悬浮球窗口失败: {error}");
    }
}

/// 根据账户数量收缩窗口；最多容纳五个小球，更多账户在列内滚动。
pub fn apply_layout(
    app: &tauri::AppHandle,
    settings: &FloatingOrbSettings,
    account_count: usize,
    theme: Theme,
) {
    let Some(window) = get_or_create(app) else {
        return;
    };
    let size = layout_size(settings.collapsed, account_count, theme);
    if let Err(error) = window.set_size(LogicalSize::new(size.0, size.1)) {
        log::error!("调整悬浮球窗口尺寸失败: {error}");
        return;
    }
    position_window(app, &window, settings, Some(size));
}

pub fn sync_visibility(app: &tauri::AppHandle, settings: &FloatingOrbSettings, theme: Theme) {
    if settings.enabled {
        let account_count = crate::core::store::configured_accounts().len();
        apply_layout(app, settings, account_count, theme);
        let Some(window) = get_or_create(app) else {
            return;
        };
        if let Err(error) = window.show() {
            log::error!("显示悬浮球窗口失败: {error}");
        }
    } else if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        if let Err(error) = window.hide() {
            log::error!("隐藏悬浮球窗口失败: {error}");
        }
    }
}

/// 开发 UI 验收入口：显式环境变量可临时展示悬浮球，不改写用户设置。
pub(crate) fn show_for_debug(app: &tauri::AppHandle) {
    let current = crate::core::settings::load();
    let account_count = crate::core::store::configured_accounts().len();
    apply_layout(app, &current.floating_orb, account_count, current.theme);
    let Some(window) = get_or_create(app) else {
        return;
    };
    if let Err(error) = window.show() {
        log::error!("显示调试悬浮球窗口失败: {error}");
    }
}

/// 拖动结束后按窗口中心吸附到当前显示器左右边缘，并返回可持久化位置。
pub fn snap_and_describe(app: &tauri::AppHandle) -> Result<FloatingOrbSettings, String> {
    let window = app
        .get_webview_window(WINDOW_LABEL)
        .ok_or_else(|| "悬浮球窗口尚未创建".to_string())?;
    let position = window.outer_position().map_err(|error| error.to_string())?;
    let size = window.outer_size().map_err(|error| error.to_string())?;
    let center_x = position.x as f64 + size.width as f64 / 2.0;
    let center_y = position.y as f64 + size.height as f64 / 2.0;
    let monitor = app
        .monitor_from_point(center_x, center_y)
        .map_err(|error| error.to_string())?
        .or_else(|| app.primary_monitor().ok().flatten())
        .ok_or_else(|| "无法获取显示器工作区".to_string())?;
    let work = monitor.work_area();
    let scale = monitor.scale_factor();
    let margin = EDGE_MARGIN * scale;
    let min_x = work.position.x as f64 + margin;
    let max_x =
        (work.position.x as f64 + work.size.width as f64 - size.width as f64 - margin).max(min_x);
    let work_center_x = work.position.x as f64 + work.size.width as f64 / 2.0;
    let edge = if center_x < work_center_x {
        FloatingOrbEdge::Left
    } else {
        FloatingOrbEdge::Right
    };
    let x = match edge {
        FloatingOrbEdge::Left => min_x,
        FloatingOrbEdge::Right => max_x,
    };
    let min_y = work.position.y as f64 + margin;
    let max_y =
        (work.position.y as f64 + work.size.height as f64 - size.height as f64 - margin).max(min_y);
    let y = (position.y as f64).clamp(min_y, max_y);
    let y_ratio = if max_y > min_y {
        (y - min_y) / (max_y - min_y)
    } else {
        0.5
    };
    window
        .set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))
        .map_err(|error| error.to_string())?;

    Ok(FloatingOrbSettings {
        enabled: true,
        edge,
        y_ratio,
        monitor_name: monitor.name().cloned(),
        active_account_id: None,
        collapsed: crate::core::settings::load().floating_orb.collapsed,
    })
}

#[cfg(test)]
mod tests {
    use super::layout_size;
    use crate::core::settings::Theme;

    #[test]
    fn expanded_height_tracks_account_count_and_caps() {
        assert_eq!(layout_size(false, 1, Theme::Classic), (156.0, 233.0));
        assert_eq!(layout_size(false, 2, Theme::Parchment), (156.0, 306.0));
        assert_eq!(layout_size(false, 4, Theme::Cyberpunk), (156.0, 430.0));
        assert_eq!(layout_size(false, 20, Theme::Classic), (156.0, 536.0));
    }

    #[test]
    fn collapsed_layout_matches_each_themes_existing_small_orb() {
        assert_eq!(layout_size(true, 1, Theme::Cyberpunk), (69.0, 67.0));
        assert_eq!(layout_size(true, 20, Theme::Parchment), (96.0, 66.0));
        assert_eq!(layout_size(true, 3, Theme::Classic), (104.0, 58.0));
        assert_eq!(layout_size(true, 0, Theme::Classic), (78.0, 76.0));
    }
}
