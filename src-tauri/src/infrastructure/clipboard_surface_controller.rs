use tauri::{AppHandle, Manager, Runtime};

use super::{
    application::ApplicationRuntime,
    clipboard_surface_window::{self, ClipboardSurfaceCloseReason},
    debug_qa,
    windowing::MAIN_WEBVIEW_LABEL,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SurfaceRequest {
    Toggle,
    #[cfg(debug_assertions)]
    Open,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SurfaceRequestSource {
    OrdinaryHotkey,
    #[cfg(debug_assertions)]
    DebugQa,
}

impl SurfaceRequestSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::OrdinaryHotkey => "ordinary_hotkey",
            #[cfg(debug_assertions)]
            Self::DebugQa => "debug_qa",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SurfaceRequestRoute {
    Open,
    #[cfg(debug_assertions)]
    KeepOpen,
    Close(ClipboardSurfaceCloseReason),
}

#[derive(Debug)]
pub enum ForcedSurfaceToggleError {
    Close(String),
    Initialize(String),
    Show(String),
}

impl ForcedSurfaceToggleError {
    pub fn user_message(&self) -> String {
        match self {
            Self::Close(error) => format!("关闭剪贴板面板失败：{error}"),
            Self::Initialize(error) => format!("初始化剪贴板面板失败：{error}"),
            Self::Show(error) => format!("显示剪贴板面板失败：{error}"),
        }
    }
}

impl std::fmt::Display for ForcedSurfaceToggleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Close(error) => write!(formatter, "close failed: {error}"),
            Self::Initialize(error) => write!(formatter, "initialization failed: {error}"),
            Self::Show(error) => write!(formatter, "show failed: {error}"),
        }
    }
}

pub fn toggle_from_foreground<R: Runtime>(
    app: &AppHandle<R>,
    runtime: &ApplicationRuntime,
) -> Result<(), String> {
    request_from_foreground(
        app,
        runtime,
        SurfaceRequest::Toggle,
        SurfaceRequestSource::OrdinaryHotkey,
    )
}

#[cfg(debug_assertions)]
pub fn open_from_foreground<R: Runtime>(
    app: &AppHandle<R>,
    runtime: &ApplicationRuntime,
) -> Result<(), String> {
    request_from_foreground(
        app,
        runtime,
        SurfaceRequest::Open,
        SurfaceRequestSource::DebugQa,
    )
}

pub fn toggle_from_forced_candidate<R: Runtime>(
    app: &AppHandle<R>,
    runtime: &ApplicationRuntime,
    foreground_window: Option<usize>,
    foreground_process_id: Option<u32>,
) -> Result<(), ForcedSurfaceToggleError> {
    if clipboard_surface_window::is_visible(app) {
        return clipboard_surface_window::close(
            app,
            runtime.surface(),
            ClipboardSurfaceCloseReason::ForcedHotkeyToggle,
        )
        .map_err(|error| ForcedSurfaceToggleError::Close(error.to_string()));
    }

    let candidate = match (
        foreground_window,
        foreground_process_id,
        app.get_webview_window(MAIN_WEBVIEW_LABEL),
    ) {
        (Some(candidate), Some(candidate_pid), Some(main)) => {
            #[cfg(windows)]
            let own_window = main.hwnd().ok().map(|hwnd| hwnd.0 as usize);
            #[cfg(not(windows))]
            let own_window = Some(0);
            own_window.map(|own_window| (candidate, candidate_pid, own_window))
        }
        _ => None,
    };
    let captured = candidate.is_some_and(|(candidate, candidate_pid, own_window)| {
        runtime
            .surface()
            .capture_external_candidate(candidate, candidate_pid, own_window)
            .is_ok()
    });
    if !captured {
        runtime
            .surface()
            .activate_without_target()
            .map_err(|error| ForcedSurfaceToggleError::Initialize(error.to_string()))?;
    }
    show(app, runtime, "forced_hotkey").map_err(ForcedSurfaceToggleError::Show)
}

fn request_route(
    visible: bool,
    request: SurfaceRequest,
    source: SurfaceRequestSource,
) -> SurfaceRequestRoute {
    match (visible, request) {
        (true, SurfaceRequest::Toggle) => SurfaceRequestRoute::Close(match source {
            SurfaceRequestSource::OrdinaryHotkey => ClipboardSurfaceCloseReason::HotkeyToggle,
            #[cfg(debug_assertions)]
            SurfaceRequestSource::DebugQa => ClipboardSurfaceCloseReason::DebugQaReset,
        }),
        #[cfg(debug_assertions)]
        (true, SurfaceRequest::Open) => SurfaceRequestRoute::KeepOpen,
        (false, _) => SurfaceRequestRoute::Open,
    }
}

