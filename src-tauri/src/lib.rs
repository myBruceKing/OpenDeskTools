mod commands;
mod infrastructure;

use std::sync::Arc;

use infrastructure::application::ApplicationRuntime;
use infrastructure::clipboard_surface_foreground;
use infrastructure::clipboard_surface_pointer;
use infrastructure::clipboard_surface_window::{
    self, ClipboardPreviewCloseReason, ClipboardSurfaceCloseReason,
    CLIPBOARD_PREVIEW_SURFACE_LABEL, CLIPBOARD_SURFACE_LABEL,
};
use infrastructure::debug_qa;
#[cfg(debug_assertions)]
use infrastructure::debug_qa::DebugQaOptions;
use infrastructure::hotkey::{HotkeyActionId, OrdinaryHotkeyTransition, TauriHotkeyRegistrar};
use infrastructure::keyboard_hook::{RuntimeHotkeyEvent, RuntimeHotkeyPhase};
use infrastructure::qr_toast_surface_window;
use infrastructure::tool_menu_surface_window::{self, TOOL_MENU_SURFACE_LABEL};
use infrastructure::tray::{
    route_window_lifecycle, TrayLifecycle, WindowLifecycleInput, WindowLifecycleRoute,
};
use infrastructure::windowing::configure_main_window;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_global_shortcut::{Shortcut, ShortcutEvent, ShortcutState};

