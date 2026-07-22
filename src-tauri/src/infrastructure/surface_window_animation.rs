use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::Duration,
};

use tauri::{Manager, Runtime, WebviewWindow};

pub const SURFACE_EXIT_FADE_DURATION_MS: u64 = 140;
const SURFACE_EXIT_HIDE_DELAY_MS: u64 = 112;

#[derive(Clone, Copy)]
enum FinalHideMode {
    Framework,
    NativeWindow,
}

fn generations() -> &'static Mutex<HashMap<String, u64>> {
    static GENERATIONS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    GENERATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn advance_generation(label: &str) -> u64 {
    let mut generations = generations()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let generation = generations.entry(label.to_owned()).or_default();
    *generation = generation.wrapping_add(1).max(1);
    *generation
}

fn is_current_generation(label: &str, generation: u64) -> bool {
    generations()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(label)
        .is_some_and(|current| *current == generation)
}

/// Cancel a pending exit and restore the document before the native window is
/// shown. The top-level HWND style never changes, avoiding WebView2/DWM
/// composition artifacts on transparent windows.
pub fn prepare_show<R: Runtime>(window: &WebviewWindow<R>) {
    advance_generation(window.label());
    let _ = window.eval(
        "document.documentElement.removeAttribute('data-surface-closing');",
    );
}

/// Start a document-level opacity transition and hide through Tauri after the
/// last frame. A newer show advances the generation and cancels this delayed
/// hide, so a rapid reopen cannot be hidden by a stale timer.
pub fn fade_hide<R: Runtime>(window: &WebviewWindow<R>) -> bool {
    fade_hide_with(window, FinalHideMode::Framework)
}

/// Clipboard surfaces are shown with native SW_SHOWNOACTIVATE, so Tao's
/// cached visibility remains false. Their final hide must use the matching
/// native API instead of the framework no-op produced by that stale cache.
pub fn fade_hide_native<R: Runtime>(window: &WebviewWindow<R>) -> bool {
    fade_hide_with(window, FinalHideMode::NativeWindow)
}

fn fade_hide_with<R: Runtime>(window: &WebviewWindow<R>, mode: FinalHideMode) -> bool {
    let label = window.label().to_owned();
    let generation = advance_generation(&label);
    if window
        .eval("document.documentElement.setAttribute('data-surface-closing','true');")
        .is_err()
    {
        return hide_now(window, mode);
    }

    let delayed_window = window.clone();
    let delayed_app = window.app_handle().clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(SURFACE_EXIT_HIDE_DELAY_MS));
        if !is_current_generation(&label, generation) {
            return;
        }
        let main_thread_window = delayed_window.clone();
        let main_thread_label = label.clone();
        if let Err(dispatch_error) = delayed_app.run_on_main_thread(move || {
            if !is_current_generation(&main_thread_label, generation) {
                return;
            }
            if !hide_now(&main_thread_window, mode) {
                eprintln!("failed to hide {main_thread_label} after the exit transition");
                return;
            }
            let _ = main_thread_window.eval(
                "document.documentElement.removeAttribute('data-surface-closing');",
            );
            #[cfg(debug_assertions)]
            super::debug_qa::trace(format!(
                "surface exit final-hide label={main_thread_label} backend={} visible_after={:?}",
                hide_mode_name(mode),
                is_visible_now(&main_thread_window, mode)
            ));
        }) {
            eprintln!("failed to dispatch {label} exit hide to the main thread: {dispatch_error}");
            let _ = hide_now(&delayed_window, mode);
        }
    });
    true
}

const fn hide_mode_name(mode: FinalHideMode) -> &'static str {
    match mode {
        FinalHideMode::Framework => "framework",
        FinalHideMode::NativeWindow => "native",
    }
}

fn hide_now<R: Runtime>(window: &WebviewWindow<R>, mode: FinalHideMode) -> bool {
    match mode {
        FinalHideMode::Framework => window.hide().is_ok(),
        FinalHideMode::NativeWindow => native_hide(window),
    }
}

fn is_visible_now<R: Runtime>(
    window: &WebviewWindow<R>,
    mode: FinalHideMode,
) -> Result<bool, tauri::Error> {
    match mode {
        FinalHideMode::Framework => window.is_visible(),
        FinalHideMode::NativeWindow => native_is_visible(window),
    }
}

#[cfg(windows)]
fn native_hide<R: Runtime>(window: &WebviewWindow<R>) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindowVisible, ShowWindow, SW_HIDE};

    let Ok(hwnd) = window.hwnd() else {
        return false;
    };
    unsafe {
        ShowWindow(hwnd.0, SW_HIDE);
        IsWindowVisible(hwnd.0) == 0
    }
}

#[cfg(not(windows))]
fn native_hide<R: Runtime>(window: &WebviewWindow<R>) -> bool {
    window.hide().is_ok()
}

#[cfg(windows)]
fn native_is_visible<R: Runtime>(window: &WebviewWindow<R>) -> Result<bool, tauri::Error> {
    use windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible;

    window
        .hwnd()
        .map(|hwnd| unsafe { IsWindowVisible(hwnd.0) != 0 })
}

#[cfg(not(windows))]
fn native_is_visible<R: Runtime>(window: &WebviewWindow<R>) -> Result<bool, tauri::Error> {
    window.is_visible()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advancing_a_surface_generation_invalidates_older_exit_work() {
        let label = "surface-animation-generation-test";
        let first = advance_generation(label);
        let second = advance_generation(label);

        assert!(!is_current_generation(label, first));
        assert!(is_current_generation(label, second));
    }

    #[test]
    fn native_hide_cuts_off_the_transparent_tail_before_host_layers_can_show() {
        assert!(SURFACE_EXIT_HIDE_DELAY_MS < SURFACE_EXIT_FADE_DURATION_MS);
    }
}
