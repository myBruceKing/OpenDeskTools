use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

use super::application::ApplicationRuntime;
use super::clipboard_surface_foreground;
use super::clipboard_surface_pointer;

const TRAY_ID: &str = "open-desk-tools";
const OPEN_MENU_ID: &str = "tray.open-main";
const EXIT_MENU_ID: &str = "tray.exit-application";
const TRAY_TOOLTIP: &str = "OpenDeskTools - 桌面工具箱";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    OpenMain,
    ExitApplication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayPointerInput {
    LeftButtonReleased,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowLifecycleInput {
    CloseRequested,
    FocusLost,
    Destroyed,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowLifecycleRoute {
    pub stop_capture: bool,
    pub prevent_close: bool,
    pub hide_main: bool,
    pub exit_app: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitStep {
    StopClipboardListener,
    StopCapture,
    StopKeyboardHook,
    StopSurfaceMonitors,
    UnregisterGlobalShortcuts,
    RestoreSystemHotkeys,
    ExitApplication,
}

const EXIT_STEPS: [ExitStep; 7] = [
    ExitStep::StopClipboardListener,
    ExitStep::StopCapture,
    ExitStep::StopKeyboardHook,
    ExitStep::StopSurfaceMonitors,
    ExitStep::UnregisterGlobalShortcuts,
    ExitStep::RestoreSystemHotkeys,
    ExitStep::ExitApplication,
];

#[derive(Debug, Default)]
pub struct TrayLifecycle {
    exit_requested: AtomicBool,
}

impl TrayLifecycle {
    pub fn is_exit_requested(&self) -> bool {
        self.exit_requested.load(Ordering::Acquire)
    }

    fn request_exit(&self) -> bool {
        self.exit_requested
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

pub fn action_for_menu_id(menu_id: &str) -> Option<TrayAction> {
    match menu_id {
        OPEN_MENU_ID => Some(TrayAction::OpenMain),
        EXIT_MENU_ID => Some(TrayAction::ExitApplication),
        _ => None,
    }
}

pub fn action_for_pointer_input(input: TrayPointerInput) -> Option<TrayAction> {
    match input {
        TrayPointerInput::LeftButtonReleased => Some(TrayAction::OpenMain),
        TrayPointerInput::Other => None,
    }
}

pub fn pointer_input_for(button: MouseButton, button_state: MouseButtonState) -> TrayPointerInput {
    if button == MouseButton::Left && button_state == MouseButtonState::Up {
        TrayPointerInput::LeftButtonReleased
    } else {
        TrayPointerInput::Other
    }
}

pub fn route_window_lifecycle(
    input: WindowLifecycleInput,
    exit_requested: bool,
    close_to_tray: bool,
) -> WindowLifecycleRoute {
    match input {
        WindowLifecycleInput::CloseRequested if !exit_requested && close_to_tray => {
            WindowLifecycleRoute {
                stop_capture: true,
                prevent_close: true,
                hide_main: true,
                exit_app: false,
            }
        }
        // When the user opted out of "close to tray", closing the main window
        // quits the whole application (with the full teardown sequence) instead
        // of hiding it.
        WindowLifecycleInput::CloseRequested if !exit_requested => WindowLifecycleRoute {
            stop_capture: true,
            prevent_close: true,
            hide_main: false,
            exit_app: true,
        },
        WindowLifecycleInput::CloseRequested
        | WindowLifecycleInput::FocusLost
        | WindowLifecycleInput::Destroyed => WindowLifecycleRoute {
            stop_capture: true,
            ..WindowLifecycleRoute::default()
        },
        WindowLifecycleInput::Other => WindowLifecycleRoute::default(),
    }
}

pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let open_item = MenuItem::with_id(app, OPEN_MENU_ID, "打开 OpenDeskTools", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let exit_item = MenuItem::with_id(app, EXIT_MENU_ID, "退出 OpenDeskTools", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open_item, &separator, &exit_item])?;
    let icon = app.default_window_icon().cloned().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "the project default icon is required for the system tray",
        )
    })?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .tooltip(TRAY_TOOLTIP)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            if let Some(action) = action_for_menu_id(event.id().as_ref()) {
                execute_action(app, action);
            }
        })
        .on_tray_icon_event(|tray, event| {
            let input = match event {
                TrayIconEvent::Click {
                    button,
                    button_state,
                    ..
                } => pointer_input_for(button, button_state),
                _ => TrayPointerInput::Other,
            };
            if let Some(action) = action_for_pointer_input(input) {
                execute_action(tray.app_handle(), action);
            }
        })
        .build(app)?;

    Ok(())
}

