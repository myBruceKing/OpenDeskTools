mod commands;
mod hotkey_action_controller;
mod infrastructure;

use std::sync::Arc;

use infrastructure::application::ApplicationRuntime;
#[cfg(debug_assertions)]
use infrastructure::clipboard_surface_controller;
use infrastructure::clipboard_surface_foreground;
use infrastructure::clipboard_surface_window::{
    self, ClipboardPreviewCloseReason, ClipboardSurfaceCloseReason,
    CLIPBOARD_PREVIEW_SURFACE_LABEL, CLIPBOARD_SURFACE_LABEL,
};
use infrastructure::debug_qa;
#[cfg(debug_assertions)]
use infrastructure::debug_qa::DebugQaOptions;
use infrastructure::hotkey::{HotkeyActionId, TauriHotkeyRegistrar};
use infrastructure::qr_toast_surface_window;
use infrastructure::surface_pointer_monitor::{self, PointerMonitorOwner};
use infrastructure::tool_menu_surface_window::{self, TOOL_MENU_SURFACE_LABEL};
use infrastructure::tray::{
    route_window_lifecycle, TrayLifecycle, WindowLifecycleInput, WindowLifecycleRoute,
};
use infrastructure::usage_statistics::UsageAction;
use infrastructure::windowing::{configure_main_window, MAIN_WEBVIEW_LABEL};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_global_shortcut::{Shortcut, ShortcutEvent, ShortcutState};

const CLIPBOARD_HISTORY_CHANGED_EVENT: &str = "clipboard://history-changed";
const USAGE_STATISTICS_CHANGED_EVENT: &str = "usage://statistics-changed";

#[cfg(debug_assertions)]
pub fn write_debug_screenshot_probe_report() -> Result<std::path::PathBuf, String> {
    let report = infrastructure::screenshot::probe::run_capture_probe()
        .map_err(|error| error.to_string())?;
    infrastructure::screenshot::probe::write_report(&report).map_err(|error| error.to_string())
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
    let startup_arguments = std::env::args_os().collect::<Vec<_>>();
    if let Some(exit_code) =
        infrastructure::elevated_autostart::configuration_exit_code(startup_arguments.clone())
    {
        std::process::exit(exit_code);
    }
    match infrastructure::elevated_autostart::redirect_normal_launch(startup_arguments.clone()) {
        Ok(true) => return,
        Ok(false) => {}
        Err(error) => {
            eprintln!("failed to redirect launch through the elevated task: {error}");
        }
    }
    if let Err(error) = infrastructure::elevation::wait_for_restart_parent() {
        eprintln!("administrator restart handshake failed: {error}");
        return;
    }
    let elevated_wake_requested = infrastructure::elevated_autostart::consume_wake_request();
    let primary_instance = match infrastructure::single_instance::claim() {
        Ok(infrastructure::single_instance::InstanceClaim::Primary(primary)) => Some(primary),
        Ok(infrastructure::single_instance::InstanceClaim::SecondaryNotified) => return,
        Err(error) => {
            eprintln!("single-instance coordination unavailable; continuing startup: {error}");
            None
        }
    };
    let builder = tauri::Builder::default();

    builder
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(handle_global_shortcut)
                .build(),
        )
        .setup(move |app| {
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
                move |event| hotkey_action_controller::queue_forced(&forced_app, event),
            );
            let hotkey_snapshot = runtime_state.hotkeys().reconcile(&registrar)?;
            runtime_state.sync_system_hotkey_disable(&hotkey_snapshot);
            // Elevated autostart owns the login launch when present. Remove a
            // stale ordinary Run entry so the two mechanisms cannot race into
            // separate normal/elevated instances. Otherwise keep the ordinary
            // command aligned with the current executable path.
            if runtime_state.elevated_autostart().is_enabled() {
                if let Err(error) = runtime_state.autostart().set(false) {
                    eprintln!("failed to remove duplicate ordinary autostart: {error}");
                }
            } else if let Err(error) = runtime_state.autostart().sync_if_enabled() {
                eprintln!("failed to reconcile the autostart command: {error}");
            }
            runtime_state.mark_startup_ready();
            let autostart_launch =
                infrastructure::autostart::is_autostart_launch(std::env::args_os())
                    && !elevated_wake_requested;
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
            infrastructure::tray::install(app.handle(), runtime_state.tray_icon_visible())?;
            if let Some(primary_instance) = primary_instance {
                let single_instance = primary_instance.start_listener(app.handle())?;
                app.manage(single_instance);
            }
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
            commands::general::set_elevated_autostart_enabled,
            commands::general::set_start_minimized,
            commands::general::set_close_to_tray,
            commands::general::set_tray_icon_visible,
            commands::general::restart_as_administrator,
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
        tauri::WindowEvent::Destroyed => tool_menu_surface_window::forget_destroyed(),
        _ => {}
    }
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
            if let Err(error) = clipboard_surface_window::stop_navigation_monitor(&runtime) {
                eprintln!("failed to stop destroyed clipboard navigation capture: {error}");
            }
            if let Err(error) = clipboard_surface_window::stop_escape_monitor(&runtime) {
                eprintln!("failed to stop destroyed clipboard Escape capture: {error}");
            }
            if let Err(error) = clipboard_surface_foreground::stop() {
                eprintln!("failed to stop destroyed clipboard surface monitor: {error}");
            }
            if let Err(error) = surface_pointer_monitor::stop(PointerMonitorOwner::Clipboard) {
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
        ShortcutState::Pressed => hotkey_action_controller::HotkeyActionPhase::Pressed,
        ShortcutState::Released => hotkey_action_controller::HotkeyActionPhase::Released,
    };
    hotkey_action_controller::dispatch_standard(
        app,
        &runtime,
        action_id,
        registration_revision,
        binding,
        phase,
    );
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

#[cfg(debug_assertions)]
fn schedule_debug_qa<R: Runtime>(app: &AppHandle<R>, options: DebugQaOptions) {
    if let Some(delay) = options.open_clipboard_surface_after {
        debug_qa::trace!(format!(
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
                    debug_qa::trace!("deterministic open timer fired");
                    let Some(runtime) = request_app.try_state::<ApplicationRuntime>() else {
                        debug_qa::trace!("deterministic open failed: runtime state unavailable");
                        return;
                    };
                    if let Err(error) =
                        clipboard_surface_controller::open_from_foreground(&request_app, &runtime)
                    {
                        debug_qa::trace!(format!("deterministic open failed: {error}"));
                    }
                }) {
                    debug_qa::trace!(format!("deterministic open dispatch failed: {error}"));
                }
            });
        if let Err(error) = spawn_result {
            debug_qa::trace!(format!("deterministic open timer thread failed: {error}"));
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
