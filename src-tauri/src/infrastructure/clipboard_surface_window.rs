use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use tauri::{
    window::Color, AppHandle, Emitter, Manager, Monitor, Runtime, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};
use thiserror::Error;

use super::application::ApplicationRuntime;
use super::clipboard_surface_foreground::{self, ForegroundMonitorError};
use super::clipboard_surface_pointer::{self, PointerMonitorError};
use super::debug_qa;
use super::surface::{SurfaceError, SurfaceManager};
use super::surface_window_animation;

pub const CLIPBOARD_SURFACE_LABEL: &str = "clipboard-surface";
pub const CLIPBOARD_PREVIEW_SURFACE_LABEL: &str = "clipboard-preview-surface";
const CLIPBOARD_SURFACE_ROUTE: &str = "index.html#clipboard-surface";
const CLIPBOARD_PREVIEW_SURFACE_ROUTE: &str = "index.html#clipboard-preview-surface";
const CLIPBOARD_SURFACE_WIDTH: f64 = 380.0;
const CLIPBOARD_SURFACE_HEIGHT: f64 = 520.0;
const CLIPBOARD_PREVIEW_SURFACE_WIDTH: f64 = 330.0;
const CLIPBOARD_PREVIEW_SURFACE_HEIGHT: f64 = 230.0;
// Keep the native HWND region aligned with the shared --radius-window token.
const CLIPBOARD_SURFACE_CORNER_RADIUS: f64 = 12.0;
const CLIPBOARD_SURFACE_CURSOR_GAP: f64 = 12.0;
const DEFAULT_CLIPBOARD_SURFACE_UNDERLAY: ClipboardSurfaceUnderlayColor =
    ClipboardSurfaceUnderlayColor::new(0xe0, 0xde, 0xdc);
const CLIPBOARD_SURFACE_STATE_EVENT: &str = "clipboard://history-changed";
const CLIPBOARD_PREVIEW_STATE_EVENT: &str = "clipboard://preview-changed";
pub(crate) const CLIPBOARD_SURFACE_OPENED_CHANGE: &str = "surface_opened";
pub(crate) const CLIPBOARD_SURFACE_CLOSED_CHANGE: &str = "surface_closed";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardSurfaceUnderlayColor {
    red: u8,
    green: u8,
    blue: u8,
}

impl ClipboardSurfaceUnderlayColor {
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }

    pub fn parse_hex(value: &str) -> Option<Self> {
        let bytes = value.as_bytes();
        if bytes.len() != 7 || bytes[0] != b'#' {
            return None;
        }
        Some(Self::new(
            parse_hex_byte(bytes[1], bytes[2])?,
            parse_hex_byte(bytes[3], bytes[4])?,
            parse_hex_byte(bytes[5], bytes[6])?,
        ))
    }

    fn as_tauri(self) -> Color {
        Color(self.red, self.green, self.blue, 255)
    }

    fn as_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.red, self.green, self.blue)
    }
}

fn parse_hex_byte(high: u8, low: u8) -> Option<u8> {
    Some(hex_nibble(high)? << 4 | hex_nibble(low)?)
}

const fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardSurfaceCloseReason {
    HotkeyToggle,
    ForcedHotkeyToggle,
    #[cfg(debug_assertions)]
    DebugQaReset,
    WindowRequest,
    FocusLost,
    ForegroundChanged,
    PointerOutside,
    PreviewDestroyed,
    Command,
    InputSucceeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardPreviewCloseReason {
    MainSurfaceClosing,
    ForegroundChanged,
    PointerOutside,
    Command,
    WindowRequest,
    MainSurfaceDestroyed,
}

impl ClipboardPreviewCloseReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MainSurfaceClosing => "main_surface_closing",
            Self::ForegroundChanged => "foreground_changed",
            Self::PointerOutside => "pointer_outside",
            Self::Command => "command",
            Self::WindowRequest => "window_request",
            Self::MainSurfaceDestroyed => "main_surface_destroyed",
        }
    }
}

impl ClipboardSurfaceCloseReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::HotkeyToggle => "hotkey_toggle",
            Self::ForcedHotkeyToggle => "forced_hotkey_toggle",
            #[cfg(debug_assertions)]
            Self::DebugQaReset => "debug_qa_reset",
            Self::WindowRequest => "window_request",
            Self::FocusLost => "focused_false",
            Self::ForegroundChanged => "foreground_changed",
            Self::PointerOutside => "pointer_outside",
            Self::PreviewDestroyed => "preview_destroyed",
            Self::Command => "command",
            Self::InputSucceeded => "input_succeeded",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClipboardSurfaceStateEvent {
    change: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClipboardPreviewStateEvent {
    change: &'static str,
    record_id: Option<String>,
    visible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardPreviewSurfaceState {
    pub record_id: Option<String>,
    pub visible: bool,
}

#[derive(Debug, Default)]
struct ClipboardPreviewSelection {
    record_id: Option<String>,
}

static CLIPBOARD_PREVIEW_SELECTION: OnceLock<Mutex<ClipboardPreviewSelection>> = OnceLock::new();

fn preview_selection() -> &'static Mutex<ClipboardPreviewSelection> {
    CLIPBOARD_PREVIEW_SELECTION.get_or_init(|| Mutex::new(ClipboardPreviewSelection::default()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PixelPoint {
    x: i32,
    y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PixelSize {
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PixelRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct MonitorGeometry {
    bounds: PixelRect,
    work_area: PixelRect,
    scale_factor: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SurfacePlacement {
    position: PixelPoint,
    size: PixelSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SurfaceAnchorSource {
    Caret,
    Cursor,
}

impl SurfaceAnchorSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Caret => "caret",
            Self::Cursor => "cursor",
        }
    }
}

trait NativeVisibilityApi {
    #[cfg(test)]
    fn hide(&mut self, window: usize);
    fn is_visible(&mut self, window: usize) -> bool;
}

#[derive(Debug, Error)]
pub enum ClipboardSurfaceWindowError {
    #[error(transparent)]
    Tauri(#[from] tauri::Error),
    #[error(transparent)]
    Surface(#[from] SurfaceError),
    #[error("clipboard surface native handle is unavailable")]
    NativeHandle,
    #[error("clipboard surface dimensions are invalid")]
    InvalidDimensions,
    #[error("clipboard surface remained visible after the native hide request")]
    StillVisible,
    #[error("clipboard preview surface requires the main clipboard surface to be visible")]
    MainSurfaceNotVisible,
    #[error("clipboard surface window group was not prepared on the Tauri main thread")]
    SurfaceGroupNotPrepared,
    #[error("clipboard preview surface state lock is poisoned")]
    PreviewStatePoisoned,
    #[error("clipboard preview surface remained hidden after the native show request")]
    PreviewStillHidden,
    #[error(transparent)]
    ForegroundMonitor(#[from] ForegroundMonitorError),
    #[error(transparent)]
    PointerMonitor(#[from] PointerMonitorError),
    #[cfg(windows)]
    #[error("Windows could not read the cursor position")]
    CursorPosition,
    #[cfg(windows)]
    #[error("Windows clipboard surface operation failed: {0}")]
    WindowsApi(&'static str),
    #[cfg(windows)]
    #[error("Windows clipboard surface operation failed: {operation} (error {code})")]
    WindowsApiCode { operation: &'static str, code: u32 },
    #[cfg(windows)]
    #[error("Windows could not create the clipboard surface region")]
    CreateRegion,
    #[cfg(windows)]
    #[error("Windows could not apply the clipboard surface region")]
    ApplyRegion,
    #[cfg(windows)]
    #[error("Windows clipboard surface region does not cover the full client area")]
    InvalidRegionGeometry,
    #[cfg(not(windows))]
    #[error("native clipboard surface shaping is unavailable on this platform")]
    UnsupportedPlatform,
}

fn get_or_create_main<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<WebviewWindow<R>, ClipboardSurfaceWindowError> {
    if let Some(window) = app.get_webview_window(CLIPBOARD_SURFACE_LABEL) {
        debug_qa::trace("surface group main result=existing");
        configure_native_popup(&window)?;
        refresh_native_shape_or_log(&window);
        return Ok(window);
    }
    debug_qa::trace("surface group main stage=build requested");
    let result = WebviewWindowBuilder::new(
        app,
        CLIPBOARD_SURFACE_LABEL,
        WebviewUrl::App(CLIPBOARD_SURFACE_ROUTE.into()),
    )
    .title("OpenDeskTools Clipboard")
    .inner_size(CLIPBOARD_SURFACE_WIDTH, CLIPBOARD_SURFACE_HEIGHT)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .background_color(DEFAULT_CLIPBOARD_SURFACE_UNDERLAY.as_tauri())
    .visible(false)
    .focused(false)
    .build()
    .map_err(ClipboardSurfaceWindowError::from)
    .and_then(|window| {
        debug_qa::trace("surface group main result=created_hidden");
        configure_native_popup(&window)?;
        refresh_native_shape_or_log(&window);
        Ok(window)
    });
    if let Err(error) = &result {
        debug_qa::trace(format!("surface group main result=error error={error}"));
    }
    result
}

/// Builds both hidden popup WebViews from the Tauri setup/main-thread path.
/// Runtime hover commands only reuse this prepared group and never call a
/// WebviewWindowBuilder from an IPC handler.
pub fn prepare_group<R: Runtime>(app: &AppHandle<R>) -> Result<(), ClipboardSurfaceWindowError> {
    debug_qa::trace("surface group prepare requested");
    let result = (|| {
        let main = get_or_create_main(app)?;
        let preview = get_or_create_preview(app)?;
        if native_is_visible(&main)? || native_is_visible(&preview)? {
            // Setup preparation is a hidden-only contract. A visible member
            // would expose a partial group before target capture.
            hide_native_verified(&preview)?;
            hide_native_verified(&main)?;
        }
        Ok(())
    })();
    match &result {
        Ok(()) => debug_qa::trace("surface group prepare result=ready main=hidden preview=hidden"),
        Err(error) => {
            debug_qa::trace(format!(
                "surface group prepare result=error policy=clipboard_surface_unavailable error={error}"
            ));
            destroy_prepared_surface_group_or_log(app);
        }
    }
    result
}

pub fn prepared_main<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<WebviewWindow<R>, ClipboardSurfaceWindowError> {
    let main = app.get_webview_window(CLIPBOARD_SURFACE_LABEL);
    let preview_prepared = app
        .get_webview_window(CLIPBOARD_PREVIEW_SURFACE_LABEL)
        .is_some();
    require_prepared_surface_group(main.is_some(), preview_prepared)?;
    let main = main.ok_or(ClipboardSurfaceWindowError::SurfaceGroupNotPrepared)?;
    Ok(main)
}

fn prepared_preview<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<WebviewWindow<R>, ClipboardSurfaceWindowError> {
    let main_prepared = app.get_webview_window(CLIPBOARD_SURFACE_LABEL).is_some();
    let preview = app.get_webview_window(CLIPBOARD_PREVIEW_SURFACE_LABEL);
    require_prepared_surface_group(main_prepared, preview.is_some())?;
    preview.ok_or(ClipboardSurfaceWindowError::SurfaceGroupNotPrepared)
}

fn require_prepared_surface_group(
    main_prepared: bool,
    preview_prepared: bool,
) -> Result<(), ClipboardSurfaceWindowError> {
    if main_prepared && preview_prepared {
        Ok(())
    } else {
        Err(ClipboardSurfaceWindowError::SurfaceGroupNotPrepared)
    }
}

fn apply_underlay_to_prepared_group<F>(
    main_prepared: bool,
    preview_prepared: bool,
    color: ClipboardSurfaceUnderlayColor,
    mut apply: F,
) -> Result<(), ClipboardSurfaceWindowError>
where
    F: FnMut(
        &'static str,
        ClipboardSurfaceUnderlayColor,
    ) -> Result<(), ClipboardSurfaceWindowError>,
{
    require_prepared_surface_group(main_prepared, preview_prepared)?;
    let mut first_error = None;
    for label in [CLIPBOARD_SURFACE_LABEL, CLIPBOARD_PREVIEW_SURFACE_LABEL] {
        match apply(label, color) {
            Ok(()) => debug_qa::trace(format!(
                "surface group underlay label={label} color={} result=success",
                color.as_hex()
            )),
            Err(error) => {
                debug_qa::trace(format!(
                    "surface group underlay label={label} color={} result=error error={error}",
                    color.as_hex()
                ));
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

pub fn set_group_underlay_color<R: Runtime>(
    app: &AppHandle<R>,
    color: ClipboardSurfaceUnderlayColor,
) -> Result<(), ClipboardSurfaceWindowError> {
    let main_prepared = app.get_webview_window(CLIPBOARD_SURFACE_LABEL).is_some();
    let preview_prepared = app
        .get_webview_window(CLIPBOARD_PREVIEW_SURFACE_LABEL)
        .is_some();
    debug_qa::trace(format!(
        "surface group underlay request color={} main_prepared={main_prepared} preview_prepared={preview_prepared}",
        color.as_hex()
    ));
    let result =
        apply_underlay_to_prepared_group(main_prepared, preview_prepared, color, |label, color| {
            app.get_webview_window(label)
                .ok_or(ClipboardSurfaceWindowError::SurfaceGroupNotPrepared)?
                .set_background_color(Some(color.as_tauri()))
                .map_err(ClipboardSurfaceWindowError::from)
        });
    match &result {
        Ok(()) => debug_qa::trace(format!(
            "surface group underlay result=success color={}",
            color.as_hex()
        )),
        Err(error) => debug_qa::trace(format!(
            "surface group underlay result=error color={} error={error}",
            color.as_hex()
        )),
    }
    result
}

fn destroy_prepared_surface_group_or_log<R: Runtime>(app: &AppHandle<R>) {
    for label in [CLIPBOARD_PREVIEW_SURFACE_LABEL, CLIPBOARD_SURFACE_LABEL] {
        let Some(window) = app.get_webview_window(label) else {
            continue;
        };
        if let Err(error) = window.destroy() {
            debug_qa::trace(format!(
                "surface group cleanup label={label} result=error error={error}"
            ));
        } else {
            debug_qa::trace(format!(
                "surface group cleanup label={label} result=destroyed"
            ));
        }
    }
    let _ = forget_preview_state();
}

fn get_or_create_preview<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<WebviewWindow<R>, ClipboardSurfaceWindowError> {
    if let Some(window) = app.get_webview_window(CLIPBOARD_PREVIEW_SURFACE_LABEL) {
        debug_qa::trace("preview get_or_create result=existing");
        configure_native_popup(&window).inspect_err(|error| {
            debug_qa::trace(format!(
                "preview native configure stage=existing result=error error={error}"
            ));
        })?;
        debug_qa::trace("preview native configure stage=existing result=success");
        refresh_native_shape_or_log(&window);
        return Ok(window);
    }
    debug_qa::trace("preview get_or_create stage=build requested");
    let result = WebviewWindowBuilder::new(
        app,
        CLIPBOARD_PREVIEW_SURFACE_LABEL,
        WebviewUrl::App(CLIPBOARD_PREVIEW_SURFACE_ROUTE.into()),
    )
    .title("OpenDeskTools Clipboard Preview")
    .inner_size(
        CLIPBOARD_PREVIEW_SURFACE_WIDTH,
        CLIPBOARD_PREVIEW_SURFACE_HEIGHT,
    )
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .decorations(false)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .background_color(DEFAULT_CLIPBOARD_SURFACE_UNDERLAY.as_tauri())
    .visible(false)
    .focused(false)
    .build()
    .map_err(ClipboardSurfaceWindowError::from)
    .and_then(|window| {
        debug_qa::trace("preview get_or_create result=created_hidden");
        configure_native_popup(&window).inspect_err(|error| {
            debug_qa::trace(format!(
                "preview native configure stage=created result=error error={error}"
            ));
        })?;
        debug_qa::trace("preview native configure stage=created result=success");
        refresh_native_shape_or_log(&window);
        Ok(window)
    });
    if let Err(error) = &result {
        debug_qa::trace(format!("preview get_or_create result=error error={error}"));
    }
    result
}

pub fn show<R: Runtime>(
    window: &WebviewWindow<R>,
    surface: &SurfaceManager,
) -> Result<(), ClipboardSurfaceWindowError> {
    #[cfg(windows)]
    let captured_target_top_window = surface.target_top_window();
    #[cfg(windows)]
    let target_top_window = captured_target_top_window.or_else(native_foreground_root);
    configure_native_popup(window)?;
    let placement = placement_for_current_anchor(window, captured_target_top_window)?;
    surface_window_animation::prepare_show(window);
    show_native_no_activate(window, placement)?;
    refresh_native_shape_or_log(window);
    #[cfg(windows)]
    if let Err(error) = start_surface_monitors(window.app_handle(), target_top_window) {
        let _ = hide_native_verified(window);
        return Err(error);
    }
    #[cfg(not(windows))]
    let _ = surface;
    Ok(())
}

pub fn notify_opened<R: Runtime>(app: &AppHandle<R>) {
    notify_state_or_log(app, CLIPBOARD_SURFACE_OPENED_CHANGE);
}

/// Hides first and clears the target only after the native window accepted the
/// close. WS_EX_NOACTIVATE keeps the original application focused, so normal
/// toggle/close paths do not restore or activate it again.
pub fn close<R: Runtime>(
    app: &AppHandle<R>,
    surface: &SurfaceManager,
    reason: ClipboardSurfaceCloseReason,
) -> Result<(), ClipboardSurfaceWindowError> {
    debug_qa::trace(format!("close request reason={}", reason.as_str()));
    if let Err(error) = clipboard_surface_foreground::stop() {
        // Closing the visible surfaces is the fail-safe. A monitor teardown
        // fault must not strand an always-on-top window on screen.
        eprintln!("failed to stop clipboard foreground monitor while closing: {error}");
    }
    if let Err(error) = clipboard_surface_pointer::stop() {
        eprintln!("failed to stop clipboard outside-pointer monitor while closing: {error}");
    }
    close_preview(app, ClipboardPreviewCloseReason::MainSurfaceClosing)?;
    let window = app.get_webview_window(CLIPBOARD_SURFACE_LABEL);
    let result = hide_then_clear(
        || match &window {
            Some(window) => hide_native_verified(window),
            None => Ok(()),
        },
        || surface.clear().map_err(ClipboardSurfaceWindowError::from),
    );
    if matches!(&result, Err(ClipboardSurfaceWindowError::Surface(_))) {
        if let Some(window) = &window {
            if let Err(show_error) = show(window, surface) {
                eprintln!(
                    "clipboard surface state cleanup failed and the window could not reshow: {show_error}"
                );
            }
        }
    }
    result?;
    notify_state_or_log(app, CLIPBOARD_SURFACE_CLOSED_CHANGE);
    debug_qa::trace(format!("close success reason={}", reason.as_str()));
    Ok(())
}

#[cfg(windows)]
fn native_foreground_root() -> Option<usize> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetAncestor, GetForegroundWindow, GA_ROOT};

    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_null() {
        return None;
    }
    let root = unsafe { GetAncestor(foreground, GA_ROOT) };
    (!root.is_null()).then_some(root as usize)
}

#[cfg(windows)]
fn start_surface_monitors<R: Runtime>(
    app: &AppHandle<R>,
    target_top_window: Option<usize>,
) -> Result<(), ClipboardSurfaceWindowError> {
    let internal_surface_roots = internal_surface_roots(app)?;
    if let Some(target_top_window) = target_top_window {
        let dispatch_app = app.clone();
        clipboard_surface_foreground::start(
            target_top_window,
            internal_surface_roots.clone(),
            move || {
                queue_external_surface_close(
                    dispatch_app,
                    ClipboardSurfaceCloseReason::ForegroundChanged,
                    ClipboardPreviewCloseReason::ForegroundChanged,
                    "target switch",
                    None,
                );
            },
        )?;
    }

    let dispatch_app = app.clone();
    if let Err(error) =
        clipboard_surface_pointer::start(internal_surface_roots, move |observation| {
            queue_external_surface_close(
                dispatch_app,
                ClipboardSurfaceCloseReason::PointerOutside,
                ClipboardPreviewCloseReason::PointerOutside,
                "outside pointer press",
                Some(observation),
            );
        })
    {
        let _ = clipboard_surface_foreground::stop();
        return Err(error.into());
    }
    Ok(())
}

#[cfg(windows)]
fn internal_surface_roots<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<Vec<usize>, ClipboardSurfaceWindowError> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetAncestor, GA_ROOT};

    let mut internal_surface_roots = Vec::with_capacity(2);
    for label in [CLIPBOARD_SURFACE_LABEL, CLIPBOARD_PREVIEW_SURFACE_LABEL] {
        let window = app
            .get_webview_window(label)
            .ok_or(ClipboardSurfaceWindowError::SurfaceGroupNotPrepared)?;
        let hwnd = window
            .hwnd()
            .map_err(|_| ClipboardSurfaceWindowError::NativeHandle)?;
        let root = unsafe { GetAncestor(hwnd.0, GA_ROOT) };
        if root.is_null() {
            return Err(ClipboardSurfaceWindowError::NativeHandle);
        }
        let root = root as usize;
        if !internal_surface_roots.contains(&root) {
            internal_surface_roots.push(root);
        }
    }
    Ok(internal_surface_roots)
}

#[cfg(windows)]
fn queue_external_surface_close<R: Runtime>(
    dispatch_app: AppHandle<R>,
    main_reason: ClipboardSurfaceCloseReason,
    preview_reason: ClipboardPreviewCloseReason,
    source: &'static str,
    pointer_observation: Option<clipboard_surface_pointer::PointerObservation>,
) {
    // Hook workers only queue work. The close runs on Tauri's main loop, so
    // teardown and join can never target the calling hook worker itself.
    let close_app = dispatch_app.clone();
    if let Err(error) = dispatch_app.run_on_main_thread(move || {
        if let Some(observation) = pointer_observation {
            debug_qa::trace(format!(
                "outside pointer close backend={} message={:#x} point=({}, {}) observed_root={:#x} pass_through=true",
                observation.backend,
                observation.message,
                observation.point_x,
                observation.point_y,
                observation.observed_root,
            ));
        }
        let Some(runtime) = close_app.try_state::<ApplicationRuntime>() else {
            return;
        };
        if is_visible(&close_app) {
            if let Err(error) = close(&close_app, runtime.surface(), main_reason) {
                eprintln!("failed to close clipboard surface after {source}: {error}");
            }
        } else if let Err(error) = close_preview(&close_app, preview_reason) {
            eprintln!("failed to close clipboard preview after {source}: {error}");
        }
    }) {
        eprintln!("failed to queue clipboard surface close after {source}: {error}");
    }
}

pub fn open_preview<R: Runtime>(
    app: &AppHandle<R>,
    record_id: String,
) -> Result<(), ClipboardSurfaceWindowError> {
    let selected_before = preview_selection()
        .lock()
        .map_err(|_| ClipboardSurfaceWindowError::PreviewStatePoisoned)?
        .record_id
        .clone();
    let main_visible = is_visible(app);
    debug_qa::trace(format!(
        "preview open request record_id={record_id} main_visible={main_visible} selected_before={selected_before:?}"
    ));
    let result = (|| {
        if !main_visible {
            return Err(ClipboardSurfaceWindowError::MainSurfaceNotVisible);
        }
        let main = app
            .get_webview_window(CLIPBOARD_SURFACE_LABEL)
            .ok_or(ClipboardSurfaceWindowError::MainSurfaceNotVisible)?;
        let preview = prepared_preview(app)?;
        debug_qa::trace("preview get result=prepared_existing builder_invoked=false");
        let visible_before = native_is_visible(&preview)?;
        let already_open = visible_before && selected_before.as_deref() == Some(record_id.as_str());
        debug_qa::trace(format!(
            "preview open state record_id={record_id} visible_before={visible_before} already_open={already_open}"
        ));
        if already_open {
            return Ok(());
        }
        let placement = configure_show_verify_preview(
            || {
                debug_qa::trace(format!(
                    "preview native configure stage=open record_id={record_id} requested"
                ));
                configure_native_popup(&preview)
            },
            || preview_placement_for_main(&main),
            |placement| {
                debug_qa::trace(format!(
                    "preview native show record_id={record_id} placement={placement:?} requested"
                ));
                surface_window_animation::prepare_show(&preview);
                show_native_no_activate(&preview, placement)
            },
            || native_is_visible(&preview),
        )?;
        debug_qa::trace(format!(
            "preview native show record_id={record_id} placement={placement:?} visible_after=true"
        ));
        refresh_native_shape_or_log(&preview);

        let change = {
            let mut selection = preview_selection()
                .lock()
                .map_err(|_| ClipboardSurfaceWindowError::PreviewStatePoisoned)?;
            let change = if selection.record_id.as_deref() == Some(record_id.as_str()) {
                "opened"
            } else if selection.record_id.is_some() {
                "selection_changed"
            } else {
                "opened"
            };
            selection.record_id = Some(record_id.clone());
            change
        };
        notify_preview_or_log(
            app,
            ClipboardPreviewStateEvent {
                change,
                record_id: Some(record_id.clone()),
                visible: true,
            },
        );
        Ok(())
    })();
    match &result {
        Ok(()) => debug_qa::trace(format!("preview open success record_id={record_id}")),
        Err(error) => debug_qa::trace(format!(
            "preview open failure record_id={record_id} error={error}"
        )),
    }
    result
}

pub fn close_preview<R: Runtime>(
    app: &AppHandle<R>,
    reason: ClipboardPreviewCloseReason,
) -> Result<(), ClipboardSurfaceWindowError> {
    let window = app.get_webview_window(CLIPBOARD_PREVIEW_SURFACE_LABEL);
    let visible_before = match &window {
        Some(window) => native_is_visible(window)?,
        None => false,
    };
    let selected_before = preview_selection()
        .lock()
        .map_err(|_| ClipboardSurfaceWindowError::PreviewStatePoisoned)?
        .record_id
        .clone();
    debug_qa::trace(format!(
        "preview close request reason={} visible_before={visible_before} selected_before={selected_before:?}",
        reason.as_str()
    ));
    let lifecycle = close_preview_lifecycle(
        || match &window {
            Some(window) => hide_native_verified(window),
            None => Ok(()),
        },
        || {
            Ok(preview_selection()
                .lock()
                .map_err(|_| ClipboardSurfaceWindowError::PreviewStatePoisoned)?
                .record_id
                .take()
                .is_some())
        },
    );
    let had_selection = match lifecycle {
        Ok(had_selection) => had_selection,
        Err(error) => {
            debug_qa::trace(format!(
                "preview close failure reason={} error={error}",
                reason.as_str()
            ));
            return Err(error);
        }
    };
    if had_selection {
        notify_preview_or_log(
            app,
            ClipboardPreviewStateEvent {
                change: "closed",
                record_id: None,
                visible: false,
            },
        );
    }
    debug_qa::trace(format!(
        "preview close success reason={} had_selection={had_selection}",
        reason.as_str()
    ));
    Ok(())
}

pub fn forget_preview_state() -> Result<(), ClipboardSurfaceWindowError> {
    preview_selection()
        .lock()
        .map_err(|_| ClipboardSurfaceWindowError::PreviewStatePoisoned)?
        .record_id = None;
    Ok(())
}

pub fn preview_state<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<ClipboardPreviewSurfaceState, ClipboardSurfaceWindowError> {
    let visible = match app.get_webview_window(CLIPBOARD_PREVIEW_SURFACE_LABEL) {
        Some(window) => native_is_visible(&window)?,
        None => false,
    };
    let mut selection = preview_selection()
        .lock()
        .map_err(|_| ClipboardSurfaceWindowError::PreviewStatePoisoned)?;
    if !visible {
        selection.record_id = None;
    }
    let state = ClipboardPreviewSurfaceState {
        record_id: selection.record_id.clone(),
        visible,
    };
    debug_qa::trace(format!(
        "preview state query record_id={:?} visible={}",
        state.record_id, state.visible
    ));
    Ok(state)
}

fn notify_preview_or_log<R: Runtime>(app: &AppHandle<R>, payload: ClipboardPreviewStateEvent) {
    if app
        .get_webview_window(CLIPBOARD_PREVIEW_SURFACE_LABEL)
        .is_none()
    {
        return;
    }
    if let Err(error) = app.emit_to(
        CLIPBOARD_PREVIEW_SURFACE_LABEL,
        CLIPBOARD_PREVIEW_STATE_EVENT,
        payload,
    ) {
        eprintln!("failed to emit clipboard preview state: {error}");
    }
}

fn notify_state_or_log<R: Runtime>(app: &AppHandle<R>, change: &'static str) {
    if app.get_webview_window(CLIPBOARD_SURFACE_LABEL).is_none() {
        return;
    }
    if let Err(error) = app.emit_to(
        CLIPBOARD_SURFACE_LABEL,
        CLIPBOARD_SURFACE_STATE_EVENT,
        ClipboardSurfaceStateEvent { change },
    ) {
        eprintln!("failed to emit clipboard surface {change} state: {error}");
    }
}

#[cfg(test)]
fn hide_and_verify<A: NativeVisibilityApi>(
    api: &mut A,
    window: usize,
) -> Result<(), ClipboardSurfaceWindowError> {
    api.hide(window);
    if api.is_visible(window) {
        Err(ClipboardSurfaceWindowError::StillVisible)
    } else {
        Ok(())
    }
}

fn hide_then_clear<H, C>(hide: H, clear: C) -> Result<(), ClipboardSurfaceWindowError>
where
    H: FnOnce() -> Result<(), ClipboardSurfaceWindowError>,
    C: FnOnce() -> Result<(), ClipboardSurfaceWindowError>,
{
    hide()?;
    clear()
}

fn configure_show_verify_preview<C, P, S, V>(
    configure: C,
    placement: P,
    show: S,
    visible_after: V,
) -> Result<SurfacePlacement, ClipboardSurfaceWindowError>
where
    C: FnOnce() -> Result<(), ClipboardSurfaceWindowError>,
    P: FnOnce() -> Result<SurfacePlacement, ClipboardSurfaceWindowError>,
    S: FnOnce(SurfacePlacement) -> Result<(), ClipboardSurfaceWindowError>,
    V: FnOnce() -> Result<bool, ClipboardSurfaceWindowError>,
{
    configure()?;
    let placement = placement()?;
    show(placement)?;
    if !visible_after()? {
        return Err(ClipboardSurfaceWindowError::PreviewStillHidden);
    }
    Ok(placement)
}

fn close_preview_lifecycle<H, C>(
    hide: H,
    clear_selection: C,
) -> Result<bool, ClipboardSurfaceWindowError>
where
    H: FnOnce() -> Result<(), ClipboardSurfaceWindowError>,
    C: FnOnce() -> Result<bool, ClipboardSurfaceWindowError>,
{
    hide()?;
    clear_selection()
}

#[cfg(windows)]
struct SystemNativeVisibilityApi;

#[cfg(windows)]
impl NativeVisibilityApi for SystemNativeVisibilityApi {
    #[cfg(test)]
    fn hide(&mut self, window: usize) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
        unsafe {
            ShowWindow(window as _, SW_HIDE);
        }
    }

    fn is_visible(&mut self, window: usize) -> bool {
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible(window as _) != 0 }
    }
}

#[cfg(windows)]
fn hide_native_verified<R: Runtime>(
    window: &WebviewWindow<R>,
) -> Result<(), ClipboardSurfaceWindowError> {
    let animated = surface_window_animation::fade_hide_native(window);
    debug_qa::trace(format!(
        "clipboard surface fade-hide result={} duration_ms={}",
        if animated {
            "transition_started"
        } else {
            "failed"
        },
        surface_window_animation::exit_duration_ms(window)
    ));
    animated
        .then_some(())
        .ok_or(ClipboardSurfaceWindowError::StillVisible)
}

#[cfg(not(windows))]
fn hide_native_verified<R: Runtime>(
    window: &WebviewWindow<R>,
) -> Result<(), ClipboardSurfaceWindowError> {
    window.hide()?;
    if window.is_visible()? {
        Err(ClipboardSurfaceWindowError::StillVisible)
    } else {
        Ok(())
    }
}

fn monitor_geometry(monitor: &Monitor) -> Option<MonitorGeometry> {
    let bounds = rect_from_origin_size(
        monitor.position().x,
        monitor.position().y,
        monitor.size().width,
        monitor.size().height,
    )?;
    let work = monitor.work_area();
    let work_area = rect_from_origin_size(
        work.position.x,
        work.position.y,
        work.size.width,
        work.size.height,
    )?;
    PixelRect::is_valid(bounds)
        .then_some(())
        .and_then(|_| PixelRect::is_valid(work_area).then_some(()))?;
    let scale_factor = monitor.scale_factor();
    (scale_factor.is_finite() && scale_factor > 0.0).then_some(MonitorGeometry {
        bounds,
        work_area,
        scale_factor,
    })
}

fn rect_from_origin_size(x: i32, y: i32, width: u32, height: u32) -> Option<PixelRect> {
    let right = i64::from(x).checked_add(i64::from(width))?;
    let bottom = i64::from(y).checked_add(i64::from(height))?;
    Some(PixelRect {
        left: x,
        top: y,
        right: i32::try_from(right).ok()?,
        bottom: i32::try_from(bottom).ok()?,
    })
}

impl PixelRect {
    fn is_valid(self) -> bool {
        self.right > self.left && self.bottom > self.top
    }

    fn contains(self, point: PixelPoint) -> bool {
        point.x >= self.left && point.x < self.right && point.y >= self.top && point.y < self.bottom
    }

    fn width(self) -> u32 {
        u32::try_from(i64::from(self.right) - i64::from(self.left)).unwrap_or(0)
    }

    fn height(self) -> u32 {
        u32::try_from(i64::from(self.bottom) - i64::from(self.top)).unwrap_or(0)
    }
}

fn select_anchor_monitor(
    anchor: PixelPoint,
    monitors: &[MonitorGeometry],
) -> Option<MonitorGeometry> {
    monitors
        .iter()
        .copied()
        .find(|monitor| monitor.bounds.contains(anchor))
        .or_else(|| {
            monitors
                .iter()
                .copied()
                .min_by_key(|monitor| squared_distance_to_rect(anchor, monitor.bounds))
        })
}

fn squared_distance_to_rect(point: PixelPoint, rect: PixelRect) -> i128 {
    let x = axis_distance(point.x, rect.left, rect.right);
    let y = axis_distance(point.y, rect.top, rect.bottom);
    i128::from(x) * i128::from(x) + i128::from(y) * i128::from(y)
}

fn axis_distance(point: i32, start: i32, end: i32) -> i64 {
    if point < start {
        i64::from(start) - i64::from(point)
    } else if point >= end {
        i64::from(point) - i64::from(end.saturating_sub(1))
    } else {
        0
    }
}

fn resolve_surface_anchor<E>(
    valid_caret: Option<PixelPoint>,
    cursor_fallback: impl FnOnce() -> Result<PixelPoint, E>,
) -> Result<(PixelPoint, SurfaceAnchorSource), E> {
    if let Some(caret) = valid_caret {
        Ok((caret, SurfaceAnchorSource::Caret))
    } else {
        cursor_fallback().map(|cursor| (cursor, SurfaceAnchorSource::Cursor))
    }
}

fn convert_caret_client_to_physical_screen<T: Copy, M, E>(
    client_point: T,
    client_to_screen: impl FnOnce(T) -> Result<T, E>,
    logical_to_physical: impl FnOnce(T) -> Result<(T, M), E>,
) -> Result<(T, T, M), E> {
    let logical_screen = client_to_screen(client_point)?;
    let (physical_screen, mode) = logical_to_physical(logical_screen)?;
    Ok((logical_screen, physical_screen, mode))
}

fn surface_placement(cursor: PixelPoint, monitor: MonitorGeometry) -> Option<SurfacePlacement> {
    if !monitor.bounds.is_valid()
        || !monitor.work_area.is_valid()
        || !monitor.scale_factor.is_finite()
        || monitor.scale_factor <= 0.0
    {
        return None;
    }
    let requested_width = scaled_dimension(CLIPBOARD_SURFACE_WIDTH, monitor.scale_factor)?;
    let requested_height = scaled_dimension(CLIPBOARD_SURFACE_HEIGHT, monitor.scale_factor)?;
    let size = PixelSize {
        width: requested_width.min(monitor.work_area.width()),
        height: requested_height.min(monitor.work_area.height()),
    };
    if size.width == 0 || size.height == 0 {
        return None;
    }
    let gap = scaled_dimension(CLIPBOARD_SURFACE_CURSOR_GAP, monitor.scale_factor)?;
    let gap = i32::try_from(gap).ok()?;
    Some(SurfacePlacement {
        position: PixelPoint {
            x: place_axis(
                cursor.x,
                monitor.work_area.left,
                monitor.work_area.right,
                size.width,
                gap,
            )?,
            y: place_axis(
                cursor.y,
                monitor.work_area.top,
                monitor.work_area.bottom,
                size.height,
                gap,
            )?,
        },
        size,
    })
}

fn preview_surface_placement(
    anchor: PixelRect,
    monitor: MonitorGeometry,
) -> Option<SurfacePlacement> {
    if !anchor.is_valid()
        || !monitor.bounds.is_valid()
        || !monitor.work_area.is_valid()
        || !monitor.scale_factor.is_finite()
        || monitor.scale_factor <= 0.0
    {
        return None;
    }
    let requested_width = scaled_dimension(CLIPBOARD_PREVIEW_SURFACE_WIDTH, monitor.scale_factor)?;
    let requested_height =
        scaled_dimension(CLIPBOARD_PREVIEW_SURFACE_HEIGHT, monitor.scale_factor)?;
    // Preview and main are one visual surface group. Their outer rectangles
    // touch; the independent cursor/caret gap only applies to the main popup.
    let right_start = anchor.right;
    let left_end = anchor.left;
    let right_space = axis_space(right_start, monitor.work_area.right);
    let left_space = axis_space(monitor.work_area.left, left_end);
    let (on_right, available_width) = if right_space >= requested_width {
        (true, right_space)
    } else if left_space >= requested_width {
        (false, left_space)
    } else if right_space >= left_space {
        (true, right_space)
    } else {
        (false, left_space)
    };
    let width = requested_width.min(available_width);
    let height = requested_height.min(monitor.work_area.height());
    if width == 0 || height == 0 {
        return None;
    }
    let width_i32 = i32::try_from(width).ok()?;
    let x = if on_right {
        right_start
    } else {
        left_end.checked_sub(width_i32)?
    };
    let maximum_y = i64::from(monitor.work_area.bottom).checked_sub(i64::from(height))?;
    let y = i64::from(anchor.top).clamp(i64::from(monitor.work_area.top), maximum_y);
    Some(SurfacePlacement {
        position: PixelPoint {
            x,
            y: i32::try_from(y).ok()?,
        },
        size: PixelSize { width, height },
    })
}

fn axis_space(start: i32, end: i32) -> u32 {
    u32::try_from(i64::from(end) - i64::from(start)).unwrap_or(0)
}

fn scaled_dimension(logical: f64, scale_factor: f64) -> Option<u32> {
    let physical = logical * scale_factor;
    if !physical.is_finite() || physical <= 0.0 || physical > f64::from(u32::MAX) {
        return None;
    }
    u32::try_from(physical.round() as u64).ok()
}

fn place_axis(cursor: i32, start: i32, end: i32, length: u32, gap: i32) -> Option<i32> {
    let start = i64::from(start);
    let end = i64::from(end);
    let cursor = i64::from(cursor);
    let length = i64::from(length);
    let gap = i64::from(gap);
    if end <= start || length <= 0 || length > end - start || gap < 0 {
        return None;
    }
    let after = cursor.checked_add(gap)?;
    if after >= start && after.checked_add(length)? <= end {
        return i32::try_from(after).ok();
    }
    let before = cursor.checked_sub(gap)?.checked_sub(length)?;
    if before >= start && before.checked_add(length)? <= end {
        return i32::try_from(before).ok();
    }
    i32::try_from(after.clamp(start, end - length)).ok()
}

#[cfg(windows)]
fn placement_for_current_anchor<R: Runtime>(
    window: &WebviewWindow<R>,
    captured_target_top_window: Option<usize>,
) -> Result<SurfacePlacement, ClipboardSurfaceWindowError> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let caret = captured_target_top_window.and_then(caret_anchor_for_target);
    let (anchor, source) = resolve_surface_anchor(caret, || {
        let mut cursor: POINT = unsafe { std::mem::zeroed() };
        if unsafe { GetCursorPos(&mut cursor) } == 0 {
            return Err(ClipboardSurfaceWindowError::CursorPosition);
        }
        Ok(PixelPoint {
            x: cursor.x,
            y: cursor.y,
        })
    })?;
    let monitors = window
        .available_monitors()?
        .iter()
        .filter_map(monitor_geometry)
        .collect::<Vec<_>>();
    let monitor = select_anchor_monitor(anchor, &monitors)
        .ok_or(ClipboardSurfaceWindowError::InvalidDimensions)?;
    let placement =
        surface_placement(anchor, monitor).ok_or(ClipboardSurfaceWindowError::InvalidDimensions)?;
    debug_qa::trace(format!(
        "surface placement source={} anchor={anchor:?} work_area={:?} monitor_bounds={:?} scale_factor={} placement={placement:?}",
        source.as_str(),
        monitor.work_area, monitor.bounds, monitor.scale_factor
    ));
    Ok(placement)
}

#[cfg(windows)]
fn caret_anchor_for_target(target_root: usize) -> Option<PixelPoint> {
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetAncestor, GetForegroundWindow, GetGUIThreadInfo, GetWindowRect,
        GetWindowThreadProcessId, IsWindow, GA_ROOT, GUITHREADINFO,
    };

    let target_root = target_root as HWND;
    if target_root.is_null() || unsafe { IsWindow(target_root) } == 0 {
        return None;
    }
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_null() || unsafe { GetAncestor(foreground, GA_ROOT) } != target_root {
        return None;
    }
    let foreground_thread = unsafe { GetWindowThreadProcessId(foreground, null_mut()) };
    if foreground_thread == 0 {
        return None;
    }
    let mut info: GUITHREADINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
    if unsafe { GetGUIThreadInfo(foreground_thread, &mut info) } == 0
        || info.hwndCaret.is_null()
        || unsafe { IsWindow(info.hwndCaret) } == 0
        || unsafe { GetAncestor(info.hwndCaret, GA_ROOT) } != target_root
        || (!info.hwndFocus.is_null()
            && unsafe { GetAncestor(info.hwndFocus, GA_ROOT) } != target_root)
        || !valid_caret_client_rect(info.rcCaret)
    {
        return None;
    }

    // GetGUIThreadInfo returns caret coordinates in the target window's own
    // logical client space, not virtualized to this thread. First translate to
    // logical screen coordinates, then apply the caret HWND's per-monitor DPI
    // transform. Tauri monitor geometry is expressed in physical pixels.
    let client_point = POINT {
        x: info.rcCaret.right,
        y: info.rcCaret.bottom,
    };
    let (logical_screen, point, dpi_converted) = convert_caret_client_to_physical_screen(
        client_point,
        |mut point| {
            (unsafe { ClientToScreen(info.hwndCaret, &mut point) } != 0)
                .then_some(point)
                .ok_or(())
        },
        |point| optional_logical_screen_to_physical(info.hwndCaret, point),
    )
    .ok()?;
    let mut root_rect: RECT = unsafe { std::mem::zeroed() };
    if unsafe { GetWindowRect(target_root, &mut root_rect) } == 0
        || point.x < root_rect.left
        || point.x > root_rect.right
        || point.y < root_rect.top
        || point.y > root_rect.bottom
    {
        return None;
    }
    debug_qa::trace(format!(
        "surface caret anchor target_root={:#x} foreground_thread={} caret_hwnd={:#x} client_rect=({},{},{},{}) logical_screen=({},{}) physical_screen=({},{}) dpi_conversion={}",
        target_root as usize,
        foreground_thread,
        info.hwndCaret as usize,
        info.rcCaret.left,
        info.rcCaret.top,
        info.rcCaret.right,
        info.rcCaret.bottom,
        logical_screen.x,
        logical_screen.y,
        point.x,
        point.y,
        if dpi_converted { "per_monitor" } else { "win7_baseline" }
    ));
    Some(PixelPoint {
        x: point.x,
        y: point.y,
    })
}

#[cfg(windows)]
fn optional_logical_screen_to_physical(
    window: windows_sys::Win32::Foundation::HWND,
    mut point: windows_sys::Win32::Foundation::POINT,
) -> Result<(windows_sys::Win32::Foundation::POINT, bool), ()> {
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

    type LogicalToPhysicalPointForPerMonitorDpi = unsafe extern "system" fn(
        windows_sys::Win32::Foundation::HWND,
        *mut windows_sys::Win32::Foundation::POINT,
    ) -> i32;

    const USER32: [u16; 11] = [
        b'u' as u16,
        b's' as u16,
        b'e' as u16,
        b'r' as u16,
        b'3' as u16,
        b'2' as u16,
        b'.' as u16,
        b'd' as u16,
        b'l' as u16,
        b'l' as u16,
        0,
    ];
    const PROCEDURE: &[u8] = b"LogicalToPhysicalPointForPerMonitorDPI\0";

    let module = unsafe { GetModuleHandleW(USER32.as_ptr()) };
    if module.is_null() {
        return Ok((point, false));
    }
    let Some(procedure) = (unsafe { GetProcAddress(module, PROCEDURE.as_ptr()) }) else {
        // Windows 7 does not export this Windows 8.1 API. ClientToScreen is the
        // compatible baseline and, importantly, the executable has no static
        // import that would prevent it from loading on Windows 7.
        return Ok((point, false));
    };
    let convert = unsafe {
        std::mem::transmute::<
            unsafe extern "system" fn() -> isize,
            LogicalToPhysicalPointForPerMonitorDpi,
        >(procedure)
    };
    if unsafe { convert(window, &mut point) } == 0 {
        return Err(());
    }
    Ok((point, true))
}

#[cfg(windows)]
fn valid_caret_client_rect(rect: windows_sys::Win32::Foundation::RECT) -> bool {
    rect.right >= rect.left && rect.bottom > rect.top
}

#[cfg(windows)]
fn preview_placement_for_main<R: Runtime>(
    main: &WebviewWindow<R>,
) -> Result<SurfacePlacement, ClipboardSurfaceWindowError> {
    let position = main.outer_position()?;
    let size = main.outer_size()?;
    let anchor = rect_from_origin_size(position.x, position.y, size.width, size.height)
        .ok_or(ClipboardSurfaceWindowError::InvalidDimensions)?;
    let monitor = main
        .current_monitor()?
        .as_ref()
        .and_then(monitor_geometry)
        .ok_or(ClipboardSurfaceWindowError::InvalidDimensions)?;
    let placement = preview_surface_placement(anchor, monitor)
        .ok_or(ClipboardSurfaceWindowError::InvalidDimensions)?;
    debug_qa::trace(format!(
        "preview placement anchor={anchor:?} work_area={:?} monitor_bounds={:?} scale_factor={} placement={placement:?}",
        monitor.work_area, monitor.bounds, monitor.scale_factor
    ));
    Ok(placement)
}

#[cfg(not(windows))]
fn preview_placement_for_main<R: Runtime>(
    _main: &WebviewWindow<R>,
) -> Result<SurfacePlacement, ClipboardSurfaceWindowError> {
    Err(ClipboardSurfaceWindowError::UnsupportedPlatform)
}

#[cfg(not(windows))]
fn placement_for_current_anchor<R: Runtime>(
    _window: &WebviewWindow<R>,
    _captured_target_top_window: Option<usize>,
) -> Result<SurfacePlacement, ClipboardSurfaceWindowError> {
    Err(ClipboardSurfaceWindowError::UnsupportedPlatform)
}

#[cfg(windows)]
fn popup_window_style(style: u32) -> u32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
    };
    (style & !(WS_CAPTION | WS_MAXIMIZEBOX | WS_MINIMIZEBOX | WS_SYSMENU | WS_THICKFRAME))
        | WS_POPUP
}

#[cfg(windows)]
fn popup_extended_style(style: u32) -> u32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };
    (style & !WS_EX_APPWINDOW) | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW
}

#[cfg(windows)]
fn child_no_activate_extended_style(style: u32) -> u32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::WS_EX_NOACTIVATE;
    style | WS_EX_NOACTIVATE
}

#[cfg(windows)]
trait WindowLongApi {
    fn get(&mut self, window: usize, index: i32) -> (isize, u32);
    fn set(&mut self, window: usize, index: i32, value: isize) -> (isize, u32);
}

#[cfg(windows)]
struct SystemWindowLongApi;

#[cfg(windows)]
impl WindowLongApi for SystemWindowLongApi {
    fn get(&mut self, window: usize, index: i32) -> (isize, u32) {
        use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
        use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW;

        unsafe {
            SetLastError(0);
        }
        let value = unsafe { GetWindowLongPtrW(window as _, index) };
        (value, unsafe { GetLastError() })
    }

    fn set(&mut self, window: usize, index: i32, value: isize) -> (isize, u32) {
        use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
        use windows_sys::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW;

        unsafe {
            SetLastError(0);
        }
        let previous = unsafe { SetWindowLongPtrW(window as _, index, value) };
        (previous, unsafe { GetLastError() })
    }
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowLongUpdate {
    Unchanged,
    Updated,
}

#[cfg(windows)]
fn update_window_long<A: WindowLongApi>(
    api: &mut A,
    window: usize,
    index: i32,
    transform: fn(u32) -> u32,
    get_operation: &'static str,
    set_operation: &'static str,
) -> Result<WindowLongUpdate, ClipboardSurfaceWindowError> {
    let (current, get_error) = api.get(window, index);
    if current == 0 && get_error != 0 {
        return Err(ClipboardSurfaceWindowError::WindowsApiCode {
            operation: get_operation,
            code: get_error,
        });
    }
    let next = transform(current as u32);
    if next == current as u32 {
        return Ok(WindowLongUpdate::Unchanged);
    }
    let (previous, set_error) = api.set(window, index, (next as i32) as isize);
    // SetWindowLongPtrW legitimately returns the previous value, which may be
    // zero. Only a nonzero GetLastError after the call identifies failure.
    if previous == 0 && set_error != 0 {
        return Err(ClipboardSurfaceWindowError::WindowsApiCode {
            operation: set_operation,
            code: set_error,
        });
    }
    Ok(WindowLongUpdate::Updated)
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoActivateChildKind {
    WryWebview,
    ChromiumHost,
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoActivateChildPolicy {
    SkipDifferentIdentity,
    SubclassOnly,
    SubclassAndStyle(NoActivateChildKind),
}

#[cfg(windows)]
enum ChildStyleUpdate {
    Applied(WindowLongUpdate),
    Degraded(ClipboardSurfaceWindowError),
}

#[cfg(windows)]
fn update_child_no_activate_style<A: WindowLongApi>(
    api: &mut A,
    window: usize,
) -> ChildStyleUpdate {
    match update_window_long(
        api,
        window,
        windows_sys::Win32::UI::WindowsAndMessaging::GWL_EXSTYLE,
        child_no_activate_extended_style,
        "GetWindowLongPtrW(child GWL_EXSTYLE)",
        "SetWindowLongPtrW(child GWL_EXSTYLE)",
    ) {
        Ok(update) => ChildStyleUpdate::Applied(update),
        Err(error) => ChildStyleUpdate::Degraded(error),
    }
}

#[cfg(windows)]
fn no_activate_child_policy(
    root_process_id: u32,
    root_thread_id: u32,
    process_id: u32,
    thread_id: u32,
    class_name: &str,
) -> NoActivateChildPolicy {
    if process_id != root_process_id || thread_id != root_thread_id {
        return NoActivateChildPolicy::SkipDifferentIdentity;
    }
    match class_name {
        "WRY_WEBVIEW" => NoActivateChildPolicy::SubclassAndStyle(NoActivateChildKind::WryWebview),
        "Chrome_WidgetWin_0" => {
            NoActivateChildPolicy::SubclassAndStyle(NoActivateChildKind::ChromiumHost)
        }
        _ => NoActivateChildPolicy::SubclassOnly,
    }
}

#[cfg(windows)]
fn require_top_level_subclass(installed: bool) -> Result<(), ClipboardSurfaceWindowError> {
    if installed {
        Ok(())
    } else {
        Err(ClipboardSurfaceWindowError::WindowsApi(
            "SetWindowSubclass(top-level no activate)",
        ))
    }
}

#[cfg(windows)]
fn configure_native_popup<R: Runtime>(
    window: &WebviewWindow<R>,
) -> Result<(), ClipboardSurfaceWindowError> {
    use windows_sys::Win32::UI::Shell::SetWindowSubclass;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GWL_STYLE};

    let hwnd = window
        .hwnd()
        .map_err(|_| ClipboardSurfaceWindowError::NativeHandle)?;
    for (index, transform, operation) in [
        (
            GWL_STYLE,
            popup_window_style as fn(u32) -> u32,
            "SetWindowLongPtrW(GWL_STYLE)",
        ),
        (
            GWL_EXSTYLE,
            popup_extended_style as fn(u32) -> u32,
            "SetWindowLongPtrW(GWL_EXSTYLE)",
        ),
    ] {
        update_window_long(
            &mut SystemWindowLongApi,
            hwnd.0 as usize,
            index,
            transform,
            "GetWindowLongPtrW(top-level)",
            operation,
        )?;
    }
    require_top_level_subclass(unsafe {
        SetWindowSubclass(
            hwnd.0,
            Some(no_activate_subclass_proc),
            NO_ACTIVATE_SUBCLASS_ID,
            0,
        ) != 0
    })?;
    configure_no_activate_children(hwnd.0)?;
    configure_dwm_non_client(hwnd.0);
    Ok(())
}

#[cfg(windows)]
fn configure_no_activate_children(
    root: windows_sys::Win32::Foundation::HWND,
) -> Result<(), ClipboardSurfaceWindowError> {
    use windows_sys::Win32::UI::Shell::SetWindowSubclass;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, GetWindowThreadProcessId, SetWindowPos, SWP_FRAMECHANGED, SWP_NOACTIVATE,
        SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    };

    let mut windows = Vec::new();
    let mut root_process_id = 0;
    let root_thread_id = unsafe { GetWindowThreadProcessId(root, &mut root_process_id) };
    if root_thread_id == 0 || root_process_id == 0 {
        return Err(ClipboardSurfaceWindowError::WindowsApi(
            "GetWindowThreadProcessId(root)",
        ));
    }
    unsafe {
        EnumChildWindows(
            root,
            Some(collect_child_window),
            (&mut windows as *mut Vec<windows_sys::Win32::Foundation::HWND>) as isize,
        );
    }
    for window in windows {
        let mut process_id = 0;
        let thread_id = unsafe { GetWindowThreadProcessId(window, &mut process_id) };
        let class_name =
            native_window_class_name(window).unwrap_or_else(|| "<class-unavailable>".to_string());
        let policy = no_activate_child_policy(
            root_process_id,
            root_thread_id,
            process_id,
            thread_id,
            &class_name,
        );
        if policy == NoActivateChildPolicy::SkipDifferentIdentity {
            debug_qa::trace(format!(
                "child noactivate skip hwnd={:#x} class={class_name} pid={process_id} thread={thread_id} reason=different_process_or_ui_thread",
                window as usize
            ));
            continue;
        }

        let subclassed = unsafe {
            SetWindowSubclass(
                window,
                Some(no_activate_subclass_proc),
                NO_ACTIVATE_SUBCLASS_ID,
                0,
            )
        } != 0;
        if !subclassed {
            debug_qa::trace(format!(
                "child noactivate degraded hwnd={:#x} class={class_name} policy={policy:?} reason=subclass_failed",
                window as usize
            ));
        } else {
            debug_qa::trace(format!(
                "child noactivate subclass hwnd={:#x} class={class_name} policy={policy:?} result=installed",
                window as usize
            ));
        }

        let NoActivateChildPolicy::SubclassAndStyle(kind) = policy else {
            debug_qa::trace(format!(
                "child noactivate style hwnd={:#x} class={class_name} policy={policy:?} result=skipped_safe_class_filter",
                window as usize
            ));
            continue;
        };
        match update_child_no_activate_style(&mut SystemWindowLongApi, window as usize) {
            ChildStyleUpdate::Applied(update) => debug_qa::trace(format!(
                "child noactivate style hwnd={:#x} class={class_name} kind={kind:?} result={update:?}",
                window as usize
            )),
            ChildStyleUpdate::Degraded(error) => debug_qa::trace(format!(
                "child noactivate degraded hwnd={:#x} class={class_name} kind={kind:?} reason=style_failed error={error}",
                window as usize
            )),
        }

        if unsafe {
            SetWindowPos(
                window,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
            )
        } == 0
        {
            debug_qa::trace(format!(
                "child noactivate degraded hwnd={:#x} class={class_name} kind={kind:?} reason=frame_refresh_failed",
                window as usize
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn native_window_class_name(window: windows_sys::Win32::Foundation::HWND) -> Option<String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClassNameW;

    let mut buffer = [0_u16; 128];
    let length = unsafe { GetClassNameW(window, buffer.as_mut_ptr(), buffer.len() as i32) };
    (length > 0).then(|| String::from_utf16_lossy(&buffer[..length as usize]))
}

#[cfg(windows)]
const NO_ACTIVATE_SUBCLASS_ID: usize = 0x4f44544e;

#[cfg(windows)]
unsafe extern "system" fn collect_child_window(
    window: windows_sys::Win32::Foundation::HWND,
    context: isize,
) -> i32 {
    let windows = &mut *(context as *mut Vec<windows_sys::Win32::Foundation::HWND>);
    windows.push(window);
    1
}

#[cfg(windows)]
unsafe extern "system" fn no_activate_subclass_proc(
    window: windows_sys::Win32::Foundation::HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
    subclass_id: usize,
    _reference_data: usize,
) -> isize {
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MA_NOACTIVATE, WM_MOUSEACTIVATE, WM_NCDESTROY,
    };

    if message == WM_MOUSEACTIVATE {
        return MA_NOACTIVATE as isize;
    }
    let result = DefSubclassProc(window, message, wparam, lparam);
    if message == WM_NCDESTROY {
        RemoveWindowSubclass(window, Some(no_activate_subclass_proc), subclass_id);
    }
    result
}

#[cfg(windows)]
fn configure_dwm_non_client(window: windows_sys::Win32::Foundation::HWND) {
    use windows_sys::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_NCRENDERING_POLICY,
    };

    let policy = DWMNCRP_DISABLED;
    unsafe {
        let _ = DwmSetWindowAttribute(
            window,
            DWMWA_NCRENDERING_POLICY as u32,
            (&policy as *const i32).cast(),
            std::mem::size_of_val(&policy) as u32,
        );
    }
    // Supported on Windows 11. Older systems return E_INVALIDARG and retain the
    // already-disabled non-client rendering policy above.
    const DWMWA_COLOR_NONE: u32 = 0xffff_fffe;
    unsafe {
        let _ = DwmSetWindowAttribute(
            window,
            DWMWA_BORDER_COLOR as u32,
            (&DWMWA_COLOR_NONE as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        );
    }
}

#[cfg(not(windows))]
fn configure_native_popup<R: Runtime>(
    _window: &WebviewWindow<R>,
) -> Result<(), ClipboardSurfaceWindowError> {
    Ok(())
}

#[cfg(windows)]
fn show_native_no_activate<R: Runtime>(
    window: &WebviewWindow<R>,
    placement: SurfacePlacement,
) -> Result<(), ClipboardSurfaceWindowError> {
    use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOOWNERZORDER,
        SWP_SHOWWINDOW,
    };

    let hwnd = window
        .hwnd()
        .map_err(|_| ClipboardSurfaceWindowError::NativeHandle)?;
    let width = i32::try_from(placement.size.width)
        .map_err(|_| ClipboardSurfaceWindowError::InvalidDimensions)?;
    let height = i32::try_from(placement.size.height)
        .map_err(|_| ClipboardSurfaceWindowError::InvalidDimensions)?;
    let flags = SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_SHOWWINDOW | SWP_FRAMECHANGED;
    unsafe {
        SetLastError(0);
    }
    if unsafe {
        SetWindowPos(
            hwnd.0,
            HWND_TOPMOST,
            placement.position.x,
            placement.position.y,
            width,
            height,
            flags,
        )
    } == 0
    {
        return Err(ClipboardSurfaceWindowError::WindowsApiCode {
            operation: "SetWindowPos(show no activate)",
            code: unsafe { GetLastError() },
        });
    }
    Ok(())
}

#[cfg(not(windows))]
fn show_native_no_activate<R: Runtime>(
    _window: &WebviewWindow<R>,
    _placement: SurfacePlacement,
) -> Result<(), ClipboardSurfaceWindowError> {
    Err(ClipboardSurfaceWindowError::UnsupportedPlatform)
}

pub fn refresh_native_shape_or_log<R: Runtime>(window: &WebviewWindow<R>) {
    if let Err(error) = apply_native_rounded_region(window) {
        eprintln!(
            "clipboard surface native rounding unavailable; using rectangular fallback: {error}"
        );
    }
}

#[cfg(windows)]
fn apply_native_rounded_region<R: Runtime>(
    window: &WebviewWindow<R>,
) -> Result<(), ClipboardSurfaceWindowError> {
    use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
    use windows_sys::Win32::Graphics::Gdi::{
        CreateRoundRectRgn, DeleteObject, GetRgnBox, GetWindowRgnBox, PtInRegion, SetWindowRgn,
        ERROR,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, GetWindowRect};

    let hwnd = window
        .hwnd()
        .map_err(|_| ClipboardSurfaceWindowError::NativeHandle)?;
    let mut window_rect = empty_windows_rect();
    unsafe {
        SetLastError(0);
    }
    if unsafe { GetWindowRect(hwnd.0, &mut window_rect) } == 0 {
        return Err(ClipboardSurfaceWindowError::WindowsApiCode {
            operation: "GetWindowRect(surface region)",
            code: unsafe { GetLastError() },
        });
    }
    let mut client_rect = empty_windows_rect();
    unsafe {
        SetLastError(0);
    }
    if unsafe { GetClientRect(hwnd.0, &mut client_rect) } == 0 {
        return Err(ClipboardSurfaceWindowError::WindowsApiCode {
            operation: "GetClientRect(surface region)",
            code: unsafe { GetLastError() },
        });
    }
    let (width, height) = region_dimensions_from_client_rect(client_rect)
        .ok_or(ClipboardSurfaceWindowError::InvalidDimensions)?;
    let tauri_outer = window.outer_size().ok();
    let scale_factor = window
        .scale_factor()
        .map_err(|_| ClipboardSurfaceWindowError::InvalidDimensions)?;
    let diameter = rounded_region_diameter(width as u32, height as u32, scale_factor)
        .ok_or(ClipboardSurfaceWindowError::InvalidDimensions)?;
    // CreateRoundRectRgn treats the lower-right coordinates as exclusive and
    // GetRgnBox reports one less than the supplied right/bottom. Requesting
    // client + 1 keeps the final client pixel inside the owned window region;
    // pixels beyond the HWND are still clipped by the window bounds.
    let (region_right, region_bottom) = region_request_bounds(width, height)
        .ok_or(ClipboardSurfaceWindowError::InvalidDimensions)?;
    let region =
        unsafe { CreateRoundRectRgn(0, 0, region_right, region_bottom, diameter, diameter) };
    if region.is_null() {
        return Err(ClipboardSurfaceWindowError::CreateRegion);
    }
    let mut requested_box = empty_windows_rect();
    let region_type = unsafe { GetRgnBox(region, &mut requested_box) };
    let edge_contains = [
        unsafe { PtInRegion(region, 0, height / 2) } != 0,
        unsafe { PtInRegion(region, width / 2, 0) } != 0,
        unsafe { PtInRegion(region, width - 1, height / 2) } != 0,
        unsafe { PtInRegion(region, width / 2, height - 1) } != 0,
    ];
    let corner_contains = [
        unsafe { PtInRegion(region, 0, 0) } != 0,
        unsafe { PtInRegion(region, width - 1, 0) } != 0,
        unsafe { PtInRegion(region, 0, height - 1) } != 0,
        unsafe { PtInRegion(region, width - 1, height - 1) } != 0,
    ];
    debug_qa::trace(format!(
        "surface native region window_rect=({},{},{},{}) client_rect=({},{},{},{}) tauri_outer={tauri_outer:?} region_request=(0,0,{region_right},{region_bottom}) region_box_before=({},{},{},{}) edge_contains={edge_contains:?} corner_contains={corner_contains:?}",
        window_rect.left,
        window_rect.top,
        window_rect.right,
        window_rect.bottom,
        client_rect.left,
        client_rect.top,
        client_rect.right,
        client_rect.bottom,
        requested_box.left,
        requested_box.top,
        requested_box.right,
        requested_box.bottom,
    ));
    if region_type == ERROR
        || requested_box.left != 0
        || requested_box.top != 0
        || requested_box.right != width
        || requested_box.bottom != height
        || edge_contains.iter().any(|contains| !contains)
        || corner_contains.iter().any(|contains| *contains)
    {
        unsafe {
            DeleteObject(region);
        }
        return Err(ClipboardSurfaceWindowError::InvalidRegionGeometry);
    }
    if unsafe { SetWindowRgn(hwnd.0, region, 1) } == 0 {
        // SetWindowRgn owns the region only after success.
        unsafe {
            DeleteObject(region);
        }
        return Err(ClipboardSurfaceWindowError::ApplyRegion);
    }
    let mut applied_box = empty_windows_rect();
    let applied_type = unsafe { GetWindowRgnBox(hwnd.0, &mut applied_box) };
    debug_qa::trace(format!(
        "surface native region applied_box=({},{},{},{}) applied_type={applied_type}",
        applied_box.left, applied_box.top, applied_box.right, applied_box.bottom
    ));
    if applied_type == ERROR
        || applied_box.left != 0
        || applied_box.top != 0
        || applied_box.right != width
        || applied_box.bottom != height
    {
        return Err(ClipboardSurfaceWindowError::InvalidRegionGeometry);
    }
    Ok(())
}

#[cfg(not(windows))]
fn apply_native_rounded_region<R: Runtime>(
    _window: &WebviewWindow<R>,
) -> Result<(), ClipboardSurfaceWindowError> {
    Err(ClipboardSurfaceWindowError::UnsupportedPlatform)
}

fn rounded_region_diameter(width: u32, height: u32, scale_factor: f64) -> Option<i32> {
    if width == 0 || height == 0 || !scale_factor.is_finite() || scale_factor <= 0.0 {
        return None;
    }
    let maximum = f64::from(width.min(height));
    let diameter = (CLIPBOARD_SURFACE_CORNER_RADIUS * 2.0 * scale_factor)
        .round()
        .clamp(2.0, maximum);
    i32::try_from(diameter as i64).ok()
}

#[cfg(windows)]
fn region_dimensions_from_client_rect(
    rect: windows_sys::Win32::Foundation::RECT,
) -> Option<(i32, i32)> {
    let width = rect.right.checked_sub(rect.left)?;
    let height = rect.bottom.checked_sub(rect.top)?;
    (width > 0 && height > 0).then_some((width, height))
}

#[cfg(windows)]
fn region_request_bounds(width: i32, height: i32) -> Option<(i32, i32)> {
    Some((width.checked_add(1)?, height.checked_add(1)?))
}

#[cfg(windows)]
const fn empty_windows_rect() -> windows_sys::Win32::Foundation::RECT {
    windows_sys::Win32::Foundation::RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    }
}

pub fn is_visible<R: Runtime>(app: &AppHandle<R>) -> bool {
    let Some(window) = app.get_webview_window(CLIPBOARD_SURFACE_LABEL) else {
        return false;
    };
    match native_is_visible(&window) {
        Ok(visible) => visible,
        Err(error) => {
            eprintln!("failed to read clipboard surface native visibility: {error}");
            false
        }
    }
}

#[cfg(windows)]
fn native_is_visible<R: Runtime>(
    window: &WebviewWindow<R>,
) -> Result<bool, ClipboardSurfaceWindowError> {
    let hwnd = window
        .hwnd()
        .map_err(|_| ClipboardSurfaceWindowError::NativeHandle)?;
    let mut api = SystemNativeVisibilityApi;
    Ok(api.is_visible(hwnd.0 as usize))
}

#[cfg(not(windows))]
fn native_is_visible<R: Runtime>(
    window: &WebviewWindow<R>,
) -> Result<bool, ClipboardSurfaceWindowError> {
    Ok(window.is_visible()?)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;

    #[cfg(windows)]
    struct FakeWindowLongApi {
        get_value: isize,
        get_error: u32,
        set_previous: isize,
        set_error: u32,
        set_calls: usize,
    }

    #[cfg(windows)]
    impl WindowLongApi for FakeWindowLongApi {
        fn get(&mut self, _window: usize, _index: i32) -> (isize, u32) {
            (self.get_value, self.get_error)
        }

        fn set(&mut self, _window: usize, _index: i32, _value: isize) -> (isize, u32) {
            self.set_calls += 1;
            (self.set_previous, self.set_error)
        }
    }

    #[cfg(windows)]
    fn fake_window_long_api(
        get_value: isize,
        get_error: u32,
        set_previous: isize,
        set_error: u32,
    ) -> FakeWindowLongApi {
        FakeWindowLongApi {
            get_value,
            get_error,
            set_previous,
            set_error,
            set_calls: 0,
        }
    }

    #[cfg(windows)]
    #[test]
    fn set_window_long_accepts_zero_previous_value_when_last_error_is_clear() {
        use windows_sys::Win32::UI::WindowsAndMessaging::GWL_EXSTYLE;

        let mut api = fake_window_long_api(0, 0, 0, 0);
        let update = update_window_long(
            &mut api,
            7,
            GWL_EXSTYLE,
            child_no_activate_extended_style,
            "get",
            "set",
        )
        .unwrap();

        assert_eq!(update, WindowLongUpdate::Updated);
        assert_eq!(api.set_calls, 1);
    }

    #[cfg(windows)]
    #[test]
    fn access_denied_and_invalid_child_style_updates_degrade_without_becoming_fatal() {
        const ERROR_ACCESS_DENIED: u32 = 5;
        const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;

        let mut access_denied = fake_window_long_api(0, 0, 0, ERROR_ACCESS_DENIED);
        assert!(matches!(
            update_child_no_activate_style(&mut access_denied, 11),
            ChildStyleUpdate::Degraded(ClipboardSurfaceWindowError::WindowsApiCode {
                operation: "SetWindowLongPtrW(child GWL_EXSTYLE)",
                code: ERROR_ACCESS_DENIED,
            })
        ));

        let mut invalid_child = fake_window_long_api(0, ERROR_INVALID_WINDOW_HANDLE, 0, 0);
        assert!(matches!(
            update_child_no_activate_style(&mut invalid_child, 13),
            ChildStyleUpdate::Degraded(ClipboardSurfaceWindowError::WindowsApiCode {
                operation: "GetWindowLongPtrW(child GWL_EXSTYLE)",
                code: ERROR_INVALID_WINDOW_HANDLE,
            })
        ));
    }

    #[cfg(windows)]
    #[test]
    fn top_level_window_contract_failures_remain_fatal() {
        const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;
        use windows_sys::Win32::UI::WindowsAndMessaging::GWL_EXSTYLE;

        let mut api = fake_window_long_api(0, ERROR_INVALID_WINDOW_HANDLE, 0, 0);
        assert!(matches!(
            update_window_long(
                &mut api,
                17,
                GWL_EXSTYLE,
                popup_extended_style,
                "GetWindowLongPtrW(top-level)",
                "SetWindowLongPtrW(top-level)",
            ),
            Err(ClipboardSurfaceWindowError::WindowsApiCode {
                operation: "GetWindowLongPtrW(top-level)",
                code: ERROR_INVALID_WINDOW_HANDLE,
            })
        ));
        assert!(matches!(
            require_top_level_subclass(false),
            Err(ClipboardSurfaceWindowError::WindowsApi(
                "SetWindowSubclass(top-level no activate)"
            ))
        ));
    }

    #[cfg(windows)]
    #[test]
    fn all_same_process_same_ui_thread_children_are_subclassed_with_safe_style_filtering() {
        assert_eq!(
            no_activate_child_policy(10, 20, 10, 20, "WRY_WEBVIEW"),
            NoActivateChildPolicy::SubclassAndStyle(NoActivateChildKind::WryWebview)
        );
        assert_eq!(
            no_activate_child_policy(10, 20, 10, 20, "Chrome_WidgetWin_0"),
            NoActivateChildPolicy::SubclassAndStyle(NoActivateChildKind::ChromiumHost)
        );
        assert_eq!(
            no_activate_child_policy(10, 20, 10, 20, "NewInteractiveChild"),
            NoActivateChildPolicy::SubclassOnly
        );
        assert_eq!(
            no_activate_child_policy(10, 20, 10, 20, "<class-unavailable>"),
            NoActivateChildPolicy::SubclassOnly
        );
        assert_eq!(
            no_activate_child_policy(10, 20, 11, 20, "WRY_WEBVIEW"),
            NoActivateChildPolicy::SkipDifferentIdentity
        );
        assert_eq!(
            no_activate_child_policy(10, 20, 10, 21, "Chrome_WidgetWin_0"),
            NoActivateChildPolicy::SkipDifferentIdentity
        );
        assert_eq!(
            no_activate_child_policy(10, 20, 11, 21, "Chrome_RenderWidgetHostHWND"),
            NoActivateChildPolicy::SkipDifferentIdentity
        );
    }

    struct FakeNativeVisibility {
        visible_after_hide: bool,
        calls: Vec<&'static str>,
    }

    impl NativeVisibilityApi for FakeNativeVisibility {
        fn hide(&mut self, _window: usize) {
            self.calls.push("hide");
        }

        fn is_visible(&mut self, _window: usize) -> bool {
            self.calls.push("is_visible");
            self.visible_after_hide
        }
    }

    #[test]
    fn native_hide_is_verified_before_surface_state_is_cleared() {
        let order = RefCell::new(Vec::new());
        let cleared = Cell::new(false);
        let mut api = FakeNativeVisibility {
            visible_after_hide: false,
            calls: Vec::new(),
        };

        hide_then_clear(
            || {
                order.borrow_mut().push("hide_and_verify");
                hide_and_verify(&mut api, 7)
            },
            || {
                order.borrow_mut().push("clear");
                cleared.set(true);
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(api.calls, ["hide", "is_visible"]);
        assert_eq!(*order.borrow(), ["hide_and_verify", "clear"]);
        assert!(cleared.get());
    }

    #[test]
    fn created_hidden_preview_is_configured_shown_and_verified_visible_in_order() {
        let visible = Cell::new(false);
        let order = RefCell::new(Vec::new());
        let placement = SurfacePlacement {
            position: PixelPoint { x: 12, y: 34 },
            size: PixelSize {
                width: 330,
                height: 230,
            },
        };

        let result = configure_show_verify_preview(
            || {
                order.borrow_mut().push("configure");
                assert!(!visible.get());
                Ok(())
            },
            || {
                order.borrow_mut().push("placement");
                Ok(placement)
            },
            |actual| {
                order.borrow_mut().push("show");
                assert_eq!(actual, placement);
                visible.set(true);
                Ok(())
            },
            || {
                order.borrow_mut().push("verify_visible");
                Ok(visible.get())
            },
        )
        .unwrap();

        assert_eq!(result, placement);
        assert_eq!(
            *order.borrow(),
            ["configure", "placement", "show", "verify_visible"]
        );
    }

    #[test]
    fn runtime_surface_routes_require_both_prepared_group_members() {
        assert!(require_prepared_surface_group(true, true).is_ok());
        for (main_prepared, preview_prepared) in [(false, false), (true, false), (false, true)] {
            assert!(matches!(
                require_prepared_surface_group(main_prepared, preview_prepared),
                Err(ClipboardSurfaceWindowError::SurfaceGroupNotPrepared)
            ));
        }
    }

    #[test]
    fn surface_underlay_color_is_strict_hex_and_defaults_to_light_border() {
        assert_eq!(
            ClipboardSurfaceUnderlayColor::parse_hex("#E0DEDC"),
            Some(DEFAULT_CLIPBOARD_SURFACE_UNDERLAY)
        );
        assert_eq!(DEFAULT_CLIPBOARD_SURFACE_UNDERLAY.as_hex(), "#e0dedc");
        for invalid in ["e0dedc", "#fff", "#e0dedcff", "#e0degc", "# e0dedc"] {
            assert_eq!(ClipboardSurfaceUnderlayColor::parse_hex(invalid), None);
        }
    }

    #[test]
    fn surface_underlay_updates_both_prepared_members_and_reports_any_failure() {
        let color = ClipboardSurfaceUnderlayColor::new(0x3b, 0x3e, 0x43);
        let labels = RefCell::new(Vec::new());
        let error = apply_underlay_to_prepared_group(true, true, color, |label, actual| {
            labels.borrow_mut().push((label, actual));
            if label == CLIPBOARD_SURFACE_LABEL {
                Err(ClipboardSurfaceWindowError::InvalidDimensions)
            } else {
                Ok(())
            }
        })
        .unwrap_err();

        assert!(matches!(
            error,
            ClipboardSurfaceWindowError::InvalidDimensions
        ));
        assert_eq!(
            *labels.borrow(),
            [
                (CLIPBOARD_SURFACE_LABEL, color),
                (CLIPBOARD_PREVIEW_SURFACE_LABEL, color)
            ]
        );
        assert!(matches!(
            apply_underlay_to_prepared_group(false, true, color, |_, _| Ok(())),
            Err(ClipboardSurfaceWindowError::SurfaceGroupNotPrepared)
        ));
    }

    #[test]
    fn preview_show_success_is_rejected_when_native_window_remains_hidden() {
        let result = configure_show_verify_preview(
            || Ok(()),
            || {
                Ok(SurfacePlacement {
                    position: PixelPoint { x: 0, y: 0 },
                    size: PixelSize {
                        width: 330,
                        height: 230,
                    },
                })
            },
            |_| Ok(()),
            || Ok(false),
        );

        assert!(matches!(
            result,
            Err(ClipboardSurfaceWindowError::PreviewStillHidden)
        ));
    }

    #[test]
    fn preview_close_hides_before_clearing_selection_and_preserves_it_on_hide_failure() {
        let selection = Cell::new(true);
        let order = RefCell::new(Vec::new());
        let had_selection = close_preview_lifecycle(
            || {
                order.borrow_mut().push("hide");
                Ok(())
            },
            || {
                order.borrow_mut().push("clear_selection");
                Ok(selection.replace(false))
            },
        )
        .unwrap();
        assert!(had_selection);
        assert!(!selection.get());
        assert_eq!(*order.borrow(), ["hide", "clear_selection"]);

        selection.set(true);
        let result = close_preview_lifecycle(
            || Err(ClipboardSurfaceWindowError::StillVisible),
            || Ok(selection.replace(false)),
        );
        assert!(matches!(
            result,
            Err(ClipboardSurfaceWindowError::StillVisible)
        ));
        assert!(selection.get());
    }

    #[test]
    fn still_visible_native_window_preserves_surface_state_for_retry() {
        let cleared = Cell::new(false);
        let mut api = FakeNativeVisibility {
            visible_after_hide: true,
            calls: Vec::new(),
        };

        let result = hide_then_clear(
            || hide_and_verify(&mut api, 11),
            || {
                cleared.set(true);
                Ok(())
            },
        );

        assert!(matches!(
            result,
            Err(ClipboardSurfaceWindowError::StillVisible)
        ));
        assert_eq!(api.calls, ["hide", "is_visible"]);
        assert!(!cleared.get());
    }

    #[test]
    fn route_and_label_are_stable_frontend_contracts() {
        assert_eq!(CLIPBOARD_SURFACE_LABEL, "clipboard-surface");
        assert_eq!(CLIPBOARD_SURFACE_ROUTE, "index.html#clipboard-surface");
        assert_eq!(CLIPBOARD_PREVIEW_SURFACE_LABEL, "clipboard-preview-surface");
        assert_eq!(
            CLIPBOARD_PREVIEW_SURFACE_ROUTE,
            "index.html#clipboard-preview-surface"
        );
        assert_eq!(CLIPBOARD_SURFACE_OPENED_CHANGE, "surface_opened");
        assert_eq!(CLIPBOARD_SURFACE_CLOSED_CHANGE, "surface_closed");
        assert_eq!(
            serde_json::to_value(ClipboardSurfaceStateEvent {
                change: CLIPBOARD_SURFACE_CLOSED_CHANGE,
            })
            .unwrap(),
            serde_json::json!({ "change": "surface_closed" })
        );
        assert_eq!(
            serde_json::to_value(ClipboardPreviewStateEvent {
                change: "selection_changed",
                record_id: Some("42".to_string()),
                visible: true,
            })
            .unwrap(),
            serde_json::json!({
                "change": "selection_changed",
                "recordId": "42",
                "visible": true
            })
        );
    }

    #[test]
    fn rounded_region_diameter_tracks_dpi_and_clamps_to_window_bounds() {
        assert_eq!(rounded_region_diameter(380, 520, 1.0), Some(24));
        assert_eq!(rounded_region_diameter(760, 1040, 2.0), Some(48));
        assert_eq!(rounded_region_diameter(20, 10, 1.0), Some(10));
        assert_eq!(rounded_region_diameter(0, 10, 1.0), None);
        assert_eq!(rounded_region_diameter(10, 10, f64::NAN), None);
    }

    #[cfg(windows)]
    #[test]
    fn rounded_region_uses_full_client_dimensions_and_contains_all_edge_midpoints() {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::Graphics::Gdi::{
            CreateRoundRectRgn, DeleteObject, GetRgnBox, PtInRegion, ERROR,
        };

        let client_rect = RECT {
            left: 0,
            top: 0,
            right: 380,
            bottom: 520,
        };
        // A stale or rounded Tauri outer size is deliberately irrelevant; the
        // client rectangle is the only region sizing source.
        let stale_tauri_outer = (379, 519);
        assert_eq!(stale_tauri_outer, (379, 519));
        let (width, height) = region_dimensions_from_client_rect(client_rect).unwrap();
        assert_eq!((width, height), (380, 520));

        let (region_right, region_bottom) = region_request_bounds(width, height).unwrap();
        assert_eq!((region_right, region_bottom), (381, 521));
        let region = unsafe { CreateRoundRectRgn(0, 0, region_right, region_bottom, 24, 24) };
        assert!(!region.is_null());
        let mut region_box = empty_windows_rect();
        assert_ne!(unsafe { GetRgnBox(region, &mut region_box) }, ERROR);
        assert_eq!(
            (
                region_box.left,
                region_box.top,
                region_box.right,
                region_box.bottom
            ),
            (0, 0, width, height)
        );
        for (x, y) in [
            (0, height / 2),
            (width / 2, 0),
            (width - 1, height / 2),
            (width / 2, height - 1),
        ] {
            assert_ne!(unsafe { PtInRegion(region, x, y) }, 0, "missing ({x},{y})");
        }
        for (x, y) in [
            (0, 0),
            (width - 1, 0),
            (0, height - 1),
            (width - 1, height - 1),
        ] {
            assert_eq!(unsafe { PtInRegion(region, x, y) }, 0, "included ({x},{y})");
        }
        unsafe {
            DeleteObject(region);
        }
    }

    fn monitor(bounds: PixelRect, work_area: PixelRect, scale_factor: f64) -> MonitorGeometry {
        MonitorGeometry {
            bounds,
            work_area,
            scale_factor,
        }
    }

    #[test]
    fn anchor_placement_prefers_right_and_below_then_flips_at_work_area_edges() {
        let geometry = monitor(
            PixelRect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            PixelRect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1040,
            },
            1.0,
        );
        assert_eq!(
            surface_placement(PixelPoint { x: 100, y: 100 }, geometry),
            Some(SurfacePlacement {
                position: PixelPoint { x: 112, y: 112 },
                size: PixelSize {
                    width: 380,
                    height: 520,
                },
            })
        );
        assert_eq!(
            surface_placement(PixelPoint { x: 1900, y: 1000 }, geometry),
            Some(SurfacePlacement {
                position: PixelPoint { x: 1508, y: 468 },
                size: PixelSize {
                    width: 380,
                    height: 520,
                },
            })
        );
    }

    #[test]
    fn valid_caret_wins_without_reading_mouse_and_missing_caret_falls_back() {
        let caret = PixelPoint { x: 420, y: 260 };
        let selected =
            resolve_surface_anchor(Some(caret), || -> Result<PixelPoint, &'static str> {
                panic!("mouse fallback must stay lazy when the caret is valid")
            })
            .unwrap();
        assert_eq!(selected, (caret, SurfaceAnchorSource::Caret));

        let mouse = PixelPoint { x: 800, y: 600 };
        assert_eq!(
            resolve_surface_anchor(None, || Ok::<_, &'static str>(mouse)),
            Ok((mouse, SurfaceAnchorSource::Cursor))
        );
        assert_eq!(
            resolve_surface_anchor(None, || Err::<PixelPoint, _>("cursor unavailable")),
            Err("cursor unavailable")
        );
    }

    #[test]
    fn caret_coordinate_pipeline_maps_client_to_screen_before_dpi_conversion() {
        let calls = RefCell::new(Vec::new());
        let converted = convert_caret_client_to_physical_screen(
            7_i32,
            |client| {
                calls.borrow_mut().push("client_to_screen");
                Ok::<_, &'static str>(client + 100)
            },
            |logical_screen| {
                calls.borrow_mut().push("logical_to_physical");
                assert_eq!(logical_screen, 107);
                Ok::<_, &'static str>((logical_screen * 2, "per_monitor"))
            },
        )
        .unwrap();

        assert_eq!(
            calls.into_inner(),
            vec!["client_to_screen", "logical_to_physical"]
        );
        assert_eq!(converted, (107, 214, "per_monitor"));
    }

    #[test]
    fn placement_handles_negative_monitors_taskbars_and_high_dpi() {
        let negative = monitor(
            PixelRect {
                left: -1920,
                top: 0,
                right: 0,
                bottom: 1080,
            },
            PixelRect {
                left: -1920,
                top: 0,
                right: 0,
                bottom: 1040,
            },
            1.0,
        );
        assert_eq!(
            surface_placement(PixelPoint { x: -10, y: 1070 }, negative),
            Some(SurfacePlacement {
                position: PixelPoint { x: -402, y: 520 },
                size: PixelSize {
                    width: 380,
                    height: 520,
                },
            })
        );

        let high_dpi = monitor(
            PixelRect {
                left: 0,
                top: 0,
                right: 2560,
                bottom: 1440,
            },
            PixelRect {
                left: 0,
                top: 0,
                right: 2560,
                bottom: 1400,
            },
            1.5,
        );
        assert_eq!(
            surface_placement(PixelPoint { x: 100, y: 100 }, high_dpi),
            Some(SurfacePlacement {
                position: PixelPoint { x: 118, y: 118 },
                size: PixelSize {
                    width: 570,
                    height: 780,
                },
            })
        );
    }

    #[test]
    fn all_four_work_area_corners_stay_inside_the_same_monitor() {
        let geometry = monitor(
            PixelRect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            PixelRect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1040,
            },
            1.0,
        );
        for (cursor, expected) in [
            (PixelPoint { x: 0, y: 0 }, PixelPoint { x: 12, y: 12 }),
            (PixelPoint { x: 1919, y: 0 }, PixelPoint { x: 1527, y: 12 }),
            (PixelPoint { x: 0, y: 1039 }, PixelPoint { x: 12, y: 507 }),
            (
                PixelPoint { x: 1919, y: 1039 },
                PixelPoint { x: 1527, y: 507 },
            ),
        ] {
            let placement = surface_placement(cursor, geometry).unwrap();
            assert_eq!(placement.position, expected);
            assert!(placement.position.x >= geometry.work_area.left);
            assert!(placement.position.y >= geometry.work_area.top);
            assert!(
                i64::from(placement.position.x) + i64::from(placement.size.width)
                    <= i64::from(geometry.work_area.right)
            );
            assert!(
                i64::from(placement.position.y) + i64::from(placement.size.height)
                    <= i64::from(geometry.work_area.bottom)
            );
        }
    }

    #[test]
    fn monitor_selection_uses_anchor_bounds_and_nearest_fallback() {
        let left = monitor(
            PixelRect {
                left: -1280,
                top: 0,
                right: 0,
                bottom: 1024,
            },
            PixelRect {
                left: -1280,
                top: 0,
                right: 0,
                bottom: 984,
            },
            1.0,
        );
        let right = monitor(
            PixelRect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            PixelRect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1040,
            },
            1.25,
        );
        assert_eq!(
            select_anchor_monitor(PixelPoint { x: -100, y: 50 }, &[right, left]),
            Some(left)
        );
        assert_eq!(
            select_anchor_monitor(PixelPoint { x: 2500, y: 50 }, &[left, right]),
            Some(right)
        );
    }

    #[test]
    fn placement_shrinks_to_tiny_work_area_without_crossing_any_edge() {
        let tiny = monitor(
            PixelRect {
                left: -300,
                top: -200,
                right: 0,
                bottom: 0,
            },
            PixelRect {
                left: -300,
                top: -200,
                right: 0,
                bottom: 0,
            },
            1.0,
        );
        assert_eq!(
            surface_placement(PixelPoint { x: -150, y: -100 }, tiny),
            Some(SurfacePlacement {
                position: PixelPoint { x: -300, y: -200 },
                size: PixelSize {
                    width: 300,
                    height: 200,
                },
            })
        );
    }

    #[test]
    fn preview_prefers_the_right_side_and_never_moves_above_or_below_the_popup() {
        let geometry = monitor(
            PixelRect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            PixelRect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1040,
            },
            1.0,
        );
        assert_eq!(
            preview_surface_placement(
                PixelRect {
                    left: 400,
                    top: 200,
                    right: 780,
                    bottom: 720,
                },
                geometry,
            ),
            Some(SurfacePlacement {
                position: PixelPoint { x: 780, y: 200 },
                size: PixelSize {
                    width: 330,
                    height: 230,
                },
            })
        );
    }

    #[test]
    fn preview_touches_main_at_every_dpi_without_intersecting() {
        let anchor = PixelRect {
            left: 800,
            top: 200,
            right: 1_180,
            bottom: 720,
        };
        for scale_factor in [1.0, 1.5, 2.0] {
            let geometry = monitor(
                PixelRect {
                    left: 0,
                    top: 0,
                    right: 4_000,
                    bottom: 2_500,
                },
                PixelRect {
                    left: 0,
                    top: 0,
                    right: 4_000,
                    bottom: 2_400,
                },
                scale_factor,
            );
            let preview = preview_surface_placement(anchor, geometry).unwrap();
            assert_eq!(preview.position.x - anchor.right, 0);
            assert!(preview.position.x >= anchor.right);
        }
    }

    #[test]
    fn preview_flips_only_horizontally_at_the_right_edge() {
        let geometry = monitor(
            PixelRect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            PixelRect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1040,
            },
            1.0,
        );
        assert_eq!(
            preview_surface_placement(
                PixelRect {
                    left: 1500,
                    top: 900,
                    right: 1880,
                    bottom: 1040,
                },
                geometry,
            ),
            Some(SurfacePlacement {
                position: PixelPoint { x: 1170, y: 810 },
                size: PixelSize {
                    width: 330,
                    height: 230,
                },
            })
        );
    }

    #[test]
    fn preview_respects_negative_monitor_coordinates_and_high_dpi_work_area() {
        let geometry = monitor(
            PixelRect {
                left: -2560,
                top: -200,
                right: 0,
                bottom: 1240,
            },
            PixelRect {
                left: -2560,
                top: -160,
                right: 0,
                bottom: 1200,
            },
            1.5,
        );
        assert_eq!(
            preview_surface_placement(
                PixelRect {
                    left: -1000,
                    top: -180,
                    right: -430,
                    bottom: 600,
                },
                geometry,
            ),
            Some(SurfacePlacement {
                position: PixelPoint { x: -1495, y: -160 },
                size: PixelSize {
                    width: 495,
                    height: 345,
                },
            })
        );
    }

    #[cfg(windows)]
    #[test]
    fn caret_rect_validation_accepts_zero_width_and_rejects_inverted_or_empty_height() {
        use windows_sys::Win32::Foundation::RECT;

        assert!(valid_caret_client_rect(RECT {
            left: 10,
            top: 20,
            right: 10,
            bottom: 38,
        }));
        assert!(!valid_caret_client_rect(RECT {
            left: 11,
            top: 20,
            right: 10,
            bottom: 38,
        }));
        assert!(!valid_caret_client_rect(RECT {
            left: 10,
            top: 20,
            right: 11,
            bottom: 20,
        }));
    }

    #[cfg(windows)]
    #[test]
    fn popup_styles_are_frameless_toolwindow_and_no_activate() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            WS_CAPTION, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
            WS_THICKFRAME,
        };
        let style = popup_window_style(WS_CAPTION | WS_THICKFRAME);
        assert_eq!(style & WS_CAPTION, 0);
        assert_eq!(style & WS_THICKFRAME, 0);
        assert_ne!(style & WS_POPUP, 0);

        let extended = popup_extended_style(WS_EX_APPWINDOW);
        assert_eq!(extended & WS_EX_APPWINDOW, 0);
        assert_ne!(extended & WS_EX_NOACTIVATE, 0);
        assert_ne!(extended & WS_EX_TOOLWINDOW, 0);

        let child_extended = child_no_activate_extended_style(0);
        assert_ne!(child_extended & WS_EX_NOACTIVATE, 0);
    }
}