const HOTKEY_ACTION_EVENT: &str = "hotkey://action";
const CLIPBOARD_HISTORY_CHANGED_EVENT: &str = "clipboard://history-changed";
const MAIN_WEBVIEW_LABEL: &str = "main";

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum HotkeyActionPhase {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HotkeyActionEvent {
    action_id: HotkeyActionId,
    phase: HotkeyActionPhase,
    timestamp_ms: u128,
    registration_revision: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClipboardHistoryChangedEvent {
    change: &'static str,
}

pub(crate) fn clipboard_history_event_sink<R: Runtime>(
    app: &AppHandle<R>,
) -> infrastructure::clipboard_listener::ClipboardHistoryEventSink {
    let event_app = app.clone();
    Arc::new(move || {
        for label in [
            MAIN_WEBVIEW_LABEL,
            CLIPBOARD_SURFACE_LABEL,
            CLIPBOARD_PREVIEW_SURFACE_LABEL,
        ] {
            if event_app.get_webview_window(label).is_some() {
                if let Err(error) = event_app.emit_to(
                    label,
                    CLIPBOARD_HISTORY_CHANGED_EVENT,
                    ClipboardHistoryChangedEvent { change: "recorded" },
                ) {
                    eprintln!("failed to emit clipboard history change to {label}: {error}");
                }
            }
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipboardSurfaceRequest {
    Toggle,
    #[cfg(debug_assertions)]
    Open,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipboardSurfaceRequestSource {
    OrdinaryHotkey,
    #[cfg(debug_assertions)]
    DebugQa,
}

impl ClipboardSurfaceRequestSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::OrdinaryHotkey => "ordinary_hotkey",
            #[cfg(debug_assertions)]
            Self::DebugQa => "debug_qa",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipboardSurfaceRequestRoute {
    Open,
    #[cfg(debug_assertions)]
    KeepOpen,
    Close(ClipboardSurfaceCloseReason),
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    // The official plugin contract requires single-instance to be registered
    // first because Tauri plugins currently execute in builder order. This
    // prevents a second process from reaching tray/listener/hook setup.
    #[cfg(windows)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
        debug_qa::trace(format!(
            "single instance activation args_count={} action=wake_main",
            args.len()
        ));
        if let Err(error) = infrastructure::tray::open_main_window(app) {
            eprintln!("failed to wake the existing main window from a second launch: {error}");
        }
    }));

    builder
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(handle_global_shortcut)
                .build(),
        )
        .setup(|app| {
            let qa_options = debug_qa::parse(std::env::args_os())?;
            let runtime = ApplicationRuntime::initialize(app.handle())?;
            app.manage(runtime);
            let runtime_state = app.state::<ApplicationRuntime>();
            if runtime_state.clipboard_monitoring_enabled() {
                if let Err(error) = runtime_state.start_clipboard_listener(clipboard_history_event_sink(app.handle())) {
                    eprintln!("clipboard listener unavailable during startup: {error}");
                }
            }
            // Construct every hotkey-owned WebView before registration. A
            // prepared failure changes the corresponding action to honestly
            // unavailable instead of leaving a registered shortcut that can
            // only log an error when pressed.
            let clipboard_surface_ready =
                match clipboard_surface_window::prepare_group(app.handle()) {
                    Ok(()) => true,
                    Err(error) => {
                        eprintln!(
                            "clipboard surface window group unavailable; the main toolbox will continue: {error}"
                        );
                        false
                    }
                };
            let quick_launch_ready = match runtime_state.quick_launch().snapshot() {
                Ok(_) => true,
                Err(error) => {
                    eprintln!("quick launch state unavailable during startup: {error}");
                    false
                }
            };
            let tool_menu_surface_ready = quick_launch_ready
                && match tool_menu_surface_window::prepare(app.handle()) {
                    Ok(()) => true,
                    Err(error) => {
                        eprintln!("tool menu surface unavailable: {error}");
                        false
                    }
                };
            let qr_toast_surface_ready =
                match qr_toast_surface_window::prepare(app.handle()) {
                    Ok(()) => true,
                    Err(error) => {
                        eprintln!("QR feedback surface unavailable: {error}");
                        false
                    }
                };
            let forced_app = app.handle().clone();
            let runtime_state = app.state::<ApplicationRuntime>();
            runtime_state.hotkeys().set_initial_action_available(
                HotkeyActionId::ClipboardOpenPanel,
                clipboard_surface_ready,
            )?;
            runtime_state.hotkeys().set_initial_action_available(
                HotkeyActionId::LauncherOpen,
                tool_menu_surface_ready,
            )?;
            runtime_state.hotkeys().set_initial_action_available(
                HotkeyActionId::ClipboardQrConvert,
                qr_toast_surface_ready,
            )?;
            let registrar = TauriHotkeyRegistrar::new(
                app.handle(),
                runtime_state.keyboard_hook(),
                move |event| queue_forced_hotkey_event(&forced_app, event),
            );
            let hotkey_snapshot = runtime_state.hotkeys().reconcile(&registrar)?;
            runtime_state.sync_system_hotkey_disable(&hotkey_snapshot);
            // Keep the registered login command pointed at the current
            // executable (self-heals across moves/updates) without ever
            // re-enabling autostart from a login launch.
            if let Err(error) = runtime_state.autostart().sync_if_enabled() {
                eprintln!("failed to reconcile the autostart command: {error}");
            }
            let autostart_launch =
                infrastructure::autostart::is_autostart_launch(std::env::args_os());
            let start_minimized = runtime_state.start_minimized();
            if let Some(window) = app.get_webview_window("main") {
                configure_main_window(&window)?;
                // The main window ships hidden (`visible: false`) so a login
                // autostart launch stays silent in the tray. A normal launch
                // reveals it explicitly unless the user asked to start
                // minimized, avoiding a startup flash either way.
                if !autostart_launch && !start_minimized {
                    if let Err(error) = window.show() {
                        eprintln!("failed to reveal the main window on launch: {error}");
                    }
                }
            }
            app.manage(TrayLifecycle::default());
            infrastructure::tray::install(app.handle())?;
            #[cfg(debug_assertions)]
            schedule_debug_qa(app.handle(), qa_options);
            #[cfg(not(debug_assertions))]
            let _ = qa_options;
            Ok(())
        })
        .on_page_load(|webview, payload| {
            if should_stop_capture_on_page_load(webview.label(), payload.event()) {
                if let Some(runtime) = webview.app_handle().try_state::<ApplicationRuntime>() {
                    let _ = runtime.hotkey_capture().stop_active();
                }
            }
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                handle_main_window_event(window, event);
            } else if window.label() == CLIPBOARD_SURFACE_LABEL {
                handle_clipboard_surface_window_event(window, event);
            } else if window.label() == CLIPBOARD_PREVIEW_SURFACE_LABEL {
                handle_clipboard_preview_surface_window_event(window, event);
            } else if window.label() == TOOL_MENU_SURFACE_LABEL {
                handle_tool_menu_surface_window_event(window, event);
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::clipboard::get_clipboard_history,
            commands::clipboard::set_clipboard_monitoring,
            commands::clipboard::update_clipboard_settings,
            commands::clipboard::set_clipboard_history_favorite,
            commands::clipboard::delete_clipboard_history_item,
            commands::clipboard::clear_unfavorite_clipboard_history,
            commands::clipboard::get_clipboard_history_image,
            commands::clipboard::update_clipboard_history_text,
            commands::clipboard::get_clipboard_history_source_icon,
            commands::clipboard::copy_clipboard_history_item,
            commands::clipboard::input_clipboard_history_item,
            commands::clipboard::close_clipboard_surface,
            commands::clipboard::open_clipboard_preview_surface,
            commands::clipboard::close_clipboard_preview_surface,
            commands::clipboard::get_clipboard_preview_surface_state,
            commands::clipboard::trace_clipboard_preview_debug,
            commands::clipboard::set_clipboard_surface_underlay_color,
            commands::hotkey::start_hotkey_capture,
            commands::hotkey::stop_hotkey_capture,
            commands::hotkey::get_hotkey_snapshot,
            commands::hotkey::classify_hotkey_binding,
            commands::hotkey::update_hotkey_binding,
            commands::overview::get_overview_view_model,
            commands::qr::convert_latest_clipboard_qr,
            commands::quick_launch::get_quick_launch_snapshot,
            commands::quick_launch::rescan_quick_launch,
            commands::quick_launch::pin_quick_launch_app,
            commands::quick_launch::unpin_quick_launch_app,
            commands::quick_launch::set_quick_launch_visible,
            commands::quick_launch::reorder_quick_launch_apps,
            commands::quick_launch::swap_quick_launch_apps,
            commands::quick_launch::update_tool_menu_preferences,
            commands::quick_launch::launch_quick_launch_app,
            commands::quick_launch::get_quick_launch_icon,
            commands::quick_launch::select_quick_launch_app,
            commands::quick_launch::close_tool_menu_surface,
            commands::general::get_general_settings,
            commands::general::set_autostart_enabled,
            commands::general::set_start_minimized,
            commands::general::set_close_to_tray,
            commands::general::set_crash_diagnostics_enabled,
            commands::general::select_and_migrate_data_directory,
            commands::theme::get_theme_preferences,
            commands::theme::update_theme_preferences
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn handle_clipboard_preview_surface_window_event<R: Runtime>(
    window: &tauri::Window<R>,
    event: &tauri::WindowEvent,
) {
    match event {
        tauri::WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            if let Err(error) = clipboard_surface_window::close_preview(
                window.app_handle(),
                ClipboardPreviewCloseReason::WindowRequest,
            ) {
                eprintln!("failed to close clipboard preview from window request: {error}");
            }
        }
        tauri::WindowEvent::Resized(_) | tauri::WindowEvent::ScaleFactorChanged { .. } => {
            if let Some(webview) = window
                .app_handle()
                .get_webview_window(CLIPBOARD_PREVIEW_SURFACE_LABEL)
            {
                clipboard_surface_window::refresh_native_shape_or_log(&webview);
            }
        }
        tauri::WindowEvent::Destroyed => {
            if let Err(error) = clipboard_surface_window::forget_preview_state() {
                eprintln!("failed to clear destroyed clipboard preview state: {error}");
            }
            if let Some(runtime) = window.app_handle().try_state::<ApplicationRuntime>() {
                if clipboard_surface_window::is_visible(window.app_handle()) {
                    if let Err(error) = clipboard_surface_window::close(
                        window.app_handle(),
                        runtime.surface(),
                        ClipboardSurfaceCloseReason::PreviewDestroyed,
                    ) {
                        eprintln!(
                            "failed to close clipboard surface after preview destruction: {error}"
                        );
                    }
                }
            }
        }
        _ => {}
    }
}

fn handle_tool_menu_surface_window_event<R: Runtime>(
    window: &tauri::Window<R>,
    event: &tauri::WindowEvent,
) {
    match event {
        tauri::WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            if let Err(error) = tool_menu_surface_window::request_hide(window.app_handle()) {
                eprintln!("failed to hide tool menu surface: {error}");
            }
        }
        // A retained menu is intentionally still dismissed by clicking any
        // other application or surface, rather than remaining above it.
        tauri::WindowEvent::Focused(false) if tool_menu_surface_window::lost_foreground(window) => {
            if let Err(error) = tool_menu_surface_window::request_hide(window.app_handle()) {
                eprintln!("failed to hide tool menu after confirmed foreground change: {error}");
            }
        }
        _ => {}
    }
}

fn show_tool_menu_surface<R: Runtime>(
    app: &AppHandle<R>,
    runtime: &ApplicationRuntime,
) -> Result<(), tool_menu_surface_window::ToolMenuSurfaceError> {
    let snapshot = runtime.quick_launch().snapshot().map_err(|error| {
        tool_menu_surface_window::ToolMenuSurfaceError::QuickLaunch(error.to_string())
    })?;
    tool_menu_surface_window::show(app, &snapshot)
}

fn release_tool_menu_surface<R: Runtime>(
    app: &AppHandle<R>,
    runtime: &ApplicationRuntime,
) -> Result<(), tool_menu_surface_window::ToolMenuSurfaceError> {
    let preferences = runtime
        .quick_launch()
        .tool_menu_preferences()
        .map_err(|error| {
            tool_menu_surface_window::ToolMenuSurfaceError::QuickLaunch(error.to_string())
        })?;
    if !preferences.keep_open_on_key_release {
        tool_menu_surface_window::request_hide(app)?;
    }
    Ok(())
}

fn should_stop_capture_on_page_load(
    webview_label: &str,
    event: tauri::webview::PageLoadEvent,
) -> bool {
    webview_label == "main" && event == tauri::webview::PageLoadEvent::Started
}

fn handle_main_window_event<R: Runtime>(window: &tauri::Window<R>, event: &tauri::WindowEvent) {
    let input = match event {
        tauri::WindowEvent::CloseRequested { .. } => WindowLifecycleInput::CloseRequested,
        tauri::WindowEvent::Focused(false) => WindowLifecycleInput::FocusLost,
        tauri::WindowEvent::Destroyed => WindowLifecycleInput::Destroyed,
        _ => WindowLifecycleInput::Other,
    };
    let exit_requested = window
        .app_handle()
        .try_state::<TrayLifecycle>()
        .is_some_and(|lifecycle| lifecycle.is_exit_requested());
    let close_to_tray = window
        .app_handle()
        .try_state::<ApplicationRuntime>()
        .is_none_or(|runtime| runtime.close_to_tray());
    let route = route_window_lifecycle(input, exit_requested, close_to_tray);
    execute_main_window_route(window, event, route);
}

fn execute_main_window_route<R: Runtime>(
    window: &tauri::Window<R>,
    event: &tauri::WindowEvent,
    route: WindowLifecycleRoute,
) {
    if route.stop_capture {
        if let Some(runtime) = window.app_handle().try_state::<ApplicationRuntime>() {
            if let Err(error) = runtime.hotkey_capture().stop_active() {
                eprintln!("failed to stop native hotkey capture on main-window event: {error}");
            }
        }
    }

    if route.prevent_close {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
        }
    }
    if route.hide_main {
        if let Err(error) = window.hide() {
            eprintln!("failed to hide the main window to the tray: {error}");
        }
    }
    if route.exit_app {
        // "Close to tray" is disabled: run the full teardown sequence and quit.
        infrastructure::tray::exit_application(window.app_handle());
    }
}

fn handle_clipboard_surface_window_event<R: Runtime>(
    window: &tauri::Window<R>,
    event: &tauri::WindowEvent,
) {
    let Some(runtime) = window.app_handle().try_state::<ApplicationRuntime>() else {
        return;
    };
    match event {
        tauri::WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            if let Err(error) = clipboard_surface_window::close(
                window.app_handle(),
                runtime.surface(),
                ClipboardSurfaceCloseReason::WindowRequest,
            ) {
                eprintln!("failed to close clipboard surface from window request: {error}");
            }
        }
        tauri::WindowEvent::Focused(false) => {
            if runtime.surface().should_close_on_focus_loss() {
                if let Err(error) = clipboard_surface_window::close(
                    window.app_handle(),
                    runtime.surface(),
                    ClipboardSurfaceCloseReason::FocusLost,
                ) {
                    eprintln!("failed to close clipboard surface after focus loss: {error}");
                }
            }
        }
        tauri::WindowEvent::Resized(_) | tauri::WindowEvent::ScaleFactorChanged { .. } => {
            if let Some(webview) = window
                .app_handle()
                .get_webview_window(CLIPBOARD_SURFACE_LABEL)
            {
                clipboard_surface_window::refresh_native_shape_or_log(&webview);
            }
        }
        tauri::WindowEvent::Destroyed => {
            if let Err(error) = clipboard_surface_foreground::stop() {
                eprintln!("failed to stop destroyed clipboard surface monitor: {error}");
            }
            if let Err(error) = clipboard_surface_pointer::stop() {
                eprintln!("failed to stop destroyed clipboard outside-pointer monitor: {error}");
            }
            if let Err(error) = clipboard_surface_window::close_preview(
                window.app_handle(),
                ClipboardPreviewCloseReason::MainSurfaceDestroyed,
            ) {
                eprintln!("failed to close preview after surface destruction: {error}");
            }
            if let Err(error) = clipboard_surface_window::forget_preview_state() {
                eprintln!("failed to clear preview after surface destruction: {error}");
            }
            if let Err(error) = runtime.surface().clear() {
                eprintln!("failed to clear destroyed clipboard surface state: {error}");
            }
        }
        _ => {}
    }
}

fn handle_global_shortcut<R: Runtime>(
    app: &AppHandle<R>,
    shortcut: &Shortcut,
    event: ShortcutEvent,
) {
    let Some(runtime) = app.try_state::<ApplicationRuntime>() else {
        return;
    };
    let Some((action_id, registration_revision)) =
        runtime.hotkeys().registered_action_for_shortcut(shortcut)
    else {
        return;
    };
    let binding = shortcut.to_string();
    let phase = match event.state {
        ShortcutState::Pressed => HotkeyActionPhase::Pressed,
        ShortcutState::Released => HotkeyActionPhase::Released,
    };
    let transition = match phase {
        HotkeyActionPhase::Pressed => OrdinaryHotkeyTransition::Pressed,
        HotkeyActionPhase::Released => OrdinaryHotkeyTransition::Released,
    };
    if !runtime
        .ordinary_hotkey_latch()
        .consume(action_id, &binding, registration_revision, transition)
        .unwrap_or(false)
    {
        return;
    }
    if action_id == HotkeyActionId::ClipboardOpenPanel
        && matches!(phase, HotkeyActionPhase::Pressed)
    {
        if let Err(error) = request_clipboard_surface_from_foreground(
            app,
            &runtime,
            ClipboardSurfaceRequest::Toggle,
            ClipboardSurfaceRequestSource::OrdinaryHotkey,
        ) {
            eprintln!("failed to process clipboard surface hotkey request: {error}");
        }
    }
    if action_id == HotkeyActionId::ClipboardQrConvert
        && matches!(phase, HotkeyActionPhase::Pressed)
    {
        trigger_qr_conversion(app, &runtime);
    }
    if action_id == HotkeyActionId::LauncherOpen {
        let result = match phase {
            HotkeyActionPhase::Pressed => show_tool_menu_surface(app, &runtime),
            HotkeyActionPhase::Released => release_tool_menu_surface(app, &runtime),
        };
        if let Err(error) = result {
            eprintln!("failed to process tool menu hotkey: {error}");
        }
    }
    if !should_broadcast_hotkey_action(action_id) {
        return;
    }
    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let _ = app.emit(
        HOTKEY_ACTION_EVENT,
        HotkeyActionEvent {
            action_id,
            phase,
            timestamp_ms,
            registration_revision,
        },
    );
}

pub(crate) fn handle_forced_hotkey_event<R: Runtime>(
    app: &AppHandle<R>,
    event: RuntimeHotkeyEvent,
) {
    let Some(runtime) = app.try_state::<ApplicationRuntime>() else {
        return;
    };
    let Some((action_id, registration_revision)) = runtime
        .hotkeys()
        .registered_action_for_forced_generation(event.generation)
    else {
        return;
    };
    let phase = match event.phase {
        RuntimeHotkeyPhase::Pressed => HotkeyActionPhase::Pressed,
        RuntimeHotkeyPhase::Released => HotkeyActionPhase::Released,
    };
    if action_id == HotkeyActionId::ClipboardOpenPanel
        && matches!(phase, HotkeyActionPhase::Pressed)
    {
        if clipboard_surface_window::is_visible(app) {
            if let Err(error) = clipboard_surface_window::close(
                app,
                runtime.surface(),
                ClipboardSurfaceCloseReason::ForcedHotkeyToggle,
            ) {
                eprintln!("failed to toggle the forced clipboard surface closed: {error}");
                disable_forced_hotkey_after_route_failure(
                    app,
                    event.generation,
                    format!("关闭剪贴板面板失败：{error}"),
                );
                return;
            }
        } else {
            let candidate = match (
                event.foreground_window,
                event.foreground_process_id,
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
                if let Err(error) = runtime.surface().activate_without_target() {
                    disable_forced_hotkey_after_route_failure(
                        app,
                        event.generation,
                        format!("初始化剪贴板面板失败：{error}"),
                    );
                    return;
                }
            }
            if let Err(error) = show_clipboard_surface(app, &runtime, "forced_hotkey") {
                eprintln!("failed to show forced clipboard surface: {error}");
                disable_forced_hotkey_after_route_failure(
                    app,
                    event.generation,
                    format!("显示剪贴板面板失败：{error}"),
                );
                return;
            }
        }
    }
    if action_id == HotkeyActionId::ClipboardQrConvert
        && matches!(phase, HotkeyActionPhase::Pressed)
    {
        trigger_qr_conversion(app, &runtime);
    }
    if action_id == HotkeyActionId::LauncherOpen {
        let result = match phase {
            HotkeyActionPhase::Pressed => show_tool_menu_surface(app, &runtime),
            HotkeyActionPhase::Released => release_tool_menu_surface(app, &runtime),
        };
        if let Err(error) = result {
            disable_forced_hotkey_after_route_failure(
                app,
                event.generation,
                format!("工具盘窗口操作失败：{error}"),
            );
            return;
        }
    }
    if !should_broadcast_hotkey_action(action_id) {
        return;
    }
    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let _ = app.emit(
        HOTKEY_ACTION_EVENT,
        HotkeyActionEvent {
            action_id,
            phase,
            timestamp_ms,
            registration_revision,
        },
    );
}

fn trigger_qr_conversion<R: Runtime>(app: &AppHandle<R>, _runtime: &ApplicationRuntime) {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(runtime) = worker_app.try_state::<ApplicationRuntime>() else {
            return;
        };
        let payload = match commands::qr::convert_latest_and_notify(&worker_app, &runtime) {
            Ok(result) => serde_json::json!({
                "success": true,
                "kind": result.kind,
                "systemClipboardSynced": result.system_clipboard_synced,
                "message": result.message,
            }),
            Err(error) => serde_json::json!({
                "success": false,
                "kind": null,
                "systemClipboardSynced": false,
                "message": error.message,
                "code": error.code,
            }),
        };
        let toast_app = worker_app.clone();
        if let Err(error) = worker_app.run_on_main_thread(move || {
            if let Err(error) = qr_toast_surface_window::show(&toast_app, &payload) {
                eprintln!("failed to show QR conversion feedback: {error}");
            }
        }) {
            eprintln!("failed to dispatch QR conversion feedback: {error}");
        }
    });
}

