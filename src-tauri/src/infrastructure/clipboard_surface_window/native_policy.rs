use super::ClipboardSurfaceWindowError;

pub(super) fn popup_window_style(style: u32) -> u32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
    };
    (style & !(WS_CAPTION | WS_MAXIMIZEBOX | WS_MINIMIZEBOX | WS_SYSMENU | WS_THICKFRAME))
        | WS_POPUP
}

pub(super) fn popup_extended_style(style: u32) -> u32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };
    (style & !WS_EX_APPWINDOW) | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW
}

pub(super) fn child_no_activate_extended_style(style: u32) -> u32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::WS_EX_NOACTIVATE;
    style | WS_EX_NOACTIVATE
}

pub(super) trait WindowLongApi {
    fn get(&mut self, window: usize, index: i32) -> (isize, u32);
    fn set(&mut self, window: usize, index: i32, value: isize) -> (isize, u32);
}

pub(super) struct SystemWindowLongApi;

impl WindowLongApi for SystemWindowLongApi {
    fn get(&mut self, window: usize, index: i32) -> (isize, u32) {
        use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
        use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW;

        unsafe {
            SetLastError(0);
        }
        let value = unsafe { GetWindowLongPtrW(window as _, index) };
        (value, unsafe { GetLastError() })
    }

    fn set(&mut self, window: usize, index: i32, value: isize) -> (isize, u32) {
        use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
        use windows_sys::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW;

        unsafe {
            SetLastError(0);
        }
        let previous = unsafe { SetWindowLongPtrW(window as _, index, value) };
        (previous, unsafe { GetLastError() })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WindowLongUpdate {
    Unchanged,
    Updated,
}

pub(super) fn update_window_long<A: WindowLongApi>(
    api: &mut A,
    window: usize,
    index: i32,
    transform: fn(u32) -> u32,
    get_operation: &'static str,
    set_operation: &'static str,
) -> Result<WindowLongUpdate, ClipboardSurfaceWindowError> {
    let (current, get_error) = api.get(window, index);
    if current == 0 && get_error != 0 {
        return Err(ClipboardSurfaceWindowError::WindowsApiCode {
            operation: get_operation,
            code: get_error,
        });
    }
    let next = transform(current as u32);
    if next == current as u32 {
        return Ok(WindowLongUpdate::Unchanged);
    }
    let (previous, set_error) = api.set(window, index, (next as i32) as isize);
    // SetWindowLongPtrW legitimately returns the previous value, which may be
    // zero. Only a nonzero GetLastError after the call identifies failure.
    if previous == 0 && set_error != 0 {
        return Err(ClipboardSurfaceWindowError::WindowsApiCode {
            operation: set_operation,
            code: set_error,
        });
    }
    Ok(WindowLongUpdate::Updated)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NoActivateChildKind {
    WryWebview,
    ChromiumHost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NoActivateChildPolicy {
    SkipDifferentIdentity,
    SubclassOnly,
    SubclassAndStyle(NoActivateChildKind),
}

pub(super) enum ChildStyleUpdate {
    Applied(WindowLongUpdate),
    Degraded(ClipboardSurfaceWindowError),
}

pub(super) fn update_child_no_activate_style<A: WindowLongApi>(
    api: &mut A,
    window: usize,
) -> ChildStyleUpdate {
    match update_window_long(
        api,
        window,
        windows_sys::Win32::UI::WindowsAndMessaging::GWL_EXSTYLE,
        child_no_activate_extended_style,
        "GetWindowLongPtrW(child GWL_EXSTYLE)",
        "SetWindowLongPtrW(child GWL_EXSTYLE)",
    ) {
        Ok(update) => ChildStyleUpdate::Applied(update),
        Err(error) => ChildStyleUpdate::Degraded(error),
    }
}

pub(super) fn no_activate_child_policy(
    root_process_id: u32,
    root_thread_id: u32,
    process_id: u32,
    thread_id: u32,
    class_name: &str,
) -> NoActivateChildPolicy {
    if process_id != root_process_id || thread_id != root_thread_id {
        return NoActivateChildPolicy::SkipDifferentIdentity;
    }
    match class_name {
        "WRY_WEBVIEW" => NoActivateChildPolicy::SubclassAndStyle(NoActivateChildKind::WryWebview),
        "Chrome_WidgetWin_0" => {
            NoActivateChildPolicy::SubclassAndStyle(NoActivateChildKind::ChromiumHost)
        }
        _ => NoActivateChildPolicy::SubclassOnly,
    }
}

pub(super) fn require_top_level_subclass(
    installed: bool,
) -> Result<(), ClipboardSurfaceWindowError> {
    if installed {
        Ok(())
    } else {
        Err(ClipboardSurfaceWindowError::WindowsApi(
            "SetWindowSubclass(top-level no activate)",
        ))
    }
}