fn execute_action<R: Runtime>(app: &AppHandle<R>, action: TrayAction) {
    match action {
        TrayAction::OpenMain => {
            if let Err(error) = open_main_window(app) {
                eprintln!("failed to open the main window from the tray: {error}");
            }
        }
        TrayAction::ExitApplication => exit_application(app),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MainWindowWakeStep {
    Show,
    Unminimize,
    Focus,
}

const MAIN_WINDOW_WAKE_STEPS: [MainWindowWakeStep; 3] = [
    MainWindowWakeStep::Show,
    MainWindowWakeStep::Unminimize,
    MainWindowWakeStep::Focus,
];

fn run_main_window_wake<E>(
    mut execute: impl FnMut(MainWindowWakeStep) -> Result<(), E>,
) -> Result<(), E> {
    for step in MAIN_WINDOW_WAKE_STEPS {
        execute(step)?;
    }
    Ok(())
}

pub(crate) fn open_main_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "the main window is unavailable"))?;
    run_main_window_wake(|step| match step {
        MainWindowWakeStep::Show => window.show(),
        MainWindowWakeStep::Unminimize => window.unminimize(),
        MainWindowWakeStep::Focus => window.set_focus(),
    })
}

pub(crate) fn exit_application<R: Runtime>(app: &AppHandle<R>) {
    let Some(lifecycle) = app.try_state::<TrayLifecycle>() else {
        eprintln!("tray exit ignored because TrayLifecycle state is unavailable");
        return;
    };
    if !lifecycle.request_exit() {
        return;
    }

    for step in EXIT_STEPS {
        match step {
            ExitStep::StopClipboardListener => {
                if let Some(runtime) = app.try_state::<ApplicationRuntime>() {
                    if let Err(error) = runtime.clipboard_listener().stop() {
                        eprintln!("failed to stop clipboard listener during exit: {error}");
                    }
                }
            }
            ExitStep::StopCapture => {
                if let Some(runtime) = app.try_state::<ApplicationRuntime>() {
                    if let Err(error) = runtime.hotkey_capture().stop_active() {
                        eprintln!("failed to stop native hotkey capture during exit: {error}");
                    }
                }
            }
            ExitStep::StopKeyboardHook => {
                if let Some(runtime) = app.try_state::<ApplicationRuntime>() {
                    if let Err(error) = runtime.keyboard_hook().shutdown() {
                        eprintln!("failed to stop low-level keyboard hook during exit: {error}");
                    }
                }
            }
            ExitStep::StopSurfaceMonitors => {
                if let Err(error) = clipboard_surface_foreground::stop() {
                    eprintln!("failed to stop clipboard foreground monitor during exit: {error}");
                }
                if let Err(error) = clipboard_surface_pointer::stop() {
                    eprintln!(
                        "failed to stop clipboard outside-pointer monitor during exit: {error}"
                    );
                }
            }
            ExitStep::UnregisterGlobalShortcuts => {
                if let Some(runtime) = app.try_state::<ApplicationRuntime>() {
                    runtime.ordinary_hotkey_latch().clear();
                }
                if let Err(error) = app.global_shortcut().unregister_all() {
                    eprintln!("failed to unregister global shortcuts during exit: {error}");
                }
            }
            ExitStep::RestoreSystemHotkeys => {
                if let Some(runtime) = app.try_state::<ApplicationRuntime>() {
                    if let Err(error) = runtime.system_hotkeys().restore_all() {
                        eprintln!(
                            "failed to restore the DisabledHotkeys registry value during exit: {error}"
                        );
                    }
                }
            }
            ExitStep::ExitApplication => {
                if let Some(runtime) = app.try_state::<ApplicationRuntime>() {
                    if let Err(error) = runtime.surface().clear() {
                        eprintln!("failed to clear clipboard surface during exit: {error}");
                    }
                }
                app.exit(0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_tray_menu_items_route_to_actions() {
        assert_eq!(action_for_menu_id(OPEN_MENU_ID), Some(TrayAction::OpenMain));
        assert_eq!(
            action_for_menu_id(EXIT_MENU_ID),
            Some(TrayAction::ExitApplication)
        );
        assert_eq!(action_for_menu_id("unrelated.menu"), None);
    }

    #[test]
    fn only_left_button_release_opens_the_main_window() {
        assert_eq!(
            pointer_input_for(MouseButton::Left, MouseButtonState::Up),
            TrayPointerInput::LeftButtonReleased
        );
        assert_eq!(
            pointer_input_for(MouseButton::Left, MouseButtonState::Down),
            TrayPointerInput::Other
        );
        assert_eq!(
            pointer_input_for(MouseButton::Right, MouseButtonState::Up),
            TrayPointerInput::Other
        );
        assert_eq!(
            action_for_pointer_input(TrayPointerInput::LeftButtonReleased),
            Some(TrayAction::OpenMain)
        );
        assert_eq!(action_for_pointer_input(TrayPointerInput::Other), None);
    }

    #[test]
    fn close_request_hides_main_window_when_close_to_tray_is_enabled() {
        assert_eq!(
            route_window_lifecycle(WindowLifecycleInput::CloseRequested, false, true),
            WindowLifecycleRoute {
                stop_capture: true,
                prevent_close: true,
                hide_main: true,
                exit_app: false,
            }
        );
    }

    #[test]
    fn close_request_exits_the_app_when_close_to_tray_is_disabled() {
        assert_eq!(
            route_window_lifecycle(WindowLifecycleInput::CloseRequested, false, false),
            WindowLifecycleRoute {
                stop_capture: true,
                prevent_close: true,
                hide_main: false,
                exit_app: true,
            }
        );
    }

    #[test]
    fn close_request_does_not_block_or_hide_during_real_exit() {
        for close_to_tray in [true, false] {
            assert_eq!(
                route_window_lifecycle(WindowLifecycleInput::CloseRequested, true, close_to_tray),
                WindowLifecycleRoute {
                    stop_capture: true,
                    prevent_close: false,
                    hide_main: false,
                    exit_app: false,
                }
            );
        }
    }

    #[test]
    fn focus_loss_and_destroy_are_capture_cleanup_boundaries_only() {
        for input in [
            WindowLifecycleInput::FocusLost,
            WindowLifecycleInput::Destroyed,
        ] {
            assert_eq!(
                route_window_lifecycle(input, false, true),
                WindowLifecycleRoute {
                    stop_capture: true,
                    ..WindowLifecycleRoute::default()
                }
            );
        }
        assert_eq!(
            route_window_lifecycle(WindowLifecycleInput::Other, false, true),
            WindowLifecycleRoute::default()
        );
    }

    #[test]
    fn exit_sequence_releases_native_and_global_hotkeys_before_exit() {
        assert_eq!(
            EXIT_STEPS,
            [
                ExitStep::StopClipboardListener,
                ExitStep::StopCapture,
                ExitStep::StopKeyboardHook,
                ExitStep::StopSurfaceMonitors,
                ExitStep::UnregisterGlobalShortcuts,
                ExitStep::RestoreSystemHotkeys,
                ExitStep::ExitApplication,
            ]
        );
    }

    #[test]
    fn main_window_wake_shows_unminimizes_then_focuses_and_stops_on_error() {
        let mut calls = Vec::new();
        run_main_window_wake::<()>(|step| {
            calls.push(step);
            Ok(())
        })
        .unwrap();
        assert_eq!(calls, MAIN_WINDOW_WAKE_STEPS);

        calls.clear();
        let error = run_main_window_wake(|step| {
            calls.push(step);
            (step != MainWindowWakeStep::Unminimize)
                .then_some(())
                .ok_or("wake failed")
        })
        .unwrap_err();
        assert_eq!(error, "wake failed");
        assert_eq!(
            calls,
            [MainWindowWakeStep::Show, MainWindowWakeStep::Unminimize]
        );
    }

    #[test]
    fn exit_request_is_a_one_way_idempotent_gate() {
        let lifecycle = TrayLifecycle::default();
        assert!(!lifecycle.is_exit_requested());
        assert!(lifecycle.request_exit());
        assert!(lifecycle.is_exit_requested());
        assert!(!lifecycle.request_exit());
    }
}
