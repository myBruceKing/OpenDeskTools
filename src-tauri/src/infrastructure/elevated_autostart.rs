use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use thiserror::Error;

use super::autostart::AUTOSTART_ARGUMENT;
use super::elevation::{self, ElevationError};

const CONFIGURE_ARGUMENT_PREFIX: &str = "--configure-elevated-autostart=";
const ENABLE_ARGUMENT: &str = "--configure-elevated-autostart=enable";
const DISABLE_ARGUMENT: &str = "--configure-elevated-autostart=disable";
const TASK_NAME: &str = "OpenDeskTools Elevated Autostart";
const WAKE_REQUEST_FILE: &str = "OpenDeskTools-elevated-wake.request";
const WAKE_REQUEST_MAX_AGE: Duration = Duration::from_secs(30);
const HELPER_EXIT_NOT_ELEVATED: i32 = 10;
const HELPER_EXIT_CONFIGURATION_FAILED: i32 = 11;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigurationRequest {
    Enable,
    Disable,
}

#[derive(Debug, Error)]
pub enum ElevatedAutostartError {
    #[error("administrator authorization failed: {0}")]
    Elevation(#[from] ElevationError),
    #[error("failed to start Windows Task Scheduler: {0}")]
    TaskSchedulerStart(#[source] std::io::Error),
    #[error("Windows Task Scheduler rejected the elevated autostart configuration")]
    TaskSchedulerRejected,
    #[error("the current Windows user identity is unavailable")]
    UserIdentityUnavailable,
    #[error("failed to prepare the elevated wake request: {0}")]
    WakeRequest(#[source] std::io::Error),
    #[error("failed to prepare the elevated task definition: {0}")]
    TaskDefinition(#[source] std::io::Error),
    #[error("the Windows-generated task definition is missing {0}")]
    InvalidTaskDefinition(&'static str),
    #[cfg(not(windows))]
    #[error("elevated autostart is unavailable on this platform")]
    UnsupportedPlatform,
}

/// Redirects a normal, non-elevated double-click through the already-authorized
/// task. The task starts a short-lived elevated peer that can cross the
/// integrity boundary and notify the existing elevated single instance.
pub fn redirect_normal_launch(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<bool, ElevatedAutostartError> {
    let arguments = arguments.into_iter().collect::<Vec<_>>();
    if elevation::is_elevated()
        || arguments
            .iter()
            .any(|argument| argument == OsStr::new(AUTOSTART_ARGUMENT))
        || parse_configuration_request(arguments.clone())
            .ok()
            .flatten()
            .is_some()
        || !platform::task_exists()?
    {
        return Ok(false);
    }
    let request = wake_request_path();
    fs::write(&request, std::process::id().to_string())
        .map_err(ElevatedAutostartError::WakeRequest)?;
    if let Err(error) = platform::run_task() {
        let _ = fs::remove_file(&request);
        return Err(error);
    }
    Ok(true)
}

/// Consumes a recent wake request before the single-instance plugin runs. If
/// no elevated instance exists, this lets the task-started process show its own
/// main window instead of remaining hidden as a login launch.
pub fn consume_wake_request() -> bool {
    let request = wake_request_path();
    let recent = fs::metadata(&request)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age <= WAKE_REQUEST_MAX_AGE);
    let _ = fs::remove_file(request);
    recent
}

#[derive(Debug)]
pub struct ElevatedAutostartManager {
    enabled: AtomicBool,
}

impl ElevatedAutostartManager {
    pub fn for_system() -> Result<Self, ElevatedAutostartError> {
        #[cfg(test)]
        let enabled = false;
        #[cfg(not(test))]
        let enabled = platform::task_exists()?;
        Ok(Self {
            enabled: AtomicBool::new(enabled),
        })
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    pub fn set(&self, enabled: bool) -> Result<(), ElevatedAutostartError> {
        let argument = if enabled {
            OsStr::new(ENABLE_ARGUMENT)
        } else {
            OsStr::new(DISABLE_ARGUMENT)
        };
        elevation::run_current_as_administrator_and_wait(argument)?;
        self.enabled.store(enabled, Ordering::Release);
        Ok(())
    }
}

/// Handles the short-lived, explicitly elevated configuration mode before
/// Tauri and the single-instance plugin are initialized. Returns `None` for a
/// normal application launch and an exit code for the helper invocation.
pub fn configuration_exit_code(arguments: impl IntoIterator<Item = OsString>) -> Option<i32> {
    let request = match parse_configuration_request(arguments) {
        Ok(request) => request?,
        Err(()) => return Some(HELPER_EXIT_CONFIGURATION_FAILED),
    };
    if !elevation::is_elevated() {
        return Some(HELPER_EXIT_NOT_ELEVATED);
    }
    let executable = match std::env::current_exe() {
        Ok(executable) => executable,
        Err(_) => return Some(HELPER_EXIT_CONFIGURATION_FAILED),
    };
    let result = match request {
        ConfigurationRequest::Enable => platform::create_task(&executable),
        ConfigurationRequest::Disable => platform::delete_task(),
    };
    Some(if result.is_ok() {
        0
    } else {
        HELPER_EXIT_CONFIGURATION_FAILED
    })
}

fn parse_configuration_request(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<Option<ConfigurationRequest>, ()> {
    let mut request = None;
    for argument in arguments.into_iter().skip(1) {
        let Some(value) = argument
            .to_str()
            .and_then(|argument| argument.strip_prefix(CONFIGURE_ARGUMENT_PREFIX))
        else {
            continue;
        };
        if request.is_some() {
            return Err(());
        }
        request = Some(match value {
            "enable" => ConfigurationRequest::Enable,
            "disable" => ConfigurationRequest::Disable,
            _ => return Err(()),
        });
    }
    Ok(request)
}

fn scheduled_action(executable: &Path) -> String {
    format!("\"{}\" {AUTOSTART_ARGUMENT}", executable.display())
}

fn patch_exported_task_xml(xml: &str) -> Result<String, ElevatedAutostartError> {
    const POLICY_OPEN: &str = "<MultipleInstancesPolicy>";
    const POLICY_CLOSE: &str = "</MultipleInstancesPolicy>";
    const LIMIT_OPEN: &str = "<ExecutionTimeLimit>";
    const LIMIT_CLOSE: &str = "</ExecutionTimeLimit>";
    const SETTINGS_CLOSE: &str = "</Settings>";

    let xml = replace_element_value(
        xml,
        POLICY_OPEN,
        POLICY_CLOSE,
        "Parallel",
        "MultipleInstancesPolicy",
    )?;
    if xml.contains(LIMIT_OPEN) {
        replace_element_value(&xml, LIMIT_OPEN, LIMIT_CLOSE, "PT0S", "ExecutionTimeLimit")
    } else {
        let insertion = format!("    {LIMIT_OPEN}PT0S{LIMIT_CLOSE}\r\n  {SETTINGS_CLOSE}");
        if !xml.contains(SETTINGS_CLOSE) {
            return Err(ElevatedAutostartError::InvalidTaskDefinition("Settings"));
        }
        Ok(xml.replacen(SETTINGS_CLOSE, &insertion, 1))
    }
}

fn replace_element_value(
    xml: &str,
    open: &str,
    close: &str,
    value: &str,
    element: &'static str,
) -> Result<String, ElevatedAutostartError> {
    let start = xml
        .find(open)
        .map(|index| index + open.len())
        .ok_or(ElevatedAutostartError::InvalidTaskDefinition(element))?;
    let end = xml[start..]
        .find(close)
        .map(|index| start + index)
        .ok_or(ElevatedAutostartError::InvalidTaskDefinition(element))?;
    let mut patched = String::with_capacity(xml.len() + value.len());
    patched.push_str(&xml[..start]);
    patched.push_str(value);
    patched.push_str(&xml[end..]);
    Ok(patched)
}

fn wake_request_path() -> std::path::PathBuf {
    std::env::temp_dir().join(WAKE_REQUEST_FILE)
}

#[cfg(windows)]
mod platform {
    use std::ffi::OsString;
    use std::fs;
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, ExitStatus, Output};

    use windows_sys::Win32::Security::Authentication::Identity::{
        GetUserNameExW, NameSamCompatible,
    };
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    use super::{patch_exported_task_xml, scheduled_action, ElevatedAutostartError, TASK_NAME};

    struct TemporaryFile {
        path: std::path::PathBuf,
    }

    impl TemporaryFile {
        fn new(path: std::path::PathBuf) -> Self {
            Self { path }
        }

        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }

    impl Drop for TemporaryFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    pub fn task_exists() -> Result<bool, ElevatedAutostartError> {
        let status = task_command()
            .args(["/Query", "/TN", TASK_NAME])
            .status()
            .map_err(ElevatedAutostartError::TaskSchedulerStart)?;
        Ok(status.success())
    }

    pub fn create_task(executable: &std::path::Path) -> Result<(), ElevatedAutostartError> {
        let user = current_user_identity()?;
        let action = scheduled_action(executable);
        let base_status = task_command()
            .args([
                OsString::from("/Create"),
                OsString::from("/TN"),
                OsString::from(TASK_NAME),
                OsString::from("/TR"),
                OsString::from(action),
                OsString::from("/SC"),
                OsString::from("ONLOGON"),
                OsString::from("/RU"),
                OsString::from(user),
                OsString::from("/RL"),
                OsString::from("HIGHEST"),
                OsString::from("/IT"),
                OsString::from("/F"),
            ])
            .status()
            .map_err(ElevatedAutostartError::TaskSchedulerStart)?;
        require_success(base_status)?;

        let exported = task_command()
            .args(["/Query", "/TN", TASK_NAME, "/XML"])
            .output()
            .map_err(ElevatedAutostartError::TaskSchedulerStart)?;
        require_output_success(&exported)?;
        let xml = decode_task_xml(&exported.stdout);
        let xml = patch_exported_task_xml(&xml)?;
        let xml_path = TemporaryFile::new(std::env::temp_dir().join(format!(
            "OpenDeskTools-elevated-autostart-{}.xml",
            std::process::id()
        )));
        fs::write(xml_path.path(), encode_utf16le(&xml))
            .map_err(ElevatedAutostartError::TaskDefinition)?;
        let status = task_command()
            .args([
                OsString::from("/Create"),
                OsString::from("/TN"),
                OsString::from(TASK_NAME),
                OsString::from("/XML"),
                xml_path.path().as_os_str().to_owned(),
                OsString::from("/F"),
            ])
            .status()
            .map_err(ElevatedAutostartError::TaskSchedulerStart)?;
        require_success(status)
    }

    pub fn delete_task() -> Result<(), ElevatedAutostartError> {
        if !task_exists()? {
            return Ok(());
        }
        let status = task_command()
            .args(["/Delete", "/TN", TASK_NAME, "/F"])
            .status()
            .map_err(ElevatedAutostartError::TaskSchedulerStart)?;
        require_success(status)
    }

    pub fn run_task() -> Result<(), ElevatedAutostartError> {
        let status = task_command()
            .args(["/Run", "/TN", TASK_NAME])
            .status()
            .map_err(ElevatedAutostartError::TaskSchedulerStart)?;
        require_success(status)
    }

    fn require_success(status: ExitStatus) -> Result<(), ElevatedAutostartError> {
        if status.success() {
            Ok(())
        } else {
            Err(ElevatedAutostartError::TaskSchedulerRejected)
        }
    }

    fn require_output_success(output: &Output) -> Result<(), ElevatedAutostartError> {
        require_success(output.status)
    }

    fn decode_task_xml(bytes: &[u8]) -> String {
        if bytes.starts_with(&[0xff, 0xfe]) {
            let units = bytes[2..]
                .chunks_exact(2)
                .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
                .collect::<Vec<_>>();
            String::from_utf16_lossy(&units)
        } else {
            String::from_utf8_lossy(bytes).into_owned()
        }
    }

    fn encode_utf16le(value: &str) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(value.len() * 2 + 2);
        bytes.extend_from_slice(&[0xff, 0xfe]);
        for unit in value.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    fn task_command() -> Command {
        let mut command = Command::new(system_directory().join("schtasks.exe"));
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }

    fn system_directory() -> std::path::PathBuf {
        let mut buffer = vec![0_u16; 512];
        let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
        if length == 0 || length as usize >= buffer.len() {
            return std::path::PathBuf::from(r"C:\Windows\System32");
        }
        std::path::PathBuf::from(OsString::from_wide(&buffer[..length as usize]))
    }

    fn current_user_identity() -> Result<String, ElevatedAutostartError> {
        let mut buffer = vec![0_u16; 256];
        let mut length = buffer.len() as u32;
        let mut succeeded =
            unsafe { GetUserNameExW(NameSamCompatible, buffer.as_mut_ptr(), &mut length) } != 0;
        if !succeeded && length as usize > buffer.len() {
            buffer.resize(length as usize, 0);
            succeeded =
                unsafe { GetUserNameExW(NameSamCompatible, buffer.as_mut_ptr(), &mut length) } != 0;
        }
        if !succeeded {
            return Err(ElevatedAutostartError::UserIdentityUnavailable);
        }
        let end = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(length as usize)
            .min(buffer.len());
        let identity = String::from_utf16_lossy(&buffer[..end]);
        if identity.trim().is_empty() {
            Err(ElevatedAutostartError::UserIdentityUnavailable)
        } else {
            Ok(identity)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::TemporaryFile;

        fn create_temporary_file_then_fail(path: std::path::PathBuf) -> Result<(), &'static str> {
            let file = TemporaryFile::new(path);
            std::fs::write(file.path(), b"task definition")
                .expect("temporary task definition should be writable");
            Err("simulated task scheduler failure")
        }

        #[test]
        fn temporary_file_is_removed_when_scope_returns_early() {
            let directory = tempfile::tempdir().expect("temporary directory should initialize");
            let path = directory.path().join("elevated-autostart.xml");

            let result = create_temporary_file_then_fail(path.clone());

            assert_eq!(result, Err("simulated task scheduler failure"));
            assert!(!path.exists());
        }

        #[test]
        fn temporary_file_is_removed_after_normal_scope_completion() {
            let directory = tempfile::tempdir().expect("temporary directory should initialize");
            let path = directory.path().join("elevated-autostart.xml");

            {
                let file = TemporaryFile::new(path.clone());
                std::fs::write(file.path(), b"task definition")
                    .expect("temporary task definition should be writable");
                assert!(path.exists());
            }

            assert!(!path.exists());
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::ElevatedAutostartError;

    pub fn task_exists() -> Result<bool, ElevatedAutostartError> {
        Ok(false)
    }

    pub fn create_task(_executable: &std::path::Path) -> Result<(), ElevatedAutostartError> {
        Err(ElevatedAutostartError::UnsupportedPlatform)
    }

    pub fn delete_task() -> Result<(), ElevatedAutostartError> {
        Err(ElevatedAutostartError::UnsupportedPlatform)
    }

    pub fn run_task() -> Result<(), ElevatedAutostartError> {
        Err(ElevatedAutostartError::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        std::iter::once(OsString::from("OpenDeskTools.exe"))
            .chain(values.iter().map(OsString::from))
            .collect()
    }

    #[test]
    fn helper_request_is_optional_and_strict() {
        assert_eq!(parse_configuration_request(args(&[])), Ok(None));
        assert_eq!(
            parse_configuration_request(args(&[ENABLE_ARGUMENT])),
            Ok(Some(ConfigurationRequest::Enable))
        );
        assert_eq!(
            parse_configuration_request(args(&[DISABLE_ARGUMENT])),
            Ok(Some(ConfigurationRequest::Disable))
        );
        assert!(
            parse_configuration_request(args(&["--configure-elevated-autostart=unknown"])).is_err()
        );
        assert!(parse_configuration_request(args(&[ENABLE_ARGUMENT, DISABLE_ARGUMENT])).is_err());
    }

    #[test]
    fn scheduled_action_quotes_the_executable_and_preserves_autostart_semantics() {
        assert_eq!(
            scheduled_action(Path::new(r"D:\Tools\OpenDeskTools\OpenDeskTools.exe")),
            r#""D:\Tools\OpenDeskTools\OpenDeskTools.exe" --autostart"#
        );
    }

    #[test]
    fn exported_task_is_patched_without_rebuilding_windows_owned_fields() {
        let xml = patch_exported_task_xml(
            r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Principals>
    <Principal id="Author">
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <IdleSettings />
  </Settings>
  <Actions Context="Author">
    <Exec><Arguments>--autostart</Arguments></Exec>
  </Actions>
</Task>"#,
        )
        .expect("Windows-exported task should be patchable");
        assert!(xml.contains("<LogonType>InteractiveToken</LogonType>"));
        assert!(xml.contains("<RunLevel>HighestAvailable</RunLevel>"));
        assert!(xml.contains("<MultipleInstancesPolicy>Parallel</MultipleInstancesPolicy>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(xml.contains("<Arguments>--autostart</Arguments>"));
    }

    #[test]
    fn existing_execution_limit_is_replaced() {
        let xml = patch_exported_task_xml(
            "<Task><Settings><MultipleInstancesPolicy>Queue</MultipleInstancesPolicy>\
             <ExecutionTimeLimit>PT72H</ExecutionTimeLimit></Settings></Task>",
        )
        .expect("existing settings should be replaced");
        assert!(xml.contains("<MultipleInstancesPolicy>Parallel</MultipleInstancesPolicy>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(!xml.contains("PT72H"));
    }
}
