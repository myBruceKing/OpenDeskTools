mod commands;
mod infrastructure;

use std::sync::Arc;

use infrastructure::application::ApplicationRuntime;
use infrastructure::clipboard_surface_controller;
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
use infrastructure::usage_statistics::UsageAction;
use infrastructure::windowing::{configure_main_window, MAIN_WEBVIEW_LABEL};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_global_shortcut::{Shortcut, ShortcutEvent, ShortcutState};

const HOTKEY_ACTION_EVENT: &str = "hotkey://action";
const CLIPBOARD_HISTORY_CHANGED_EVENT: &str = "clipboard://history-changed";
const USAGE_STATISTICS_CHANGED_EVENT: &str = "usage://statistics-changed";

#[cfg(debug_assertions)]
pub fn write_debug_screenshot_probe_report() -> Result<std::path::PathBuf, String> {
    let report =
        infrastructure::screenshot::probe::run_gdi_probe().map_err(|error| error.to_string())?;
    infrastructure::screenshot::probe::write_report(&report).map_err(|error| error.to_string())
}

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
            let screenshot_ready = match runtime_state.screenshot().probe() {
                Ok(()) => true,
                Err(error) => {
                    eprintln!("screenshot service unavailable: {error}");
                    false
                }
            };
            let pin_image_ready = match runtime_state.pin_image().probe() {
                Ok(()) => true,
                Err(error) => {
                    eprintln!("pin image service unavailable: {error}");
                    false
                }
            };
            runtime_state.hotkeys().set_initial_action_available(
                HotkeyActionId::ScreenshotCapture,
                screenshot_ready,
            )?;
            runtime_state.hotkeys().set_initial_action_available(
                HotkeyActionId::ClipboardPinImage,
                pin_image_ready,
            )?;
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
            runtime_state.mark_startup_ready();
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
            commands::capture::capture_screenshot,
            commands::capture::pin_latest_image,
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
            commands::hotkey::update_hotkey_enabled,
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
            commands::theme::update_theme_preferences,
            commands::theme::select_theme_background,
            commands::theme::remove_theme_background,
            commands::theme::get_theme_background_image
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
            if let Err(error) = runtime.keyboard_hook().stop_surface_escape() {
                eprintln!("failed to stop destroyed clipboard Escape capture: {error}");
            }
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
        match clipboard_surface_controller::toggle_from_foreground(app, &runtime) {
            Ok(()) => record_usage_success(app, &runtime, UsageAction::ClipboardPanel),
            Err(error) => {
                eprintln!("failed to process clipboard surface hotkey request: {error}");
            }
        }
    }
    if action_id == HotkeyActionId::ClipboardQrConvert
        && matches!(phase, HotkeyActionPhase::Pressed)
    {
        trigger_qr_conversion(app, &runtime);
    }
    if action_id == HotkeyActionId::ScreenshotCapture && matches!(phase, HotkeyActionPhase::Pressed)
    {
        trigger_screenshot_capture(app);
    }
    if action_id == HotkeyActionId::ClipboardPinImage && matches!(phase, HotkeyActionPhase::Pressed)
    {
        trigger_pin_latest_image(app);
    }
    if action_id == HotkeyActionId::LauncherOpen {
        let result = match phase {
            HotkeyActionPhase::Pressed => show_tool_menu_surface(app, &runtime),
            HotkeyActionPhase::Released => release_tool_menu_surface(app, &runtime),
        };
        match result {
            Ok(()) if matches!(phase, HotkeyActionPhase::Pressed) => {
                record_usage_success(app, &runtime, UsageAction::ToolMenu);
            }
            Ok(()) => {}
            Err(error) => {
                eprintln!("failed to process tool menu hotkey: {error}");
            }
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
        if let Err(error) = clipboard_surface_controller::toggle_from_forced_candidate(
            app,
            &runtime,
            event.foreground_window,
            event.foreground_process_id,
        ) {
            eprintln!("failed to process forced clipboard surface toggle: {error}");
            disable_forced_hotkey_after_route_failure(app, event.generation, error.user_message());
            return;
        }
        record_usage_success(app, &runtime, UsageAction::ClipboardPanel);
    }
    if action_id == HotkeyActionId::ClipboardQrConvert
        && matches!(phase, HotkeyActionPhase::Pressed)
    {
        trigger_qr_conversion(app, &runtime);
    }
    if action_id == HotkeyActionId::ScreenshotCapture && matches!(phase, HotkeyActionPhase::Pressed)
    {
        trigger_screenshot_capture(app);
    }
    if action_id == HotkeyActionId::ClipboardPinImage && matches!(phase, HotkeyActionPhase::Pressed)
    {
        trigger_pin_latest_image(app);
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
        if matches!(phase, HotkeyActionPhase::Pressed) {
            record_usage_success(app, &runtime, UsageAction::ToolMenu);
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

fn trigger_screenshot_capture<R: Runtime>(app: &AppHandle<R>) {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(runtime) = worker_app.try_state::<ApplicationRuntime>() else {
            return;
        };
        match commands::capture::capture_and_notify(&worker_app, &runtime) {
            Ok(result) if result.status == "cancelled" => {}
            Ok(result) => {
                eprintln!(
                    "screenshot copied width={} height={}",
                    result.width.unwrap_or_default(),
                    result.height.unwrap_or_default()
                );
            }
            Err(error) => eprintln!(
                "screenshot capture failed code={} message={}",
                error.code, error.message
            ),
        }
    });
}

fn trigger_pin_latest_image<R: Runtime>(app: &AppHandle<R>) {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(runtime) = worker_app.try_state::<ApplicationRuntime>() else {
            return;
        };
        match commands::capture::pin_latest_and_record(&worker_app, &runtime) {
            Ok(outcome) => {
                eprintln!(
                    "image pinned pin_id={} width={} height={}",
                    outcome.pin_id, outcome.width, outcome.height
                );
            }
            Err(error) => {
                show_pin_image_error(&worker_app, error.code, error.message);
                eprintln!(
                    "pin image failed code={} message={}",
                    error.code, error.message
                );
            }
        }
    });
}

