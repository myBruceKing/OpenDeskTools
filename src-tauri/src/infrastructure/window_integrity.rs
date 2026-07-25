#[cfg(windows)]
mod platform {
    use std::mem::size_of;
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HWND};
    use windows_sys::Win32::Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel,
        TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

    static CURRENT_PROCESS_INTEGRITY: OnceLock<Option<u32>> = OnceLock::new();

    pub fn window_has_higher_integrity(window: usize) -> bool {
        let Some(current) = *CURRENT_PROCESS_INTEGRITY.get_or_init(current_process_integrity)
        else {
            return false;
        };
        window_integrity(window).is_some_and(|foreground| foreground > current)
    }

    fn current_process_integrity() -> Option<u32> {
        let process = unsafe { GetCurrentProcess() };
        integrity_for_process_handle(process)
    }

    fn window_integrity(window: usize) -> Option<u32> {
        if window == 0 {
            return None;
        }
        let mut process_id = 0;
        if unsafe { GetWindowThreadProcessId(window as HWND, &mut process_id) } == 0
            || process_id == 0
        {
            return None;
        }
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
        if process.is_null() {
            return None;
        }
        let result = integrity_for_process_handle(process);
        unsafe {
            CloseHandle(process);
        }
        result
    }

    fn integrity_for_process_handle(process: HANDLE) -> Option<u32> {
        let mut token = std::ptr::null_mut();
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 || token.is_null() {
            return None;
        }
        let result = integrity_for_token(token);
        unsafe {
            CloseHandle(token);
        }
        result
    }

    fn integrity_for_token(token: HANDLE) -> Option<u32> {
        let mut byte_count = 0u32;
        unsafe {
            let _ = GetTokenInformation(
                token,
                TokenIntegrityLevel,
                std::ptr::null_mut(),
                0,
                &mut byte_count,
            );
        }
        if byte_count < size_of::<TOKEN_MANDATORY_LABEL>() as u32 {
            return None;
        }
        let word_count = usize::try_from(byte_count)
            .ok()?
            .div_ceil(size_of::<usize>());
        let mut storage = vec![0usize; word_count];
        if unsafe {
            GetTokenInformation(
                token,
                TokenIntegrityLevel,
                storage.as_mut_ptr().cast(),
                byte_count,
                &mut byte_count,
            )
        } == 0
        {
            return None;
        }
        let label = unsafe { &*storage.as_ptr().cast::<TOKEN_MANDATORY_LABEL>() };
        if label.Label.Sid.is_null() {
            return None;
        }
        let sub_authority_count = unsafe { GetSidSubAuthorityCount(label.Label.Sid) };
        if sub_authority_count.is_null() {
            return None;
        }
        let count = unsafe { *sub_authority_count };
        if count == 0 {
            return None;
        }
        let integrity = unsafe { GetSidSubAuthority(label.Label.Sid, u32::from(count - 1)) };
        (!integrity.is_null()).then(|| unsafe { *integrity })
    }
}

#[cfg(windows)]
pub use platform::window_has_higher_integrity;

#[cfg(not(windows))]
pub fn window_has_higher_integrity(_window: usize) -> bool {
    false
}
