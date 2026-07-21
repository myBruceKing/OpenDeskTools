//! Seamless "launch at login" support through the per-user Windows `Run` key.
//!
//! Windows lets each user register a command to run at sign-in by adding a
//! string value under
//! `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run`. This needs
//! no administrator rights and no separate scheduled task, so it is the least
//! intrusive way to start OpenDeskTools in the background.
//!
//! The registered command carries the [`AUTOSTART_ARGUMENT`] flag so the app can
//! recognise a login launch and stay hidden in the tray (see
//! [`is_autostart_launch`]). The `Run` key value is the single source of truth
//! for whether autostart is enabled: enabling rewrites it to the current
//! executable path (self-healing across moves/updates) and disabling removes it.

use std::path::Path;
use std::sync::{Arc, Mutex};

use thiserror::Error;

/// Command-line flag appended to the autostart command so a login launch can be
/// told apart from a normal double-click.
pub const AUTOSTART_ARGUMENT: &str = "--autostart";

#[cfg(windows)]
const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const RUN_VALUE_NAME: &str = "OpenDeskTools";

#[derive(Debug, Error)]
pub enum AutostartError {
    #[error("autostart state lock is poisoned")]
    LockPoisoned,
    #[error("failed to resolve the executable path for the autostart command: {0}")]
    CurrentExecutable(#[source] std::io::Error),
    #[cfg(windows)]
    #[error("Windows registry operation {operation} failed with status {status}")]
    Registry {
        operation: &'static str,
        status: u32,
    },
}

/// Returns `true` when the process was started with the [`AUTOSTART_ARGUMENT`]
/// flag, meaning it should start hidden in the tray instead of showing the main
/// window.
pub fn is_autostart_launch<I, S>(arguments: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    arguments
        .into_iter()
        .any(|argument| argument.as_ref() == std::ffi::OsStr::new(AUTOSTART_ARGUMENT))
}

/// Builds the autostart command string (`"<exe>" --autostart`) for the given
/// executable path. Quoting keeps paths with spaces intact.
fn autostart_command(executable: &Path) -> String {
    format!("\"{}\" {}", executable.display(), AUTOSTART_ARGUMENT)
}

/// Abstraction over the `Run` registry value so the manager can be exercised
/// without touching the real registry.
pub trait AutostartRegistry: Send + Sync {
    /// Reads the current command, or `None` when the value is absent/empty.
    fn read_command(&self) -> Result<Option<String>, AutostartError>;
    fn write_command(&self, command: &str) -> Result<(), AutostartError>;
    fn remove(&self) -> Result<(), AutostartError>;
}

/// Coordinates the `Run` key value with the desired autostart preference.
pub struct AutostartManager {
    registry: Arc<dyn AutostartRegistry>,
    desired_command: String,
    lock: Mutex<()>,
}

impl std::fmt::Debug for AutostartManager {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AutostartManager")
            .field("desired_command", &self.desired_command)
            .finish()
    }
}

impl AutostartManager {
    pub fn new(registry: Arc<dyn AutostartRegistry>, desired_command: String) -> Self {
        Self {
            registry,
            desired_command,
            lock: Mutex::new(()),
        }
    }

    /// Builds the manager that talks to the real per-user `Run` key on Windows
    /// and to a harmless no-op registry elsewhere. The command targets the
    /// currently running executable so autostart self-heals after updates/moves.
    pub fn for_system() -> Result<Self, AutostartError> {
        let executable = std::env::current_exe().map_err(AutostartError::CurrentExecutable)?;
        let desired_command = autostart_command(&executable);
        #[cfg(windows)]
        let registry: Arc<dyn AutostartRegistry> = Arc::new(SystemRunKeyRegistry);
        #[cfg(not(windows))]
        let registry: Arc<dyn AutostartRegistry> = Arc::new(NoopAutostartRegistry);
        Ok(Self::new(registry, desired_command))
    }

