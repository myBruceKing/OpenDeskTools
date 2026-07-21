//! Persisted "General" behaviour preferences.
//!
//! These are simple boolean toggles that must be readable synchronously from the
//! startup and window-lifecycle paths (not just from a command), so they live in
//! the shared [`StorageService`] key/value settings table with explicit defaults.

use super::storage::{StorageError, StorageService};

const START_MINIMIZED_KEY: &str = "general.start_minimized";
const CLOSE_TO_TRAY_KEY: &str = "general.close_to_tray";

const START_MINIMIZED_DEFAULT: bool = false;
const CLOSE_TO_TRAY_DEFAULT: bool = true;

/// Whether a normal (non-autostart) launch should stay hidden in the tray.
pub fn start_minimized(storage: &StorageService) -> Result<bool, StorageError> {
    read_bool(storage, START_MINIMIZED_KEY, START_MINIMIZED_DEFAULT)
}

pub fn set_start_minimized(storage: &StorageService, enabled: bool) -> Result<(), StorageError> {
    write_bool(storage, START_MINIMIZED_KEY, enabled)
}

/// Whether closing the main window hides it to the tray (default) instead of
/// quitting the whole application.
pub fn close_to_tray(storage: &StorageService) -> Result<bool, StorageError> {
    read_bool(storage, CLOSE_TO_TRAY_KEY, CLOSE_TO_TRAY_DEFAULT)
}

pub fn set_close_to_tray(storage: &StorageService, enabled: bool) -> Result<(), StorageError> {
    write_bool(storage, CLOSE_TO_TRAY_KEY, enabled)
}

fn read_bool(storage: &StorageService, key: &str, default: bool) -> Result<bool, StorageError> {
    Ok(storage
        .read_setting(key)?
        .map(|value| value == "true")
        .unwrap_or(default))
}

fn write_bool(storage: &StorageService, key: &str, enabled: bool) -> Result<(), StorageError> {
    storage.write_settings(&[(key, if enabled { "true" } else { "false" })])
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn storage() -> (tempfile::TempDir, StorageService) {
        let temp = tempdir().unwrap();
        let storage = StorageService::initialize(temp.path().join("data")).unwrap();
        (temp, storage)
    }

    #[test]
    fn defaults_apply_before_any_write() {
        let (_temp, storage) = storage();
        assert!(!start_minimized(&storage).unwrap());
        assert!(close_to_tray(&storage).unwrap());
    }

    #[test]
    fn toggles_round_trip_through_storage() {
        let (_temp, storage) = storage();

        set_start_minimized(&storage, true).unwrap();
        set_close_to_tray(&storage, false).unwrap();

        assert!(start_minimized(&storage).unwrap());
        assert!(!close_to_tray(&storage).unwrap());

        set_start_minimized(&storage, false).unwrap();
        set_close_to_tray(&storage, true).unwrap();

        assert!(!start_minimized(&storage).unwrap());
        assert!(close_to_tray(&storage).unwrap());
    }
}
