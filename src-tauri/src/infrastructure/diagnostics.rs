//! Minimal local crash diagnostics.
//!
//! This intentionally records only Rust panic text and a timestamp in the
//! active data directory. It has no network path and does not inspect clipboard
//! contents or other application data.

use std::backtrace::Backtrace;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use super::general_settings;
use super::storage::{StorageError, StorageService};

static LOGGER: OnceLock<DiagnosticsLogger> = OnceLock::new();
static FIRST_PANIC_WRITE_STARTED: AtomicBool = AtomicBool::new(false);

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

/// Persists a content-free startup health marker for post-mortem diagnosis.
/// Clipboard data, window titles and user paths are intentionally excluded.
pub fn write_startup_health(status: &str, stage: &str) {
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
    let body = format!(
        "OpenDeskTools startup health\ntimestamp_ms={timestamp}\nprocess_id={}\nstatus={status}\nstage={stage}\n",
        std::process::id()
    );
    let _ = fs::write(directory.join("startup-latest.log"), body);
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
    // A teardown failure can trigger a second panic before the process aborts.
    // Preserve the initiating panic instead of letting the later TAO panic
    // overwrite or truncate it.
    if FIRST_PANIC_WRITE_STARTED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
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
    let path = directory.join(format!("crash-{timestamp}-{}.log", std::process::id()));
    let Ok(mut file) = OpenOptions::new().write(true).create_new(true).open(path) else {
        return;
    };
    let header = format!(
        "OpenDeskTools local crash report\ntimestamp_ms={timestamp}\nprocess_id={}\n{info}\n",
        std::process::id()
    );
    if file.write_all(header.as_bytes()).is_err() {
        return;
    }
    // Commit the primary panic text before the more expensive backtrace so a
    // fast process abort still leaves a useful, non-empty report.
    let _ = file.sync_all();
    let backtrace = Backtrace::force_capture();
    let _ = writeln!(file, "backtrace:\n{backtrace}");
    let _ = file.sync_all();
}

/// Bounded, asynchronous runtime diagnostics. Call sites supply static stages only;
/// clipboard contents, history IDs, paths and window titles cannot enter this API.
pub fn clipboard_event(
    stage: &'static str,
    result: &'static str,
    attempts: usize,
    elapsed_ms: u128,
    holder_pid: u32,
) {
    clipboard_event_details(stage, result, attempts, elapsed_ms, holder_pid, None);
}

pub fn clipboard_read_failure(
    stage: &'static str,
    win32_error: u32,
    holder_pid: u32,
    owner_pid: u32,
) {
    clipboard_event_details(
        stage,
        "failed",
        1,
        0,
        holder_pid,
        Some((win32_error, owner_pid)),
    );
}

fn clipboard_event_details(
    stage: &'static str,
    result: &'static str,
    attempts: usize,
    elapsed_ms: u128,
    holder_pid: u32,
    failure: Option<(u32, u32)>,
) {
    use std::sync::mpsc::{sync_channel, SyncSender};
    static QUEUE: OnceLock<Option<SyncSender<String>>> = OnceLock::new();
    let Some(logger) = LOGGER.get() else {
        return;
    };
    if !logger.enabled.load(Ordering::Acquire) {
        return;
    }
    let sender = QUEUE.get_or_init(|| {
        let (sender, receiver) = sync_channel::<String>(128);
        std::thread::Builder::new()
            .name("clipboard-diagnostics".into())
            .spawn(move || {
                for line in receiver {
                    let Some(logger) = LOGGER.get() else {
                        continue;
                    };
                    if !logger.enabled.load(Ordering::Acquire) {
                        continue;
                    }
                    let Ok(root) = logger.data_root.lock() else {
                        continue;
                    };
                    let _ = append_clipboard_event(&root.join("diagnostics"), &line, 1_048_576);
                }
            })
            .ok()
            .map(|_| sender)
    });
    if let Some(sender) = sender {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let mut line = format!("timestamp_ms={timestamp} process_id={} stage={stage} result={result} attempts={attempts} elapsed_ms={elapsed_ms} holder_pid={holder_pid}", std::process::id());
        if let Some((error, owner_pid)) = failure {
            line.push_str(&format!(" win32_error={error} owner_pid={owner_pid}"));
        }
        let _ = sender.try_send(line);
    }
}

fn append_clipboard_event(
    directory: &std::path::Path,
    line: &str,
    limit: u64,
) -> std::io::Result<()> {
    fs::create_dir_all(directory)?;
    let path = directory.join("clipboard-runtime.log");
    if fs::metadata(&path).is_ok_and(|m| m.len() >= limit) {
        let previous = directory.join("clipboard-runtime.previous.log");
        if previous.exists() {
            fs::remove_file(&previous)?;
        }
        fs::rename(&path, &previous)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{line}")
}

#[cfg(test)]
mod clipboard_log_tests {
    use super::*;

    #[test]
    fn rotation_preserves_latest_and_one_previous_log() {
        let directory = std::env::temp_dir().join(format!(
            "odt-log-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        append_clipboard_event(&directory, "first", 1).unwrap();
        append_clipboard_event(&directory, "second", 1).unwrap();
        assert_eq!(
            fs::read_to_string(directory.join("clipboard-runtime.previous.log")).unwrap(),
            "first\n"
        );
        append_clipboard_event(&directory, "third", 1).unwrap();
        assert_eq!(
            fs::read_to_string(directory.join("clipboard-runtime.previous.log")).unwrap(),
            "second\n"
        );
        assert_eq!(
            fs::read_to_string(directory.join("clipboard-runtime.log")).unwrap(),
            "third\n"
        );
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 2);
        fs::remove_dir_all(directory).unwrap();
    }
}
