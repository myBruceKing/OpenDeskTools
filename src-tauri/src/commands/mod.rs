#[cfg(windows)]
use tauri::Manager;
use tauri::{AppHandle, Runtime};

pub mod capture;
pub mod clipboard;
pub mod general;
pub mod hotkey;
pub mod overview;
pub mod qr;
pub mod quick_launch;
pub mod theme;

#[cfg(windows)]
pub(crate) fn main_window_handle<R: Runtime>(app: &AppHandle<R>) -> Option<usize> {
    app.get_webview_window("main")
        .and_then(|window| window.hwnd().ok())
        .map(|handle| handle.0 as usize)
        .filter(|handle| *handle != 0)
}

#[cfg(not(windows))]
pub(crate) fn main_window_handle<R: Runtime>(_app: &AppHandle<R>) -> Option<usize> {
    None
}
