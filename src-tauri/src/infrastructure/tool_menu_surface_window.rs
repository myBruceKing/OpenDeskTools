use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Runtime, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};
use thiserror::Error;

use super::{
    debug_qa,
    popup_geometry::{
        fit_centered_surface_to_work_area as fit_surface_to_work_area, point_in_rect,
        squared_distance_to_rect,
    },
    quick_launch::{QuickLaunchSnapshot, ToolMenuLayout},
    surface_window_animation,
};

pub const TOOL_MENU_SURFACE_LABEL: &str = "tool-menu-surface";
pub const TOOL_MENU_SURFACE_SHOWN_EVENT: &str = "tool-menu://shown";
pub const TOOL_MENU_SURFACE_SNAPSHOT_EVENT: &str = "tool-menu://snapshot";
const TOOL_MENU_SURFACE_ROUTE: &str = "index.html#tool-menu-surface";
const TOOL_MENU_SURFACE_SIZE: f64 = 320.0;
const TOOL_MENU_DOCK_COLUMNS: usize = 6;
const TOOL_MENU_WHEEL_TRANSPARENT_GUARD: u32 = 4;
#[cfg(windows)]
const TOOL_MENU_POPUP_STYLE_SUBCLASS_ID: usize = 0x4f44_5453;
static TOOL_MENU_VISIBLE: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Error)]
pub enum ToolMenuSurfaceError {
    #[error("the tool menu surface is not prepared")]
    NotPrepared,
    #[error("tool menu quick launch state is unavailable: {0}")]
    QuickLaunch(String),
    #[error("tool menu window operation failed: {0}")]
    Window(#[from] tauri::Error),
    #[error("tool menu surface dimensions are invalid")]
    InvalidDimensions,
    #[cfg(windows)]
    #[error("Windows could not read the cursor position")]
    CursorPosition,
    #[cfg(windows)]
    #[error("Windows could not create the tool menu circular region")]
    CreateRegion,
    #[cfg(windows)]
    #[error("Windows could not apply the tool menu circular region")]
    ApplyRegion,
    #[cfg(windows)]
    #[error("Windows could not configure the tool menu as a borderless popup")]
    ConfigurePopupStyle,
}

/// Construct the launcher WebView during setup. Hotkey callbacks only reveal
/// this already-created surface, so they cannot block the keyboard hook path.
pub fn prepare<R: Runtime>(app: &AppHandle<R>) -> Result<(), ToolMenuSurfaceError> {
    if app.get_webview_window(TOOL_MENU_SURFACE_LABEL).is_some() {
        debug_qa::trace("tool-menu prepare result=existing");
        return Ok(());
    }
    debug_qa::trace("tool-menu prepare stage=build requested");
    let window = WebviewWindowBuilder::new(
        app,
        TOOL_MENU_SURFACE_LABEL,
        WebviewUrl::App(TOOL_MENU_SURFACE_ROUTE.into()),
    )
    .title("")
    .inner_size(TOOL_MENU_SURFACE_SIZE, TOOL_MENU_SURFACE_SIZE)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .transparent(true)
    .visible(false)
    .build()?;
    install_popup_style_guard(&window)?;
    configure_popup_style(&window)?;
    configure_transparent_non_client(&window);
    apply_circular_region(&window)?;
    debug_qa::trace("tool-menu prepare result=ready hidden");
    Ok(())
}

fn prepared<R: Runtime>(app: &AppHandle<R>) -> Result<WebviewWindow<R>, ToolMenuSurfaceError> {
    app.get_webview_window(TOOL_MENU_SURFACE_LABEL)
        .ok_or(ToolMenuSurfaceError::NotPrepared)
}

pub fn show<R: Runtime>(
    app: &AppHandle<R>,
    snapshot: &QuickLaunchSnapshot,
) -> Result<(), ToolMenuSurfaceError> {
    let window = prepared(app)?;
    let size = configure_window(&window, snapshot, false)?;
    let pointer = pointer_position()?;
    let position = placement_for_pointer(&window, pointer, size)?;
    window.set_position(PhysicalPosition::new(position.0, position.1))?;
    publish_window_snapshot(&window, snapshot)?;
    surface_window_animation::prepare_show(&window);
    configure_popup_style(&window)?;
    window.show()?;
    TOOL_MENU_VISIBLE.store(true, Ordering::SeqCst);
    window.set_focus()?;
    window.emit(TOOL_MENU_SURFACE_SHOWN_EVENT, ())?;
    debug_qa::trace(format!(
        "tool-menu show result=visible layout={:?} visible_items={} size={}x{}",
        snapshot.tool_menu.layout,
        snapshot
            .pinned_apps
            .iter()
            .filter(|app| app.visible)
            .count(),
        size.0,
        size.1
    ));
    Ok(())
}

/// Keep a prepared Surface coherent while settings mutate it. In particular,
/// deleting the seventh item changes a wheel from two rings to one; updating
/// only React would leave the old native circle around the new wheel.
pub fn sync_snapshot<R: Runtime>(
    app: &AppHandle<R>,
    snapshot: &QuickLaunchSnapshot,
) -> Result<(), ToolMenuSurfaceError> {
    let window = prepared(app)?;
    let visible = window.is_visible()?;
    configure_window(&window, snapshot, visible)?;
    publish_window_snapshot(&window, snapshot)
}

fn configure_window<R: Runtime>(
    window: &WebviewWindow<R>,
    snapshot: &QuickLaunchSnapshot,
    preserve_center: bool,
) -> Result<(u32, u32), ToolMenuSurfaceError> {
    let visible_item_count = snapshot
        .pinned_apps
        .iter()
        .filter(|app| app.visible)
        .count();
    let size = surface_size(snapshot.tool_menu.layout, visible_item_count);
    let center = if preserve_center {
        let position = window.outer_position()?;
        let current_size = window.outer_size()?;
        Some((
            position.x + current_size.width as i32 / 2,
            position.y + current_size.height as i32 / 2,
        ))
    } else {
        None
    };
    window.set_size(PhysicalSize::new(size.0, size.1))?;
    match snapshot.tool_menu.layout {
        ToolMenuLayout::Wheel => apply_circular_region(window)?,
        ToolMenuLayout::Dock | ToolMenuLayout::Vertical => clear_window_region(window)?,
    }
    if let Some(center) = center {
        window.set_position(PhysicalPosition::new(
            center.0 - (size.0 / 2) as i32,
            center.1 - (size.1 / 2) as i32,
        ))?;
    }
    Ok(size)
}

fn publish_window_snapshot<R: Runtime>(
    window: &WebviewWindow<R>,
    snapshot: &QuickLaunchSnapshot,
) -> Result<(), ToolMenuSurfaceError> {
    // Send directly to this hidden WebView before revealing it. It receives
    // exactly the same snapshot that selected the native shape and size.
    window.emit(TOOL_MENU_SURFACE_SNAPSHOT_EVENT, snapshot)?;
    // React state in a hidden WebView can be one render behind. Write the
    // selected layout into the page itself so a dock/vertical surface can
    // never render the cached wheel into a non-circular native region.
    let _ = window.eval(layout_assignment_script(snapshot.tool_menu.layout));
    Ok(())
}

fn layout_assignment_script(layout: ToolMenuLayout) -> &'static str {
    match layout {
        ToolMenuLayout::Wheel => "window.__OPENDESK_TOOL_MENU_LAYOUT='wheel';window.dispatchEvent(new Event('opendesk-tool-menu-layout'));",
        ToolMenuLayout::Dock => "window.__OPENDESK_TOOL_MENU_LAYOUT='dock';window.dispatchEvent(new Event('opendesk-tool-menu-layout'));",
        ToolMenuLayout::Vertical => "window.__OPENDESK_TOOL_MENU_LAYOUT='vertical';window.dispatchEvent(new Event('opendesk-tool-menu-layout'));",
    }
}

