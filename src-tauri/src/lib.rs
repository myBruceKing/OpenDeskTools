mod commands;
mod infrastructure;

use infrastructure::application::ApplicationRuntime;
use infrastructure::hotkey::{HotkeyActionId, TauriHotkeyRegistrar};
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
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
