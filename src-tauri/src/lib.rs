mod commands;
mod infrastructure;

use infrastructure::application::ApplicationRuntime;
use infrastructure::hotkey::{HotkeyActionId, TauriHotkeyRegistrar};
use infrastructure::tray::{
    route_window_lifecycle, TrayLifecycle, WindowLifecycleInput, WindowLifecycleRoute,
};
use infrastructure::windowing::configure_main_window;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_global_shortcut::{Shortcut, ShortcutEvent, ShortcutState};

const HOTKEY_ACTION_EVENT: &str = "hotkey://action";

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(handle_global_shortcut)
                .build(),
        )
        .setup(|app| {
            let runtime = ApplicationRuntime::initialize(app.handle())?;
            app.manage(runtime);
            let registrar = TauriHotkeyRegistrar::new(app.handle());
            app.state::<ApplicationRuntime>()
                .hotkeys()
                .reconcile(&registrar)?;
            if let Some(window) = app.get_webview_window("main") {
                configure_main_window(&window)?;
            }
            app.manage(TrayLifecycle::default());
            infrastructure::tray::install(app.handle())?;
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
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::clipboard::get_clipboard_history,
            commands::clipboard::set_clipboard_history_favorite,
            commands::clipboard::delete_clipboard_history_item,
            commands::clipboard::clear_unfavorite_clipboard_history,
            commands::hotkey::start_hotkey_capture,
            commands::hotkey::stop_hotkey_capture,
            commands::hotkey::get_hotkey_snapshot,
            commands::hotkey::classify_hotkey_binding,
            commands::hotkey::update_hotkey_binding,
            commands::overview::get_overview_view_model,
            commands::theme::get_theme_preferences,
            commands::theme::update_theme_preferences
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
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
    let route = route_window_lifecycle(input, exit_requested);
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
}

fn handle_global_shortcut<R: Runtime>(
    app: &AppHandle<R>,
    shortcut: &Shortcut,
    event: ShortcutEvent,
) {
    let Some(runtime) = app.try_state::<ApplicationRuntime>() else {
        return;
    };
    let Some((action_id, registration_revision)) = runtime
        .hotkeys()
        .registered_action_for_plugin_binding(&shortcut.to_string())
    else {
        return;
    };
    let phase = match event.state {
        ShortcutState::Pressed => HotkeyActionPhase::Pressed,
        ShortcutState::Released => HotkeyActionPhase::Released,
    };
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
}