fn surface_size(layout: ToolMenuLayout, visible_item_count: usize) -> (u32, u32) {
    let item_count = visible_item_count.max(1);
    match layout {
        ToolMenuLayout::Wheel => {
            let ring_count = wheel_ring_count(item_count);
            let outer_radius = wheel_ring_radius(ring_count.saturating_sub(1)) + 25 + 10;
            let diameter = 264.max(outer_radius * 2);
            // Keep the browser-rendered circular border inside the native
            // region by two transparent pixels on each edge. The native GDI
            // region remains circular for hit testing, while the visible CSS
            // edge is no longer rasterized on the region's stair-stepped edge.
            let surface = (diameter + TOOL_MENU_WHEEL_TRANSPARENT_GUARD).min(700);
            (surface, surface)
        }
        ToolMenuLayout::Dock => {
            let rows = item_count.div_ceil(TOOL_MENU_DOCK_COLUMNS) as u32;
            // Six independent 52px icon slots with five 10px gutters and
            // 12px padding on both sides. Keep this in lockstep with the
            // shared dock Grid instead of relying on separator borders.
            (388, (rows * 52 + rows.saturating_sub(1) * 10 + 26).max(78))
        }
        ToolMenuLayout::Vertical => {
            let columns = item_count.div_ceil(TOOL_MENU_DOCK_COLUMNS) as u32;
            (columns * 52 + columns.saturating_sub(1) * 10 + 26, 390)
        }
    }
}

