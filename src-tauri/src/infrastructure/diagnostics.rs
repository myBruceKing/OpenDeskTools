//! Minimal local crash diagnostics.
//!
//! This intentionally records only Rust panic text and a timestamp in the
//! active data directory. It has no network path and does not inspect clipboard
//! contents or other application data.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use super::general_settings;
use super::storage::{StorageError, StorageService};

static LOGGER: OnceLock<DiagnosticsLogger> = OnceLock::new();

#[derive(Debug)]
struct DiagnosticsLogger {
    enabled: AtomicBool,
    data_root: Mutex<PathBuf>,
}

/// Installs the process panic hook once and updates its active local directory
/// and enabled state for the current runtime.
pub fn initialize(storage: &StorageService) -> Result<(), StorageError> {
    let enabled = general_settings::crash_diagnostics_enabled(storage)?;
    let logger = LOGGER.get_or_init(|| DiagnosticsLogger {
        enabled: AtomicBool::new(enabled),
        data_root: Mutex::new(storage.data_root().to_path_buf()),
    });
    logger.enabled.store(enabled, Ordering::Release);
    if let Ok(mut root) = logger.data_root.lock() {
        *root = storage.data_root().to_path_buf();
    }
    install_hook_once();
    Ok(())
}

pub fn set_enabled(storage: &StorageService, enabled: bool) -> Result<(), StorageError> {
    general_settings::set_crash_diagnostics_enabled(storage, enabled)?;
    if let Some(logger) = LOGGER.get() {
        logger.enabled.store(enabled, Ordering::Release);
    }
    Ok(())
}

fn install_hook_once() {
    static HOOK_INSTALLED: OnceLock<()> = OnceLock::new();
    HOOK_INSTALLED.get_or_init(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            write_panic(info);
            previous(info);
        }));
    });
}

fn write_panic(info: &std::panic::PanicHookInfo<'_>) {
    let Some(logger) = LOGGER.get() else {
        return;
    };
    if !logger.enabled.load(Ordering::Acquire) {
        return;
    }
    let Ok(data_root) = logger.data_root.lock().map(|root| root.clone()) else {
        return;
    };
    let directory = data_root.join("diagnostics");
    if fs::create_dir_all(&directory).is_err() {
        return;
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let body = format!("OpenDeskTools local crash report\ntimestamp_ms={timestamp}\n{info}\n");
    let _ = fs::write(directory.join(format!("crash-{timestamp}.log")), body);
}