    /// Whether a login autostart entry is currently registered.
    pub fn is_enabled(&self) -> Result<bool, AutostartError> {
        let _guard = self.lock.lock().map_err(|_| AutostartError::LockPoisoned)?;
        Ok(self.registry.read_command()?.is_some())
    }

    /// Enables or disables autostart. Returns `true` when the registry value was
    /// changed by this call.
    pub fn set(&self, enabled: bool) -> Result<bool, AutostartError> {
        let _guard = self.lock.lock().map_err(|_| AutostartError::LockPoisoned)?;
        let current = self.registry.read_command()?;
        if enabled {
            if current.as_deref() == Some(self.desired_command.as_str()) {
                return Ok(false);
            }
            self.registry.write_command(&self.desired_command)?;
            Ok(true)
        } else {
            if current.is_none() {
                return Ok(false);
            }
            self.registry.remove()?;
            Ok(true)
        }
    }

    /// Rewrites the command to the current executable when autostart is already
    /// enabled but the stored path drifted (moved/updated executable). No-op when
    /// autostart is disabled so a login launch never re-enables itself.
    pub fn sync_if_enabled(&self) -> Result<bool, AutostartError> {
        let _guard = self.lock.lock().map_err(|_| AutostartError::LockPoisoned)?;
        match self.registry.read_command()? {
            Some(current) if current != self.desired_command => {
                self.registry.write_command(&self.desired_command)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

/// No-op registry used on non-Windows platforms so the runtime can be built.
#[cfg(not(windows))]
struct NoopAutostartRegistry;

#[cfg(not(windows))]
impl AutostartRegistry for NoopAutostartRegistry {
    fn read_command(&self) -> Result<Option<String>, AutostartError> {
        Ok(None)
    }

    fn write_command(&self, _command: &str) -> Result<(), AutostartError> {
        Ok(())
    }

    fn remove(&self) -> Result<(), AutostartError> {
        Ok(())
    }
}

/// Registry implementation backed by the real per-user `Run` key.
#[cfg(windows)]
struct SystemRunKeyRegistry;

#[cfg(windows)]
struct HkeyGuard(windows_sys::Win32::System::Registry::HKEY);

#[cfg(windows)]
impl Drop for HkeyGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Registry::RegCloseKey(self.0);
        }
    }
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn registry_error(operation: &'static str, status: u32) -> AutostartError {
    AutostartError::Registry { operation, status }
}

#[cfg(windows)]
impl AutostartRegistry for SystemRunKeyRegistry {
    fn read_command(&self) -> Result<Option<String>, AutostartError> {
        use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
        use windows_sys::Win32::System::Registry::{
            RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_SZ,
        };

        let subkey = wide(RUN_SUBKEY);
        let value_name = wide(RUN_VALUE_NAME);

        let mut key: HKEY = std::ptr::null_mut();
        let status =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_READ, &mut key) };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != ERROR_SUCCESS {
            return Err(registry_error("RegOpenKeyExW", status));
        }
        let key = HkeyGuard(key);

        let mut value_type: u32 = 0;
        let mut byte_len: u32 = 0;
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                value_name.as_ptr(),
                std::ptr::null(),
                &mut value_type,
                std::ptr::null_mut(),
                &mut byte_len,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != ERROR_SUCCESS {
            return Err(registry_error("RegQueryValueExW", status));
        }
        if value_type != REG_SZ || byte_len == 0 {
            return Ok(None);
        }

        let mut buffer: Vec<u16> = vec![0u16; (byte_len as usize).div_ceil(2)];
        let mut buffer_bytes: u32 = (buffer.len() * std::mem::size_of::<u16>()) as u32;
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                value_name.as_ptr(),
                std::ptr::null(),
                &mut value_type,
                buffer.as_mut_ptr() as *mut u8,
                &mut buffer_bytes,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(registry_error("RegQueryValueExW", status));
        }