fn wheel_ring_count(item_count: usize) -> usize {
    let mut remaining = item_count.max(1);
    let mut ring_count = 0usize;
    while remaining > 0 {
        let ring_index = ring_count;
        let radius = wheel_ring_radius(ring_index) as f64;
        let capacity = ((2.0 * std::f64::consts::PI * radius) / 84.0).floor() as usize;
        remaining = remaining.saturating_sub(capacity.max(6));
        ring_count += 1;
    }
    ring_count
}

fn wheel_ring_radius(ring_index: usize) -> u32 {
    if ring_index == 0 {
        68
    } else {
        136 + (ring_index.saturating_sub(1) as u32 * 72)
    }
}

fn begin_hide(visible: &AtomicBool) -> bool {
    visible.swap(false, Ordering::SeqCst)
}

fn restore_visible_after_failed_hide(visible: &AtomicBool) {
    visible.store(true, Ordering::SeqCst);
}

pub fn hide<R: Runtime>(app: &AppHandle<R>) -> Result<(), ToolMenuSurfaceError> {
    if !begin_hide(&TOOL_MENU_VISIBLE) {
        debug_qa::trace("tool-menu hide result=skipped already_hidden_or_closing");
        return Ok(());
    }
    let window = match prepared(app) {
        Ok(window) => window,
        Err(error) => {
            restore_visible_after_failed_hide(&TOOL_MENU_VISIBLE);
            return Err(error);
        }
    };
    let animated = surface_window_animation::fade_hide(&window);
    debug_qa::trace(format!(
        "tool-menu fade-hide result={} duration_ms={}",
        if animated { "animated" } else { "fallback" },
        surface_window_animation::exit_duration_ms(&window)
    ));
    if !animated {
        if let Err(error) = window.hide() {
            restore_visible_after_failed_hide(&TOOL_MENU_VISIBLE);
            return Err(error.into());
        }
    }
    debug_qa::trace("tool-menu hide result=transition_started");
    Ok(())
}

/// A Tauri focus event can be emitted while focus transfers between the
/// top-level window and its WebView child. Only close when Windows confirms
/// that foreground has actually moved to another root HWND.
#[cfg(windows)]
pub fn lost_foreground<R: Runtime>(window: &tauri::Window<R>) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetAncestor, GetForegroundWindow, GA_ROOT};

    let Ok(surface) = window
        .app_handle()
        .get_webview_window(TOOL_MENU_SURFACE_LABEL)
        .ok_or(())
    else {
        return false;
    };
    let Ok(surface_hwnd) = surface.hwnd() else {
        return false;
    };
    let surface_root = unsafe { GetAncestor(surface_hwnd.0, GA_ROOT) };
    let foreground = unsafe { GetForegroundWindow() };
    if surface_root.is_null() || foreground.is_null() {
        return false;
    }
    let foreground_root = unsafe { GetAncestor(foreground, GA_ROOT) };
    !foreground_root.is_null() && foreground_root != surface_root
}