pub(crate) fn queue_forced_hotkey_event<R: Runtime>(app: &AppHandle<R>, event: RuntimeHotkeyEvent) {
    let main_thread_app = app.clone();
    let generation = event.generation;
    debug_qa::trace(format!(
        "forced hotkey dispatch queued generation={generation} phase={:?}",
        event.phase
    ));
    if let Err(error) = app.run_on_main_thread(move || {
        debug_qa::trace(format!(
            "forced hotkey dispatch main_thread generation={} phase={:?}",
            event.generation, event.phase
        ));
        handle_forced_hotkey_event(&main_thread_app, event);
    }) {
        disable_forced_hotkey_after_route_failure(
            app,
            generation,
            format!("快捷键事件无法切换到窗口线程：{error}"),
        );
    }
}

fn disable_forced_hotkey_after_route_failure<R: Runtime>(
    app: &AppHandle<R>,
    generation: u64,
    reason: String,
) {
    let Some(runtime) = app.try_state::<ApplicationRuntime>() else {
        return;
    };
    let unregister_result = runtime.keyboard_hook().unregister_win_v(generation);
    let restored = unregister_result.as_ref().is_ok_and(|removed| *removed);
    let detail = if unregister_result.is_ok() {
        format!("{reason}。强制覆盖已停止，系统快捷键已恢复；请重试或重启应用。")
    } else {
        format!("{reason}。强制覆盖后端未能正常停止，请立即退出并重启应用。")
    };
    match runtime
        .hotkeys()
        .mark_forced_generation_degraded(generation, detail.clone())
    {
        Ok(true) => debug_qa::trace(format!(
            "forced hotkey degraded generation={generation} input_restored={restored} detail={detail}"
        )),
        Ok(false) => debug_qa::trace(format!(
            "forced hotkey degrade ignored stale_generation={generation} input_restored={restored}"
        )),
        Err(error) => eprintln!(
            "failed to mark forced hotkey generation {generation} degraded after route failure: {error}"
        ),
    }
    if let Err(error) = unregister_result {
        eprintln!(
            "failed to unregister forced hotkey generation {generation} after route failure: {error}"
        );
    }
}

