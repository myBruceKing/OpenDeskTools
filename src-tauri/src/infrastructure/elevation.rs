use std::ffi::OsString;

use thiserror::Error;

const RESTART_PARENT_ARGUMENT: &str = "--elevated-restart-parent-pid=";

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ElevationError {
    #[error("the elevated restart parent process id is invalid")]
    InvalidParentProcessId,
    #[error("the elevated restart parent process id was provided more than once")]
    DuplicateParentProcessId,
    #[error("the previous OpenDeskTools process did not exit in time")]
    ParentExitTimedOut,
    #[error("Windows elevation was cancelled or unavailable (ShellExecuteW result {0})")]
    LaunchFailed(isize),
    #[error("Windows elevation operation failed: {0}")]
    WindowsApi(&'static str),
    #[cfg(not(windows))]
    #[error("administrator restart is unavailable on this platform")]
    UnsupportedPlatform,
}

pub fn wait_for_restart_parent() -> Result<(), ElevationError> {
    let Some(parent_process_id) = restart_parent_process_id(std::env::args_os())? else {
        return Ok(());
    };
    platform::wait_for_process_exit(parent_process_id)
}

pub fn is_elevated() -> bool {
    platform::is_elevated()
}

pub fn launch_current_as_administrator() -> Result<(), ElevationError> {
    platform::launch_current_as_administrator()
}

fn restart_parent_process_id(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<Option<u32>, ElevationError> {
    let mut parent_process_id = None;
    for argument in arguments.into_iter().skip(1) {
        let Some(value) = argument
            .to_str()
            .and_then(|argument| argument.strip_prefix(RESTART_PARENT_ARGUMENT))
        else {
            continue;
        };
        if parent_process_id.is_some() {
            return Err(ElevationError::DuplicateParentProcessId);
        }
        parent_process_id = Some(
            value
                .parse::<u32>()
                .ok()
                .filter(|process_id| *process_id != 0)
                .ok_or(ElevationError::InvalidParentProcessId)?,
        );
    }
    Ok(parent_process_id)
}

#[cfg(windows)]
mod platform {
    use std::ffi::OsStr;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, WaitForSingleObject,
    };
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    use super::{ElevationError, RESTART_PARENT_ARGUMENT};

    const PARENT_EXIT_TIMEOUT_MS: u32 = 15_000;
    const PROCESS_SYNCHRONIZE_ACCESS: u32 = 0x0010_0000;

    pub fn is_elevated() -> bool {
        let mut token = null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0
            || token.is_null()
        {
            return false;
        }
        let mut elevation: TOKEN_ELEVATION = unsafe { zeroed() };
        let mut byte_count = size_of::<TOKEN_ELEVATION>() as u32;
        let succeeded = unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                (&mut elevation as *mut TOKEN_ELEVATION).cast(),
                byte_count,
                &mut byte_count,
            )
        } != 0;
        unsafe {
            CloseHandle(token);
        }
        succeeded && elevation.TokenIsElevated != 0
    }

    pub fn launch_current_as_administrator() -> Result<(), ElevationError> {
        let executable =
            std::env::current_exe().map_err(|_| ElevationError::WindowsApi("current_exe"))?;
        let executable = wide(executable.as_os_str());
        let operation = wide(OsStr::new("runas"));
        let parameters = wide(OsStr::new(&format!(
            "{RESTART_PARENT_ARGUMENT}{}",
            std::process::id()
        )));
        let result = unsafe {
            ShellExecuteW(
                null_mut(),
                operation.as_ptr(),
                executable.as_ptr(),
                parameters.as_ptr(),
                null(),
                SW_SHOWNORMAL,
            )
        } as isize;
        if result <= 32 {
            Err(ElevationError::LaunchFailed(result))
        } else {
            Ok(())
        }
    }

    pub fn wait_for_process_exit(process_id: u32) -> Result<(), ElevationError> {
        let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE_ACCESS, 0, process_id) };
        if process.is_null() {
            // The parent can finish between argument parsing and OpenProcess.
            return Ok(());
        }
        let wait_result = unsafe { WaitForSingleObject(process, PARENT_EXIT_TIMEOUT_MS) };
        unsafe {
            CloseHandle(process);
        }
        match wait_result {
            WAIT_OBJECT_0 => Ok(()),
            WAIT_TIMEOUT => Err(ElevationError::ParentExitTimedOut),
            _ => Err(ElevationError::WindowsApi("WaitForSingleObject")),
        }
    }

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }
}

#[cfg(not(windows))]
mod platform {
    use super::ElevationError;

    pub fn is_elevated() -> bool {
        false
    }

    pub fn launch_current_as_administrator() -> Result<(), ElevationError> {
        Err(ElevationError::UnsupportedPlatform)
    }

    pub fn wait_for_process_exit(_process_id: u32) -> Result<(), ElevationError> {
        Err(ElevationError::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        std::iter::once(OsString::from("open-desk-tools.exe"))
            .chain(values.iter().map(OsString::from))
            .collect()
    }

    #[test]
    fn restart_parent_argument_is_optional_and_ignores_other_arguments() {
        assert_eq!(restart_parent_process_id(args(&[])).unwrap(), None);
        assert_eq!(
            restart_parent_process_id(args(&["--autostart", "--elevated-restart-parent-pid=42"]))
                .unwrap(),
            Some(42)
        );
    }

    #[test]
    fn restart_parent_argument_rejects_invalid_and_duplicate_values() {
        for value in ["", "0", "-1", "abc"] {
            assert_eq!(
                restart_parent_process_id(args(&[&format!(
                    "--elevated-restart-parent-pid={value}"
                )])),
                Err(ElevationError::InvalidParentProcessId)
            );
        }
        assert_eq!(
            restart_parent_process_id(args(&[
                "--elevated-restart-parent-pid=1",
                "--elevated-restart-parent-pid=2"
            ])),
            Err(ElevationError::DuplicateParentProcessId)
        );
    }
}