#[cfg(not(windows))]
pub fn lost_foreground<R: Runtime>(_window: &tauri::Window<R>) -> bool {
    true
}

/// One framework-managed close path for key release, Esc, launch and click-away.
/// The document fades without changing HWND styles or geometry; Tauri/Tao
/// performs the delayed final hide to keep cached visibility coherent.
pub fn request_hide<R: Runtime>(app: &AppHandle<R>) -> Result<(), ToolMenuSurfaceError> {
    hide(app)
}

#[cfg(windows)]
fn popup_window_style(style: isize) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
    };

    let frame_styles = WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
    (style & !(frame_styles as isize)) | WS_POPUP as isize
}

#[cfg(windows)]
fn install_popup_style_guard<R: Runtime>(
    window: &WebviewWindow<R>,
) -> Result<(), ToolMenuSurfaceError> {
    use windows_sys::Win32::UI::Shell::SetWindowSubclass;

    let hwnd = window
        .hwnd()
        .map_err(|_| ToolMenuSurfaceError::ConfigurePopupStyle)?;
    let installed = unsafe {
        SetWindowSubclass(
            hwnd.0,
            Some(popup_style_subclass_proc),
            TOOL_MENU_POPUP_STYLE_SUBCLASS_ID,
            0,
        ) != 0
    };
    if !installed {
        return Err(ToolMenuSurfaceError::ConfigurePopupStyle);
    }
    Ok(())
}

#[cfg(windows)]
unsafe extern "system" fn popup_style_subclass_proc(
    window: windows_sys::Win32::Foundation::HWND,
    message: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
    subclass_id: usize,
    _reference_data: usize,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_STYLE, STYLESTRUCT, WM_ERASEBKGND, WM_NCACTIVATE, WM_NCDESTROY, WM_NCPAINT,
        WM_STYLECHANGING,
    };

    if message == WM_STYLECHANGING && wparam as isize == GWL_STYLE as isize && lparam != 0 {
        let change = unsafe { &mut *(lparam as *mut STYLESTRUCT) };
        let requested = change.styleNew;
        change.styleNew = popup_window_style(requested as isize) as u32;
        debug_qa::trace(format!(
            "tool-menu popup-style-guard requested={requested:#x} applied={:#x}",
            change.styleNew
        ));
    }

    // The transparent WebView covers the whole client area. Letting DefWindowProc
    // repaint the retained Tao caption/background surface on deactivation exposes
    // a light strip above the fading document even though WS_CAPTION is gone.
    if message == WM_NCACTIVATE {
        debug_qa::trace(format!(
            "tool-menu native-paint suppressed=ncactivate active={}",
            wparam != 0
        ));
        return 1;
    }
    if message == WM_NCPAINT {
        debug_qa::trace("tool-menu native-paint suppressed=ncpaint");
        return 0;
    }
    if message == WM_ERASEBKGND {
        debug_qa::trace("tool-menu native-paint suppressed=erase-background");
        return 1;
    }

    let result = unsafe { DefSubclassProc(window, message, wparam, lparam) };
    if message == WM_NCDESTROY {
        unsafe {
            RemoveWindowSubclass(window, Some(popup_style_subclass_proc), subclass_id);
        }
    }
    result
}

