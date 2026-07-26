use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::commands;
use crate::infrastructure::application::ApplicationRuntime;
use crate::infrastructure::clipboard_surface_controller;
use crate::infrastructure::debug_qa;
use crate::infrastructure::hotkey::{HotkeyActionId, OrdinaryHotkeyTransition};
use crate::infrastructure::keyboard_hook::{RuntimeHotkeyEvent, RuntimeHotkeyPhase};
use crate::infrastructure::qr_toast_surface_window;
use crate::infrastructure::tool_menu_surface_window;
use crate::infrastructure::usage_statistics::UsageAction;
use crate::record_usage_success;

const HOTKEY_ACTION_EVENT: &str = "hotkey://action";

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HotkeyActionPhase {
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

#[derive(Debug, Clone, Copy)]
enum HotkeyBackend {
    Standard,
    Forced {
        foreground_window: Option<usize>,
        foreground_process_id: Option<u32>,
    },
}

pub(crate) fn dispatch_standard<R: Runtime>(
    app: &AppHandle<R>,
    runtime: &ApplicationRuntime,
    action_id: HotkeyActionId,
    registration_revision: u64,
    binding: String,
    phase: HotkeyActionPhase,
) {
    let transition = match phase {
        HotkeyActionPhase::Pressed => OrdinaryHotkeyTransition::Pressed,
        HotkeyActionPhase::Released => OrdinaryHotkeyTransition::Released,
    };
    let should_dispatch = match runtime.ordinary_hotkey_latch().consume(
        action_id,
        &binding,
        registration_revision,
        transition,
    ) {
        Ok(should_dispatch) => should_dispatch,
        Err(error) => {
            eprintln!(
                "failed to update ordinary hotkey latch action={} binding={binding}: {error}",
                action_id.as_str()
            );
            return;
        }
    };
    if !should_dispatch {
        return;
    }
    debug_qa::trace!(format!(
        "hotkey dispatch backend=standard action={} phase={phase:?} binding={binding}",
        action_id.as_str(),
    ));
    let _ = route_action(
        app,
        runtime,
        action_id,
        registration_revision,
        phase,
        HotkeyBackend::Standard,
    );
}

pub(crate) fn queue_forced<R: Runtime>(app: &AppHandle<R>, event: RuntimeHotkeyEvent) {
    let main_thread_app = app.clone();
    let generation = event.generation;
    debug_qa::trace!(format!(
        "forced hotkey dispatch queued generation={generation} phase={:?}",
        event.phase
    ));
    if let Err(error) = app.run_on_main_thread(move || {
        debug_qa::trace!(format!(
            "forced hotkey dispatch main_thread generation={} phase={:?}",
            event.generation, event.phase
        ));
        dispatch_forced(&main_thread_app, event);
    }) {
        disable_forced_after_route_failure(
            app,
            generation,
            format!("快捷键事件无法切换到窗口线程：{error}"),
        );
    }
}

fn dispatch_forced<R: Runtime>(app: &AppHandle<R>, event: RuntimeHotkeyEvent) {
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
    debug_qa::trace!(format!(
        "hotkey dispatch backend=forced action={} phase={phase:?} foreground_hwnd={:?} foreground_pid={:?}",
        action_id.as_str(),
        event.foreground_window,
        event.foreground_process_id
    ));
    if let Err(reason) = route_action(
        app,
        &runtime,
        action_id,
        registration_revision,
        phase,
        HotkeyBackend::Forced {
            foreground_window: event.foreground_window,
            foreground_process_id: event.foreground_process_id,
        },
    ) {
        disable_forced_after_route_failure(app, event.generation, reason);
    }
}

fn route_action<R: Runtime>(
    app: &AppHandle<R>,
    runtime: &ApplicationRuntime,
    action_id: HotkeyActionId,
    registration_revision: u64,
    phase: HotkeyActionPhase,
    backend: HotkeyBackend,
) -> Result<(), String> {
    match action_id {
        HotkeyActionId::ClipboardOpenPanel if matches!(phase, HotkeyActionPhase::Pressed) => {
            route_clipboard_panel(app, runtime, backend)?;
        }
        HotkeyActionId::ClipboardQrConvert if matches!(phase, HotkeyActionPhase::Pressed) => {
            trigger_qr_conversion(app);
        }
        HotkeyActionId::ScreenshotCapture if matches!(phase, HotkeyActionPhase::Pressed) => {
            trigger_screenshot_capture(app);
        }
        HotkeyActionId::ClipboardPinImage if matches!(phase, HotkeyActionPhase::Pressed) => {
            trigger_pin_latest_image(app);
        }
        HotkeyActionId::LauncherOpen => {
            route_tool_menu(app, runtime, phase, backend)?;
        }
        _ => {}
    }

    if should_broadcast_hotkey_action(action_id) {
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
    Ok(())
}

fn route_clipboard_panel<R: Runtime>(
    app: &AppHandle<R>,
    runtime: &ApplicationRuntime,
    backend: HotkeyBackend,
) -> Result<(), String> {
    match backend {
        HotkeyBackend::Standard => {
            match clipboard_surface_controller::toggle_from_foreground(app, runtime) {
                Ok(()) => record_usage_success(app, runtime, UsageAction::ClipboardPanel),
                Err(error) => {
                    eprintln!("failed to process clipboard surface hotkey request: {error}");
                }
            }
            Ok(())
        }
        HotkeyBackend::Forced {
            foreground_window,
            foreground_process_id,
            ..
        } => clipboard_surface_controller::toggle_from_forced_candidate(
            app,
            runtime,
            foreground_window,
            foreground_process_id,
        )
        .map(|()| record_usage_success(app, runtime, UsageAction::ClipboardPanel))
        .map_err(|error| error.user_message()),
    }
}

fn route_tool_menu<R: Runtime>(
    app: &AppHandle<R>,
    runtime: &ApplicationRuntime,
    phase: HotkeyActionPhase,
    backend: HotkeyBackend,
) -> Result<(), String> {
    let result = match phase {
        HotkeyActionPhase::Pressed => show_tool_menu_surface(app, runtime),
        HotkeyActionPhase::Released => release_tool_menu_surface(app, runtime),
    };
    match (backend, result) {
        (_, Ok(())) => {
            if matches!(phase, HotkeyActionPhase::Pressed) {
                record_usage_success(app, runtime, UsageAction::ToolMenu);
            }
            Ok(())
        }
        (HotkeyBackend::Standard, Err(error)) => {
            eprintln!("failed to process tool menu hotkey: {error}");
            Ok(())
        }
        (HotkeyBackend::Forced { .. }, Err(error)) => Err(format!("工具盘窗口操作失败：{error}")),
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

fn trigger_qr_conversion<R: Runtime>(app: &AppHandle<R>) {
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
                    "screenshot completed width={} height={} history_status={}",
                    result.width.unwrap_or_default(),
                    result.height.unwrap_or_default(),
                    result.history_status
                );
            }
            Err(error) => {
                show_capture_error(&worker_app, error.code, error.message);
                eprintln!(
                    "screenshot capture failed code={} message={}",
                    error.code, error.message
                );
            }
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
                show_capture_error(&worker_app, error.code, error.message);
                eprintln!(
                    "pin image failed code={} message={}",
                    error.code, error.message
                );
            }
        }
    });
}

fn show_capture_error<R: Runtime>(app: &AppHandle<R>, code: &'static str, message: &'static str) {
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

fn disable_forced_after_route_failure<R: Runtime>(
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
        Ok(true) => debug_qa::trace!(format!(
            "forced hotkey degraded generation={generation} input_restored={restored} detail={detail}"
        )),
        Ok(false) => debug_qa::trace!(format!(
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
    fn native_hotkey_actions_do_not_broadcast_main_navigation_events() {
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
}