fn request_from_foreground<R: Runtime>(
    app: &AppHandle<R>,
    runtime: &ApplicationRuntime,
    request: SurfaceRequest,
    source: SurfaceRequestSource,
) -> Result<(), String> {
    let visible = clipboard_surface_window::is_visible(app);
    debug_qa::trace(format!(
        "surface request source={} request={request:?} visible_before={visible}",
        source.as_str()
    ));
    match request_route(visible, request, source) {
        SurfaceRequestRoute::Close(reason) => {
            return clipboard_surface_window::close(app, runtime.surface(), reason)
                .map_err(|error| error.to_string());
        }
        #[cfg(debug_assertions)]
        SurfaceRequestRoute::KeepOpen => {
            debug_qa::trace(format!(
                "surface request source={} kept existing visible surface",
                source.as_str()
            ));
            return Ok(());
        }
        SurfaceRequestRoute::Open => {}
    }

    let main_window = app
        .get_webview_window(MAIN_WEBVIEW_LABEL)
        .ok_or_else(|| "main window is unavailable".to_owned())?;
    #[cfg(windows)]
    let owner_window = main_window
        .hwnd()
        .map(|handle| handle.0 as usize)
        .map_err(|error| format!("main HWND is unavailable: {error}"))?;
    #[cfg(not(windows))]
    let owner_window = {
        let _ = main_window;
        return Err("native clipboard surface is unavailable".to_owned());
    };

    let capture_result = runtime.surface().capture_external_target(owner_window);
    if let Err(error) = &capture_result {
        debug_qa::trace(format!(
            "target capture unavailable source={} error={error}",
            source.as_str()
        ));
        runtime
            .surface()
            .activate_without_target()
            .map_err(|fallback| {
                format!("target capture failed: {error}; fallback failed: {fallback}")
            })?;
    }
    debug_qa::trace(format!(
        "target captured source={} target_top={:?} input_available={}",
        source.as_str(),
        runtime.surface().target_top_window(),
        runtime.surface().input_available()
    ));
    show(app, runtime, source.as_str())
}

fn show<R: Runtime>(
    app: &AppHandle<R>,
    runtime: &ApplicationRuntime,
    source: &str,
) -> Result<(), String> {
    let result = clipboard_surface_window::prepared_main(app)
        .and_then(|window| clipboard_surface_window::show(&window, runtime.surface()));
    match result {
        Ok(()) => {
            clipboard_surface_window::notify_opened(app);
            debug_qa::trace(format!(
                "show success source={source} visible_after={}",
                clipboard_surface_window::is_visible(app)
            ));
            Ok(())
        }
        Err(error) => {
            let clear_result = runtime.surface().clear();
            debug_qa::trace(format!(
                "show failure source={source} error={error} clear_result={clear_result:?}"
            ));
            Err(error.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(debug_assertions)]
    #[test]
    fn ordinary_toggle_and_debug_open_share_one_request_router() {
        assert_eq!(
            request_route(
                false,
                SurfaceRequest::Toggle,
                SurfaceRequestSource::OrdinaryHotkey,
            ),
            SurfaceRequestRoute::Open
        );
        assert_eq!(
            request_route(
                true,
                SurfaceRequest::Toggle,
                SurfaceRequestSource::OrdinaryHotkey,
            ),
            SurfaceRequestRoute::Close(ClipboardSurfaceCloseReason::HotkeyToggle)
        );
        assert_eq!(
            request_route(false, SurfaceRequest::Open, SurfaceRequestSource::DebugQa,),
            SurfaceRequestRoute::Open
        );
        assert_eq!(
            request_route(true, SurfaceRequest::Open, SurfaceRequestSource::DebugQa,),
            SurfaceRequestRoute::KeepOpen
        );
    }

    #[test]
    fn forced_toggle_error_preserves_stage_specific_user_messages() {
        assert_eq!(
            ForcedSurfaceToggleError::Close("x".to_owned()).user_message(),
            "关闭剪贴板面板失败：x"
        );
        assert_eq!(
            ForcedSurfaceToggleError::Initialize("x".to_owned()).user_message(),
            "初始化剪贴板面板失败：x"
        );
        assert_eq!(
            ForcedSurfaceToggleError::Show("x".to_owned()).user_message(),
            "显示剪贴板面板失败：x"
        );
    }
}
