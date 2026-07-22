//! Persistent preference for the managed application data root.
//!
//! The preference deliberately lives outside the SQLite database that it
//! selects.  On Windows it uses the current-user registry, so a copied data
//! directory can become active on the next launch without relying on a file in
//! the old directory that is about to be retired.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use thiserror::Error;

#[cfg(windows)]
const DATA_DIRECTORY_SUBKEY: &str = r"Software\OpenDeskTools";
#[cfg(windows)]
const DATA_DIRECTORY_VALUE: &str = "DataDirectory";

#[derive(Debug, Error)]
pub enum DataDirectoryPreferenceError {
    #[error("data directory preference lock is poisoned")]
    LockPoisoned,
    #[cfg(windows)]
    #[error("Windows registry operation {operation} failed with status {status}")]
    Registry {
        operation: &'static str,
        status: u32,
    },
}

trait DataDirectoryRegistry: Send + Sync {
    fn read(&self) -> Result<Option<PathBuf>, DataDirectoryPreferenceError>;
    fn write(&self, directory: &Path) -> Result<(), DataDirectoryPreferenceError>;
}

/// Serializes access to the per-user data-root preference.
pub struct DataDirectoryPreference {
    registry: Arc<dyn DataDirectoryRegistry>,
    lock: Mutex<()>,
}

impl std::fmt::Debug for DataDirectoryPreference {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DataDirectoryPreference")
            .finish_non_exhaustive()
    }
}

impl DataDirectoryPreference {
    pub fn for_system() -> Self {
        #[cfg(windows)]
        let registry: Arc<dyn DataDirectoryRegistry> = Arc::new(SystemDataDirectoryRegistry);
        #[cfg(not(windows))]
        let registry: Arc<dyn DataDirectoryRegistry> = Arc::new(NoopDataDirectoryRegistry);
        Self {
            registry,
            lock: Mutex::new(()),
        }
    }

    pub fn read(&self) -> Result<Option<PathBuf>, DataDirectoryPreferenceError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| DataDirectoryPreferenceError::LockPoisoned)?;
        self.registry.read()
    }

    pub fn set(&self, directory: &Path) -> Result<(), DataDirectoryPreferenceError> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| DataDirectoryPreferenceError::LockPoisoned)?;
        self.registry.write(directory)
    }
}

#[cfg(not(windows))]
#[derive(Debug)]
struct NoopDataDirectoryRegistry;

#[cfg(not(windows))]
impl DataDirectoryRegistry for NoopDataDirectoryRegistry {
    fn read(&self) -> Result<Option<PathBuf>, DataDirectoryPreferenceError> {
        Ok(None)
    }

    fn write(&self, _directory: &Path) -> Result<(), DataDirectoryPreferenceError> {
        Ok(())
    }
}

#[cfg(windows)]
#[derive(Debug)]
struct SystemDataDirectoryRegistry;

#[cfg(windows)]
struct HkeyGuard(windows_sys::Win32::System::Registry::HKEY);

#[cfg(windows)]
impl Drop for HkeyGuard {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::System::Registry::RegCloseKey(self.0) };
    }
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn registry_error(operation: &'static str, status: u32) -> DataDirectoryPreferenceError {
    DataDirectoryPreferenceError::Registry { operation, status }
}

#[cfg(windows)]
impl DataDirectoryRegistry for SystemDataDirectoryRegistry {
    fn read(&self) -> Result<Option<PathBuf>, DataDirectoryPreferenceError> {
        use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
        use windows_sys::Win32::System::Registry::{
            RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_SZ,
        };

        let mut key: HKEY = std::ptr::null_mut();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                wide(DATA_DIRECTORY_SUBKEY).as_ptr(),
                0,
                KEY_READ,
                &mut key,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != ERROR_SUCCESS {
            return Err(registry_error("RegOpenKeyExW", status));
        }
        let key = HkeyGuard(key);
        let value_name = wide(DATA_DIRECTORY_VALUE);
        let mut value_type = 0;
        let mut byte_len = 0;
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
        let mut units = vec![0u16; (byte_len as usize).div_ceil(2)];
        let mut buffer_bytes = (units.len() * std::mem::size_of::<u16>()) as u32;
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                value_name.as_ptr(),
                std::ptr::null(),
                &mut value_type,
                units.as_mut_ptr() as *mut u8,
                &mut buffer_bytes,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(registry_error("RegQueryValueExW", status));
        }
        let visible_len = (buffer_bytes as usize / 2).min(units.len());
        let end = units
            .iter()
            .take(visible_len)
            .position(|unit| *unit == 0)
            .unwrap_or(visible_len);
        let path = PathBuf::from(String::from_utf16_lossy(&units[..end]));
        Ok((path.is_absolute() && !path.as_os_str().is_empty()).then_some(path))
    }

    fn write(&self, directory: &Path) -> Result<(), DataDirectoryPreferenceError> {
        use windows_sys::Win32::Foundation::ERROR_SUCCESS;
        use windows_sys::Win32::System::Registry::{
            RegCreateKeyW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, REG_SZ,
        };

        let mut key: HKEY = std::ptr::null_mut();
        let status = unsafe {
            RegCreateKeyW(
                HKEY_CURRENT_USER,
                wide(DATA_DIRECTORY_SUBKEY).as_ptr(),
                &mut key,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(registry_error("RegCreateKeyW", status));
        }
        let key = HkeyGuard(key);
        let value_name = wide(DATA_DIRECTORY_VALUE);
        let data = wide(&directory.to_string_lossy());
        let status = unsafe {
            RegSetValueExW(
                key.0,
                value_name.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr() as *const u8,
                (data.len() * std::mem::size_of::<u16>()) as u32,
            )
        };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(registry_error("RegSetValueExW", status))
        }
    }
}