fn clipboard_surface_request_route(
    visible: bool,
    request: ClipboardSurfaceRequest,
    source: ClipboardSurfaceRequestSource,
) -> ClipboardSurfaceRequestRoute {
    match (visible, request) {
        (true, ClipboardSurfaceRequest::Toggle) => {
            ClipboardSurfaceRequestRoute::Close(match source {
                ClipboardSurfaceRequestSource::OrdinaryHotkey => {
                    ClipboardSurfaceCloseReason::HotkeyToggle
                }
                #[cfg(debug_assertions)]
                ClipboardSurfaceRequestSource::DebugQa => ClipboardSurfaceCloseReason::DebugQaReset,
            })
        }
        #[cfg(debug_assertions)]
        (true, ClipboardSurfaceRequest::Open) => ClipboardSurfaceRequestRoute::KeepOpen,
        (false, _) => ClipboardSurfaceRequestRoute::Open,
    }
}

fn request_clipboard_surface_from_foreground<R: Runtime>(
    app: &AppHandle<R>,
    runtime: &ApplicationRuntime,
    request: ClipboardSurfaceRequest,
    source: ClipboardSurfaceRequestSource,
) -> Result<(), String> {
    let visible = clipboard_surface_window::is_visible(app);
    debug_qa::trace(format!(
        "surface request source={} request={request:?} visible_before={visible}",
        source.as_str()
    ));
    match clipboard_surface_request_route(visible, request, source) {
        ClipboardSurfaceRequestRoute::Close(reason) => {
            return clipboard_surface_window::close(app, runtime.surface(), reason)
                .map_err(|error| error.to_string());
        }
        #[cfg(debug_assertions)]
        ClipboardSurfaceRequestRoute::KeepOpen => {
            debug_qa::trace(format!(
                "surface request source={} kept existing visible surface",
                source.as_str()
            ));
            return Ok(());
        }
        ClipboardSurfaceRequestRoute::Open => {}
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
    show_clipboard_surface(app, runtime, source.as_str())
}

fn show_clipboard_surface<R: Runtime>(
    app: &AppHandle<R>,
    runtime: &ApplicationRuntime,
    source: &str,
) -> Result<(), String> {
    let result = clipboard_surface_window::prepared_main(app)
        .and_then(|window| clipboard_surface_window::show(&window, runtime.surface()));
    match result {
        Ok(()) => {
            notify_clipboard_surface_opened(app);
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

#[cfg(debug_assertions)]
fn schedule_debug_qa<R: Runtime>(app: &AppHandle<R>, options: DebugQaOptions) {
    let Some(delay) = options.open_clipboard_surface_after else {
        return;
    };
    debug_qa::trace(format!(
        "scheduled deterministic open delay_ms={} trace_path={}",
        delay.as_millis(),
        debug_qa::trace_path().display()
    ));
    let qa_app = app.clone();
    let spawn_result = std::thread::Builder::new()
        .name("clipboard-surface-qa-delay".to_owned())
        .spawn(move || {
            std::thread::sleep(delay);
            let request_app = qa_app.clone();
            if let Err(error) = qa_app.run_on_main_thread(move || {
                debug_qa::trace("deterministic open timer fired");
                let Some(runtime) = request_app.try_state::<ApplicationRuntime>() else {
                    debug_qa::trace("deterministic open failed: runtime state unavailable");
                    return;
                };
                if let Err(error) = request_clipboard_surface_from_foreground(
                    &request_app,
                    &runtime,
                    ClipboardSurfaceRequest::Open,
                    ClipboardSurfaceRequestSource::DebugQa,
                ) {
                    debug_qa::trace(format!("deterministic open failed: {error}"));
                }
            }) {
                debug_qa::trace(format!("deterministic open dispatch failed: {error}"));
            }
        });
    if let Err(error) = spawn_result {
        debug_qa::trace(format!("deterministic open timer thread failed: {error}"));
    }
}

fn notify_clipboard_surface_opened<R: Runtime>(app: &AppHandle<R>) {
    clipboard_surface_window::notify_opened(app);
}

fn should_broadcast_hotkey_action(action_id: HotkeyActionId) -> bool {
    action_id != HotkeyActionId::ClipboardOpenPanel
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipboardSurfaceOpenRoute {
    ShowWithInput,
    ShowBrowseOnly,
}

#[cfg(test)]
fn clipboard_surface_open_route(target_captured: bool) -> ClipboardSurfaceOpenRoute {
    if target_captured {
        ClipboardSurfaceOpenRoute::ShowWithInput
    } else {
        ClipboardSurfaceOpenRoute::ShowBrowseOnly
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_webview_navigation_start_is_a_capture_cleanup_boundary() {
        assert!(should_stop_capture_on_page_load(
            "main",
            tauri::webview::PageLoadEvent::Started
        ));
        assert!(!should_stop_capture_on_page_load(
            "main",
            tauri::webview::PageLoadEvent::Finished
        ));
        assert!(!should_stop_capture_on_page_load(
            "secondary",
            tauri::webview::PageLoadEvent::Started
        ));
    }

    #[test]
    fn clipboard_history_event_payload_is_minimal_and_contains_no_clipboard_content() {
        assert_eq!(
            CLIPBOARD_HISTORY_CHANGED_EVENT,
            "clipboard://history-changed"
        );
        assert_eq!(MAIN_WEBVIEW_LABEL, "main");
        assert_eq!(CLIPBOARD_SURFACE_LABEL, "clipboard-surface");
        assert_eq!(
            clipboard_surface_window::CLIPBOARD_SURFACE_OPENED_CHANGE,
            "surface_opened"
        );
        assert_eq!(
            clipboard_surface_window::CLIPBOARD_SURFACE_CLOSED_CHANGE,
            "surface_closed"
        );
        assert_eq!(
            serde_json::to_value(ClipboardHistoryChangedEvent { change: "recorded" }).unwrap(),
            serde_json::json!({ "change": "recorded" })
        );
        assert_eq!(
            serde_json::to_value(ClipboardHistoryChangedEvent {
                change: clipboard_surface_window::CLIPBOARD_SURFACE_OPENED_CHANGE
            })
            .unwrap(),
            serde_json::json!({ "change": "surface_opened" })
        );
    }

    #[test]
    fn clipboard_panel_hotkey_is_consumed_by_native_surface_without_main_navigation_event() {
        assert!(!should_broadcast_hotkey_action(
            HotkeyActionId::ClipboardOpenPanel
        ));
        for action in [
            HotkeyActionId::ScreenshotCapture,
            HotkeyActionId::ClipboardPinImage,
            HotkeyActionId::ClipboardQrConvert,
            HotkeyActionId::LauncherOpen,
        ] {
            assert!(should_broadcast_hotkey_action(action));
        }
    }

    #[test]
    fn clipboard_surface_always_opens_and_only_external_target_enables_input() {
        // Main/own foreground and absent HWND/PID both fail target capture but still
        // open a browse/copy-capable surface. A validated external target enables input.
        assert_eq!(
            clipboard_surface_open_route(false),
            ClipboardSurfaceOpenRoute::ShowBrowseOnly
        );
        assert_eq!(
            clipboard_surface_open_route(false),
            ClipboardSurfaceOpenRoute::ShowBrowseOnly
        );
        assert_eq!(
            clipboard_surface_open_route(true),
            ClipboardSurfaceOpenRoute::ShowWithInput
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    fn ordinary_toggle_and_debug_open_share_the_same_surface_request_router() {
        assert_eq!(
            clipboard_surface_request_route(
                false,
                ClipboardSurfaceRequest::Toggle,
                ClipboardSurfaceRequestSource::OrdinaryHotkey,
            ),
            ClipboardSurfaceRequestRoute::Open
        );
        assert_eq!(
            clipboard_surface_request_route(
                true,
                ClipboardSurfaceRequest::Toggle,
                ClipboardSurfaceRequestSource::OrdinaryHotkey,
            ),
            ClipboardSurfaceRequestRoute::Close(ClipboardSurfaceCloseReason::HotkeyToggle)
        );
        assert_eq!(
            clipboard_surface_request_route(
                false,
                ClipboardSurfaceRequest::Open,
                ClipboardSurfaceRequestSource::DebugQa,
            ),
            ClipboardSurfaceRequestRoute::Open
        );
        assert_eq!(
            clipboard_surface_request_route(
                true,
                ClipboardSurfaceRequest::Open,
                ClipboardSurfaceRequestSource::DebugQa,
            ),
            ClipboardSurfaceRequestRoute::KeepOpen
        );
    }

    #[test]
    fn close_reasons_are_stable_debug_trace_contracts() {
        assert_eq!(
            ClipboardSurfaceCloseReason::HotkeyToggle.as_str(),
            "hotkey_toggle"
        );
        assert_eq!(
            ClipboardSurfaceCloseReason::ForcedHotkeyToggle.as_str(),
            "forced_hotkey_toggle"
        );
        #[cfg(debug_assertions)]
        assert_eq!(
            ClipboardSurfaceCloseReason::DebugQaReset.as_str(),
            "debug_qa_reset"
        );
        assert_eq!(
            ClipboardSurfaceCloseReason::WindowRequest.as_str(),
            "window_request"
        );
        assert_eq!(
            ClipboardSurfaceCloseReason::FocusLost.as_str(),
            "focused_false"
        );
        assert_eq!(
            ClipboardSurfaceCloseReason::ForegroundChanged.as_str(),
            "foreground_changed"
        );
        assert_eq!(
            ClipboardSurfaceCloseReason::PointerOutside.as_str(),
            "pointer_outside"
        );
        assert_eq!(
            ClipboardSurfaceCloseReason::PreviewDestroyed.as_str(),
            "preview_destroyed"
        );
        assert_eq!(ClipboardSurfaceCloseReason::Command.as_str(), "command");
        assert_eq!(
            ClipboardSurfaceCloseReason::InputSucceeded.as_str(),
            "input_succeeded"
        );
    }
}