fn show_pin_image_error<R: Runtime>(app: &AppHandle<R>, code: &'static str, message: &'static str) {
    let payload = serde_json::json!({
        "success": false,
        "kind": null,
        "systemClipboardSynced": false,
        "message": message,
        "code": code,
    });
    let toast_app = app.clone();
    if let Err(dispatch_error) = app.run_on_main_thread(move || {
        if let Err(show_error) = qr_toast_surface_window::show(&toast_app, &payload) {
            eprintln!("failed to show pin image feedback: {show_error}");
        }
    }) {
        eprintln!("failed to dispatch pin image feedback: {dispatch_error}");
    }
}

pub(crate) fn record_usage_success<R: Runtime>(
    app: &AppHandle<R>,
    runtime: &ApplicationRuntime,
    action: UsageAction,
) {
    match runtime.usage_statistics().record_success(action) {
        Ok(()) => {
            if let Err(error) = app.emit_to(MAIN_WEBVIEW_LABEL, USAGE_STATISTICS_CHANGED_EVENT, ())
            {
                eprintln!("failed to publish usage statistics change: {error}");
            }
        }
        Err(error) => {
            eprintln!("failed to record successful tool usage: {error}");
        }
    }
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

#[cfg(debug_assertions)]
fn schedule_debug_qa<R: Runtime>(app: &AppHandle<R>, options: DebugQaOptions) {
    if let Some(delay) = options.open_clipboard_surface_after {
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
                    if let Err(error) =
                        clipboard_surface_controller::open_from_foreground(&request_app, &runtime)
                    {
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

    if options.screenshot_probe {
        let spawn_result = std::thread::Builder::new()
            .name("screenshot-qa-probe".to_owned())
            .spawn(|| match write_debug_screenshot_probe_report() {
                Ok(path) => eprintln!("[screenshot-probe] report={}", path.display()),
                Err(error) => eprintln!("[screenshot-probe] failed: {error}"),
            });
        if let Err(error) = spawn_result {
            eprintln!("[screenshot-probe] failed to start: {error}");
        }
    }
}

fn should_broadcast_hotkey_action(action_id: HotkeyActionId) -> bool {
    !matches!(
        action_id,
        HotkeyActionId::ClipboardOpenPanel
            | HotkeyActionId::ScreenshotCapture
            | HotkeyActionId::ClipboardPinImage
    )
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
        assert!(!should_broadcast_hotkey_action(
            HotkeyActionId::ScreenshotCapture
        ));
        assert!(!should_broadcast_hotkey_action(
            HotkeyActionId::ClipboardPinImage
        ));
        for action in [
            HotkeyActionId::ClipboardQrConvert,
            HotkeyActionId::LauncherOpen,
        ] {
            assert!(should_broadcast_hotkey_action(action));
        }
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
        assert_eq!(ClipboardSurfaceCloseReason::Escape.as_str(), "escape");
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