/// Tao can retain caption/system-menu bits on an undecorated transparent
/// window. They are normally covered by the WebView, but become visible as a
/// clipped title-bar cap when the document fades. Make the native surface a
/// real popup before its first visible frame.
#[cfg(windows)]
fn configure_popup_style<R: Runtime>(
    window: &WebviewWindow<R>,
) -> Result<(), ToolMenuSurfaceError> {
    use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_STYLE, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    };

    let hwnd = window
        .hwnd()
        .map_err(|_| ToolMenuSurfaceError::ConfigurePopupStyle)?;
    unsafe {
        SetLastError(0);
        let original_style = GetWindowLongPtrW(hwnd.0, GWL_STYLE);
        if original_style == 0 && GetLastError() != 0 {
            return Err(ToolMenuSurfaceError::ConfigurePopupStyle);
        }
        let popup_style = popup_window_style(original_style);
        SetLastError(0);
        let previous_style = SetWindowLongPtrW(hwnd.0, GWL_STYLE, popup_style);
        if previous_style == 0 && GetLastError() != 0 {
            return Err(ToolMenuSurfaceError::ConfigurePopupStyle);
        }
        if SetWindowPos(
            hwnd.0,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
        ) == 0
        {
            return Err(ToolMenuSurfaceError::ConfigurePopupStyle);
        }
        debug_qa::trace(format!(
            "tool-menu popup-style result=applied original={original_style:#x} current={:#x}",
            GetWindowLongPtrW(hwnd.0, GWL_STYLE)
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn install_popup_style_guard<R: Runtime>(
    _window: &WebviewWindow<R>,
) -> Result<(), ToolMenuSurfaceError> {
    Ok(())
}

#[cfg(not(windows))]
fn configure_popup_style<R: Runtime>(
    _window: &WebviewWindow<R>,
) -> Result<(), ToolMenuSurfaceError> {
    Ok(())
}

#[cfg(windows)]
fn pointer_position() -> Result<(i32, i32), ToolMenuSurfaceError> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let mut point: POINT = unsafe { std::mem::zeroed() };
    if unsafe { GetCursorPos(&mut point) } == 0 {
        return Err(ToolMenuSurfaceError::CursorPosition);
    }
    Ok((point.x, point.y))
}

#[cfg(not(windows))]
fn pointer_position() -> Result<(i32, i32), ToolMenuSurfaceError> {
    Ok((0, 0))
}

fn placement_for_pointer<R: Runtime>(
    window: &WebviewWindow<R>,
    pointer: (i32, i32),
    size: (u32, u32),
) -> Result<(i32, i32), ToolMenuSurfaceError> {
    let monitors = window.available_monitors()?;
    let monitor = monitors
        .iter()
        .find(|monitor| {
            let position = monitor.position();
            let monitor_size = monitor.size();
            point_in_rect(
                pointer,
                (position.x, position.y),
                (monitor_size.width, monitor_size.height),
            )
        })
        .or_else(|| {
            monitors.iter().min_by_key(|monitor| {
                let position = monitor.position();
                let monitor_size = monitor.size();
                squared_distance_to_rect(
                    pointer,
                    (position.x, position.y),
                    (monitor_size.width, monitor_size.height),
                )
            })
        })
        .ok_or(ToolMenuSurfaceError::InvalidDimensions)?;
    let work = monitor.work_area();
    fit_surface_to_work_area(
        pointer,
        size,
        (work.position.x, work.position.y),
        (work.size.width, work.size.height),
    )
    .ok_or(ToolMenuSurfaceError::InvalidDimensions)
}

#[cfg(windows)]
fn apply_circular_region<R: Runtime>(
    window: &WebviewWindow<R>,
) -> Result<(), ToolMenuSurfaceError> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{CreateEllipticRgn, DeleteObject, SetWindowRgn};
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;

    let hwnd = window
        .hwnd()
        .map_err(|_| ToolMenuSurfaceError::CreateRegion)?;
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    if unsafe { GetClientRect(hwnd.0, &mut rect) } == 0 || rect.right <= 0 || rect.bottom <= 0 {
        return Err(ToolMenuSurfaceError::CreateRegion);
    }
    // The lower-right coordinates are exclusive. Extending them one pixel
    // preserves the final edge pixel while keeping every square corner outside
    // the native hit-test and paint region.
    let region = unsafe { CreateEllipticRgn(0, 0, rect.right + 1, rect.bottom + 1) };
    if region.is_null() {
        return Err(ToolMenuSurfaceError::CreateRegion);
    }
    if unsafe { SetWindowRgn(hwnd.0, region, 1) } == 0 {
        unsafe { DeleteObject(region) };
        return Err(ToolMenuSurfaceError::ApplyRegion);
    }
    Ok(())
}

/// The menu is a circular content surface, not a conventional application
/// window. Disable DWM's non-client painting and remove the border color so a
/// title-bar or frame surface can never be composed above the transparent
/// WebView while it closes. DWM's automatic transitions stay disabled so only
/// the document-level exit transition participates in composition.
#[cfg(windows)]
fn configure_transparent_non_client<R: Runtime>(window: &WebviewWindow<R>) {
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_NCRENDERING_POLICY,
        DWMWA_TRANSITIONS_FORCEDISABLED,
    };

    let Ok(hwnd) = window.hwnd() else {
        debug_qa::trace("tool-menu non-client result=unavailable no_hwnd");
        return;
    };
    let policy = DWMNCRP_DISABLED;
    let policy_result = unsafe {
        DwmSetWindowAttribute(
            hwnd.0,
            DWMWA_NCRENDERING_POLICY as u32,
            (&policy as *const i32).cast(),
            std::mem::size_of_val(&policy) as u32,
        )
    };
    // Supported on Windows 11. Older Windows retains the disabled non-client
    // policy even if it does not recognize this color attribute.
    const DWMWA_COLOR_NONE: u32 = 0xffff_fffe;
    let border_result = unsafe {
        DwmSetWindowAttribute(
            hwnd.0,
            DWMWA_BORDER_COLOR as u32,
            (&DWMWA_COLOR_NONE as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        )
    };
    let transitions_disabled = 1_i32;
    let transition_result = unsafe {
        DwmSetWindowAttribute(
            hwnd.0,
            DWMWA_TRANSITIONS_FORCEDISABLED as u32,
            (&transitions_disabled as *const i32).cast(),
            std::mem::size_of_val(&transitions_disabled) as u32,
        )
    };
    debug_qa::trace(format!(
        "tool-menu non-client result=applied policy_hresult={policy_result:#x} border_hresult={border_result:#x} transition_hresult={transition_result:#x}"
    ));
}

#[cfg(not(windows))]
fn configure_transparent_non_client<R: Runtime>(_window: &WebviewWindow<R>) {}

#[cfg(windows)]
fn clear_window_region<R: Runtime>(window: &WebviewWindow<R>) -> Result<(), ToolMenuSurfaceError> {
    use windows_sys::Win32::Graphics::Gdi::SetWindowRgn;

    let hwnd = window
        .hwnd()
        .map_err(|_| ToolMenuSurfaceError::ApplyRegion)?;
    if unsafe { SetWindowRgn(hwnd.0, std::ptr::null_mut(), 1) } == 0 {
        return Err(ToolMenuSurfaceError::ApplyRegion);
    }
    Ok(())
}

#[cfg(not(windows))]
fn clear_window_region<R: Runtime>(_window: &WebviewWindow<R>) -> Result<(), ToolMenuSurfaceError> {
    Ok(())
}

#[cfg(not(windows))]
fn apply_circular_region<R: Runtime>(
    _window: &WebviewWindow<R>,
) -> Result<(), ToolMenuSurfaceError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_close_requests_enter_native_hide_only_once() {
        let visible = AtomicBool::new(true);

        assert!(begin_hide(&visible));
        assert!(!begin_hide(&visible));
    }

    #[test]
    fn failed_native_hide_restores_the_close_gate_for_retry() {
        let visible = AtomicBool::new(true);

        assert!(begin_hide(&visible));
        restore_visible_after_failed_hide(&visible);
        assert!(begin_hide(&visible));
    }

    #[cfg(windows)]
    #[test]
    fn popup_style_removes_every_native_caption_control() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
        };

        let decorated =
            (WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX) as isize;
        let style = popup_window_style(decorated);

        assert_ne!(style & WS_POPUP as isize, 0);
        assert_eq!(style & decorated, 0);
    }

    #[test]
    fn tool_menu_surface_can_subscribe_to_tauri_events() {
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../../capabilities/default.json"))
                .expect("default capability should be valid JSON");
        let windows = capability["windows"]
            .as_array()
            .expect("default capability should declare its windows");

        assert!(windows
            .iter()
            .any(|label| label.as_str() == Some(TOOL_MENU_SURFACE_LABEL)));
    }
}