        let units = (buffer_bytes as usize) / std::mem::size_of::<u16>();
        let slice = &buffer[..units.min(buffer.len())];
        let end = slice
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(slice.len());
        let command = String::from_utf16_lossy(&slice[..end]);
        if command.is_empty() {
            Ok(None)
        } else {
            Ok(Some(command))
        }
    }

    fn write_command(&self, command: &str) -> Result<(), AutostartError> {
        use windows_sys::Win32::Foundation::ERROR_SUCCESS;
        use windows_sys::Win32::System::Registry::{
            RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ,
        };

        let subkey = wide(RUN_SUBKEY);
        let value_name = wide(RUN_VALUE_NAME);

        // The `Run` key ships with every Windows profile, so a plain open with
        // write access is sufficient and avoids the security-attributes surface
        // required by `RegCreateKeyExW`.
        let mut key: HKEY = std::ptr::null_mut();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                0,
                KEY_SET_VALUE,
                &mut key,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(registry_error("RegOpenKeyExW", status));
        }
        let key = HkeyGuard(key);

        let data = wide(command);
        let data_bytes = (data.len() * std::mem::size_of::<u16>()) as u32;
        let status = unsafe {
            RegSetValueExW(
                key.0,
                value_name.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr() as *const u8,
                data_bytes,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(registry_error("RegSetValueExW", status));
        }
        Ok(())
    }

    fn remove(&self) -> Result<(), AutostartError> {
        use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
        use windows_sys::Win32::System::Registry::{
            RegDeleteValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
        };

        let subkey = wide(RUN_SUBKEY);
        let value_name = wide(RUN_VALUE_NAME);

        let mut key: HKEY = std::ptr::null_mut();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                0,
                KEY_SET_VALUE,
                &mut key,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        if status != ERROR_SUCCESS {
            return Err(registry_error("RegOpenKeyExW", status));
        }
        let key = HkeyGuard(key);

        let status = unsafe { RegDeleteValueW(key.0, value_name.as_ptr()) };
        if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(registry_error("RegDeleteValueW", status))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct FakeRegistry {
        value: StdMutex<Option<String>>,
        writes: StdMutex<Vec<String>>,
        removes: StdMutex<u32>,
    }

    impl FakeRegistry {
        fn with_value(value: &str) -> Self {
            let registry = FakeRegistry::default();
            *registry.value.lock().unwrap() = Some(value.to_owned());
            registry
        }

        fn snapshot(&self) -> Option<String> {
            self.value.lock().unwrap().clone()
        }

        fn write_count(&self) -> usize {
            self.writes.lock().unwrap().len()
        }

        fn remove_count(&self) -> u32 {
            *self.removes.lock().unwrap()
        }
    }

    impl AutostartRegistry for FakeRegistry {
        fn read_command(&self) -> Result<Option<String>, AutostartError> {
            Ok(self.value.lock().unwrap().clone())
        }

        fn write_command(&self, command: &str) -> Result<(), AutostartError> {
            *self.value.lock().unwrap() = Some(command.to_owned());
            self.writes.lock().unwrap().push(command.to_owned());
            Ok(())
        }

        fn remove(&self) -> Result<(), AutostartError> {
            *self.value.lock().unwrap() = None;
            *self.removes.lock().unwrap() += 1;
            Ok(())
        }
    }

    const COMMAND: &str = "\"C:\\Apps\\OpenDeskTools.exe\" --autostart";

    fn manager(registry: Arc<FakeRegistry>) -> AutostartManager {
        AutostartManager::new(registry, COMMAND.to_owned())
    }

    #[test]
    fn autostart_command_quotes_paths_and_appends_the_flag() {
        let command = autostart_command(Path::new(r"C:\Program Files\OpenDeskTools\app.exe"));
        assert_eq!(
            command,
            "\"C:\\Program Files\\OpenDeskTools\\app.exe\" --autostart"
        );
    }

    #[test]
    fn is_autostart_launch_only_matches_the_flag() {
        assert!(is_autostart_launch([
            OsString::from("OpenDeskTools.exe"),
            OsString::from("--autostart"),
        ]));
        assert!(!is_autostart_launch([
            OsString::from("OpenDeskTools.exe"),
            OsString::from("--data-dir"),
            OsString::from("data"),
        ]));
        assert!(!is_autostart_launch([OsString::from("OpenDeskTools.exe")]));
    }

    #[test]
    fn is_enabled_reflects_registry_presence() {
        let registry = Arc::new(FakeRegistry::default());
        let manager = manager(registry.clone());
        assert!(!manager.is_enabled().unwrap());

        manager.set(true).unwrap();
        assert!(manager.is_enabled().unwrap());
    }

    #[test]
    fn enabling_writes_the_desired_command_once_and_is_idempotent() {
        let registry = Arc::new(FakeRegistry::default());
        let manager = manager(registry.clone());

        assert!(manager.set(true).unwrap());
        assert_eq!(registry.snapshot().as_deref(), Some(COMMAND));

        assert!(!manager.set(true).unwrap());
        assert_eq!(registry.write_count(), 1);
    }

    #[test]
    fn disabling_removes_the_value_once_and_is_idempotent() {
        let registry = Arc::new(FakeRegistry::with_value(COMMAND));
        let manager = manager(registry.clone());

        assert!(manager.set(false).unwrap());
        assert_eq!(registry.snapshot(), None);
        assert_eq!(registry.remove_count(), 1);

        assert!(!manager.set(false).unwrap());
        assert_eq!(registry.remove_count(), 1);
    }

    #[test]
    fn enabling_rewrites_a_stale_command_path() {
        let registry = Arc::new(FakeRegistry::with_value(
            "\"C:\\Old\\OpenDeskTools.exe\" --autostart",
        ));
        let manager = manager(registry.clone());

        assert!(manager.set(true).unwrap());
        assert_eq!(registry.snapshot().as_deref(), Some(COMMAND));
    }

    /// Deterministic real-registry re-verification entry: exercises the actual
    /// Win32 `HKCU\...\Run` write/read/remove path used by the autostart toggle,
    /// then restores any pre-existing value. Ignored by default so normal test
    /// runs never touch the machine registry; run explicitly with
    /// `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored real_run_key`.
    #[cfg(windows)]
    #[test]
    #[ignore = "mutates the real HKCU Run key; run explicitly for live verification"]
    fn real_run_key_round_trips_write_read_and_remove() {
        let registry = SystemRunKeyRegistry;
        let restore = registry
            .read_command()
            .expect("initial read should succeed");

        registry
            .write_command(COMMAND)
            .expect("write should succeed");
        assert_eq!(registry.read_command().unwrap().as_deref(), Some(COMMAND));

        registry.remove().expect("remove should succeed");
        assert_eq!(registry.read_command().unwrap(), None);

        // Restore any value that existed before the test.
        if let Some(previous) = restore {
            registry.write_command(&previous).unwrap();
        }
    }

    #[test]
    fn sync_if_enabled_heals_stale_path_but_leaves_disabled_untouched() {
        let disabled = Arc::new(FakeRegistry::default());
        let disabled_manager = manager(disabled.clone());
        assert!(!disabled_manager.sync_if_enabled().unwrap());
        assert_eq!(disabled.snapshot(), None);

        let stale = Arc::new(FakeRegistry::with_value(
            "\"C:\\Old\\OpenDeskTools.exe\" --autostart",
        ));
        let stale_manager = manager(stale.clone());
        assert!(stale_manager.sync_if_enabled().unwrap());
        assert_eq!(stale.snapshot().as_deref(), Some(COMMAND));

        // Already current: no rewrite.
        assert!(!stale_manager.sync_if_enabled().unwrap());
        assert_eq!(stale.write_count(), 1);
    }
}
