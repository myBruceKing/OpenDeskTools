use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use thiserror::Error;

use super::quick_launch::{QuickLaunchSnapshot, ToolMenuLayout};

pub const TOOL_MENU_SURFACE_LABEL: &str = "tool-menu-surface";
pub const TOOL_MENU_SURFACE_SHOWN_EVENT: &str = "tool-menu://shown";
pub const TOOL_MENU_SURFACE_SNAPSHOT_EVENT: &str = "tool-menu://snapshot";
pub const TOOL_MENU_SURFACE_CLOSING_EVENT: &str = "tool-menu://closing";
const TOOL_MENU_SURFACE_ROUTE: &str = "index.html#tool-menu-surface";
const TOOL_MENU_SURFACE_SIZE: f64 = 320.0;
const TOOL_MENU_DOCK_COLUMNS: usize = 6;
const TOOL_MENU_WHEEL_TRANSPARENT_GUARD: u32 = 4;
const TOOL_MENU_CLOSE_ANIMATION: Duration = Duration::from_millis(95);
static HIDE_GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum ToolMenuSurfaceError {
    #[error("the tool menu surface is not prepared")]
    NotPrepared,
    #[error("tool menu quick launch state is unavailable: {0}")]
    QuickLaunch(String),
    #[error("tool menu window operation failed: {0}")]
    Window(#[from] tauri::Error),
    #[cfg(windows)]
    #[error("Windows could not read the cursor position")]
    CursorPosition,
    #[cfg(windows)]
    #[error("Windows could not create the tool menu circular region")]
    CreateRegion,
    #[cfg(windows)]
    #[error("Windows could not apply the tool menu circular region")]
    ApplyRegion,
}

/// Construct the launcher WebView during setup. Hotkey callbacks only reveal
/// this already-created surface, so they cannot block the keyboard hook path.
pub fn prepare<R: Runtime>(app: &AppHandle<R>) -> Result<(), ToolMenuSurfaceError> {
    if app.get_webview_window(TOOL_MENU_SURFACE_LABEL).is_some() {
        return Ok(());
    }
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
    apply_circular_region(&window)?;
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
    // A new press always wins over a delayed close from the previous release.
    HIDE_GENERATION.fetch_add(1, Ordering::SeqCst);
    let window = prepared(app)?;
    let visible_item_count = snapshot.pinned_apps.iter().filter(|app| app.visible).count();
    let size = surface_size(snapshot.tool_menu.layout, visible_item_count);
    window.set_size(PhysicalSize::new(size.0, size.1))?;
    match snapshot.tool_menu.layout {
        ToolMenuLayout::Wheel => apply_circular_region(&window)?,
        ToolMenuLayout::Dock | ToolMenuLayout::Vertical => clear_window_region(&window)?,
    }
    let pointer = pointer_position()?;
    window.set_position(PhysicalPosition::new(
        pointer.0 - (size.0 / 2) as i32,
        pointer.1 - (size.1 / 2) as i32,
    ))?;
    // Send directly to this hidden WebView before revealing it. It receives
    // exactly the same snapshot that selected the native shape and size.
    window.emit(TOOL_MENU_SURFACE_SNAPSHOT_EVENT, snapshot)?;
    // React state in a hidden WebView can be one render behind. Write the
    // selected layout into the page itself so a dock/vertical surface can
    // never render the cached wheel into a non-circular native region.
    let _ = window.eval(layout_assignment_script(snapshot.tool_menu.layout));
    window.show()?;
    window.set_focus()?;
    window.emit(TOOL_MENU_SURFACE_SHOWN_EVENT, ())?;
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
            let outer_radius = 77 + ring_count.saturating_sub(1) as u32 * 75 + 25 + 12;
            let diameter = 264.max(outer_radius * 2);
            // Keep the browser-rendered circular border inside the native
            // region by two transparent pixels on each edge. The native GDI
            // region remains circular for hit testing, while the visible CSS
            // edge is no longer rasterized on the region's stair-stepped edge.
            let surface = (diameter + TOOL_MENU_WHEEL_TRANSPARENT_GUARD).min(700);
            (surface, surface)
        }
        ToolMenuLayout::Dock => {
            let rows = ((item_count + TOOL_MENU_DOCK_COLUMNS - 1) / TOOL_MENU_DOCK_COLUMNS) as u32;
            // Six independent 52px icon slots with five 10px gutters and
            // 12px padding on both sides. Keep this in lockstep with the
            // shared dock Grid instead of relying on separator borders.
            (388, (rows * 52 + rows.saturating_sub(1) * 10 + 26).max(78))
        }
        ToolMenuLayout::Vertical => {
            let columns = ((item_count + TOOL_MENU_DOCK_COLUMNS - 1) / TOOL_MENU_DOCK_COLUMNS) as u32;
            (columns * 52 + columns.saturating_sub(1) * 10 + 26, 390)
        }
    }
}

fn wheel_ring_count(item_count: usize) -> usize {
    let mut remaining = item_count.max(1);
    let mut ring_count = 0usize;
    while remaining > 0 {
        let ring_index = ring_count;
        let radius = 77.0 + ring_index as f64 * 75.0;
        let capacity = ((2.0 * std::f64::consts::PI * radius) / 94.0).floor() as usize;
        remaining = remaining.saturating_sub(capacity.max(6));
        ring_count += 1;
    }
    ring_count
}

pub fn hide<R: Runtime>(app: &AppHandle<R>) -> Result<(), ToolMenuSurfaceError> {
    prepared(app)?.hide()?;
    Ok(())
}

/// Animate the WebView content back into its center point, then hide the
/// native HWND. This avoids a blank native window or document title flashing
/// after the visual content has already vanished.
pub fn request_hide<R: Runtime>(app: &AppHandle<R>) -> Result<(), ToolMenuSurfaceError> {
    let window = prepared(app)?;
    let generation = HIDE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    window.emit(TOOL_MENU_SURFACE_CLOSING_EVENT, ())?;
    let delayed_app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(TOOL_MENU_CLOSE_ANIMATION);
        if HIDE_GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        let main_thread_app = delayed_app.clone();
        let _ = delayed_app.run_on_main_thread(move || {
            if HIDE_GENERATION.load(Ordering::SeqCst) != generation {
                return;
            }
            if let Err(error) = hide(&main_thread_app) {
                eprintln!("failed to hide tool menu after close animation: {error}");
            }
        });
    });
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

#[cfg(windows)]
fn apply_circular_region<R: Runtime>(window: &WebviewWindow<R>) -> Result<(), ToolMenuSurfaceError> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{CreateEllipticRgn, DeleteObject, SetWindowRgn};
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;

    let hwnd = window.hwnd().map_err(|_| ToolMenuSurfaceError::CreateRegion)?;
    let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
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

#[cfg(windows)]
fn clear_window_region<R: Runtime>(window: &WebviewWindow<R>) -> Result<(), ToolMenuSurfaceError> {
    use windows_sys::Win32::Graphics::Gdi::SetWindowRgn;

    let hwnd = window.hwnd().map_err(|_| ToolMenuSurfaceError::ApplyRegion)?;
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
fn apply_circular_region<R: Runtime>(_window: &WebviewWindow<R>) -> Result<(), ToolMenuSurfaceError> {
    Ok(())
}
