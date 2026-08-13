//! Process-level startup health guard.
//!
//! Tauri creates WebView2 windows synchronously on its event-loop thread. A
//! broken or freshly updated WebView2 runtime can therefore leave the native
//! tray and hotkey hosts alive while setup never completes. The watchdog stays
//! outside that event loop and terminates the unhealthy task instance with a
//! non-zero exit code so Task Scheduler can start a clean replacement.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::diagnostics;

pub const UNHEALTHY_RUNTIME_EXIT_CODE: i32 = 12;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug)]
struct StartupState {
    completed: AtomicBool,
    stage: Mutex<&'static str>,
}

#[derive(Debug, Clone)]
pub struct StartupWatchdog {
    state: Arc<StartupState>,
}

impl StartupWatchdog {
    pub fn start() -> Self {
        let state = Arc::new(StartupState {
            completed: AtomicBool::new(false),
            stage: Mutex::new("tauri_builder"),
        });
        let worker_state = Arc::clone(&state);
        let spawn_result = thread::Builder::new()
            .name("startup-watchdog".to_owned())
            .spawn(move || {
                thread::sleep(STARTUP_TIMEOUT);
                if worker_state.completed.load(Ordering::Acquire) {
                    return;
                }
                let stage = worker_state
                    .stage
                    .lock()
                    .map(|stage| *stage)
                    .unwrap_or("stage_lock_unavailable");
                diagnostics::write_startup_health("timed_out", stage);
                std::process::exit(UNHEALTHY_RUNTIME_EXIT_CODE);
            });
        if let Err(error) = spawn_result {
            eprintln!("failed to start the startup watchdog: {error}");
        }
        Self { state }
    }

    pub fn enter(&self, stage: &'static str) {
        if let Ok(mut current) = self.state.stage.lock() {
            *current = stage;
        }
    }

    pub fn complete(&self) {
        self.enter("ready");
        self.state.completed.store(true, Ordering::Release);
        diagnostics::write_startup_health("ready", "ready");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_updates_do_not_mark_startup_complete() {
        let state = Arc::new(StartupState {
            completed: AtomicBool::new(false),
            stage: Mutex::new("tauri_builder"),
        });
        let watchdog = StartupWatchdog { state };

        watchdog.enter("clipboard_surface");

        assert_eq!(*watchdog.state.stage.lock().unwrap(), "clipboard_surface");
        assert!(!watchdog.state.completed.load(Ordering::Acquire));
    }

    #[test]
    fn completion_records_ready_and_releases_the_worker() {
        let state = Arc::new(StartupState {
            completed: AtomicBool::new(false),
            stage: Mutex::new("main_window"),
        });
        let watchdog = StartupWatchdog { state };

        watchdog.complete();

        assert_eq!(*watchdog.state.stage.lock().unwrap(), "ready");
        assert!(watchdog.state.completed.load(Ordering::Acquire));
    }
}
