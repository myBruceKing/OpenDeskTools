#[cfg(windows)]
mod platform {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::thread::{self, JoinHandle};

    use crate::infrastructure::debug_qa;
    use thiserror::Error;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetAncestor, GetMessageW, PeekMessageW, PostQuitMessage,
        PostThreadMessageW, TranslateMessage, EVENT_SYSTEM_FOREGROUND, GA_ROOT, MSG, PM_NOREMOVE,
        WINEVENT_OUTOFCONTEXT, WM_QUIT,
    };

    #[derive(Debug, Error)]
    pub enum ForegroundMonitorError {
        #[error("clipboard foreground monitor lock is poisoned")]
        LockPoisoned,
        #[error("clipboard foreground monitor worker could not start")]
        ThreadStart,
        #[error("Windows could not install the clipboard foreground event hook")]
        InstallHook,
        #[error("Windows could not stop the clipboard foreground event hook")]
        StopHook,
        #[error("clipboard foreground monitor worker panicked")]
        WorkerPanicked,
    }

    struct HookContext {
        target_top_window: usize,
        internal_surface_roots: Vec<usize>,
        callback: Option<Box<dyn FnOnce() + Send>>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ForegroundDecision {
        IgnoreInvalid,
        KeepTarget,
        KeepInternalSurface,
        CloseDifferentRoot,
    }

    impl ForegroundDecision {
        const fn as_str(self) -> &'static str {
            match self {
                Self::IgnoreInvalid => "ignore_invalid",
                Self::KeepTarget => "keep_target_root",
                Self::KeepInternalSurface => "keep_internal_surface_root",
                Self::CloseDifferentRoot => "close_different_root",
            }
        }
    }

    thread_local! {
        static HOOK_CONTEXT: RefCell<Option<HookContext>> = const { RefCell::new(None) };
    }

    struct MonitorWorker {
        thread_id: u32,
        finished: Arc<AtomicBool>,
        join: JoinHandle<()>,
    }

    static MONITOR_WORKER: OnceLock<Mutex<Option<MonitorWorker>>> = OnceLock::new();

    fn worker_slot() -> &'static Mutex<Option<MonitorWorker>> {
        MONITOR_WORKER.get_or_init(|| Mutex::new(None))
    }

    pub fn start(
        target_top_window: usize,
        internal_surface_roots: Vec<usize>,
        callback: impl FnOnce() + Send + 'static,
    ) -> Result<(), ForegroundMonitorError> {
        if target_top_window == 0 {
            return Err(ForegroundMonitorError::InstallHook);
        }
        debug_qa::trace(format!(
            "foreground monitor start target_top={target_top_window:#x} internal_roots={internal_surface_roots:x?} flags=out_of_context include_own_process=true"
        ));
        let mut slot = worker_slot()
            .lock()
            .map_err(|_| ForegroundMonitorError::LockPoisoned)?;
        stop_locked(&mut slot)?;

        let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
        let finished = Arc::new(AtomicBool::new(false));
        let worker_finished = Arc::clone(&finished);
        let join = thread::Builder::new()
            .name("clipboard-foreground-monitor".to_string())
            .spawn(move || {
                let thread_id = unsafe { GetCurrentThreadId() };
                let mut message: MSG = unsafe { std::mem::zeroed() };
                unsafe {
                    PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
                }
                HOOK_CONTEXT.with(|context| {
                    *context.borrow_mut() = Some(HookContext {
                        target_top_window,
                        internal_surface_roots,
                        callback: Some(Box::new(callback)),
                    });
                });
                let hook = unsafe {
                    SetWinEventHook(
                        EVENT_SYSTEM_FOREGROUND,
                        EVENT_SYSTEM_FOREGROUND,
                        std::ptr::null_mut(),
                        Some(foreground_event_callback),
                        0,
                        0,
                        foreground_hook_flags(),
                    )
                };
                if hook.is_null() {
                    HOOK_CONTEXT.with(|context| *context.borrow_mut() = None);
                    let _ = started_tx.send(Err(ForegroundMonitorError::InstallHook));
                    worker_finished.store(true, Ordering::Release);
                    return;
                }
                if started_tx.send(Ok(thread_id)).is_err() {
                    unsafe {
                        UnhookWinEvent(hook);
                    }
                    HOOK_CONTEXT.with(|context| *context.borrow_mut() = None);
                    worker_finished.store(true, Ordering::Release);
                    return;
                }

                loop {
                    let result = unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) };
                    if result <= 0 {
                        break;
                    }
                    unsafe {
                        TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
                unsafe {
                    UnhookWinEvent(hook);
                }
                HOOK_CONTEXT.with(|context| *context.borrow_mut() = None);
                worker_finished.store(true, Ordering::Release);
            })
            .map_err(|_| ForegroundMonitorError::ThreadStart)?;

        let thread_id = match started_rx.recv() {
            Ok(Ok(thread_id)) => thread_id,
            Ok(Err(error)) => {
                let _ = join.join();
                return Err(error);
            }
            Err(_) => {
                let _ = join.join();
                return Err(ForegroundMonitorError::ThreadStart);
            }
        };
        *slot = Some(MonitorWorker {
            thread_id,
            finished,
            join,
        });
        Ok(())
    }

    pub fn stop() -> Result<(), ForegroundMonitorError> {
        let mut slot = worker_slot()
            .lock()
            .map_err(|_| ForegroundMonitorError::LockPoisoned)?;
        stop_locked(&mut slot)
    }

    fn stop_locked(slot: &mut Option<MonitorWorker>) -> Result<(), ForegroundMonitorError> {
        let Some(worker) = slot.as_ref() else {
            return Ok(());
        };
        if !worker.finished.load(Ordering::Acquire)
            && unsafe { PostThreadMessageW(worker.thread_id, WM_QUIT, 0, 0) } == 0
        {
            return Err(ForegroundMonitorError::StopHook);
        }
        let worker = slot.take().expect("worker existence checked above");
        worker
            .join
            .join()
            .map_err(|_| ForegroundMonitorError::WorkerPanicked)
    }

    unsafe extern "system" fn foreground_event_callback(
        _hook: *mut core::ffi::c_void,
        event: u32,
        window: HWND,
        _object_id: i32,
        _child_id: i32,
        _event_thread: u32,
        _event_time: u32,
    ) {
        if event != EVENT_SYSTEM_FOREGROUND || window.is_null() {
            return;
        }
        let top = GetAncestor(window, GA_ROOT);
        if top.is_null() {
            return;
        }
        let callback = HOOK_CONTEXT.with(|context| {
            let mut context = context.borrow_mut();
            let context = context.as_mut()?;
            let observed_top_window = top as usize;
            let decision = foreground_decision(
                context.target_top_window,
                &context.internal_surface_roots,
                observed_top_window,
            );
            debug_qa::trace(format!(
                "foreground observed target_top={:#x} internal_roots={:x?} observed_top={observed_top_window:#x} decision={}",
                context.target_top_window,
                context.internal_surface_roots,
                decision.as_str()
            ));
            (decision == ForegroundDecision::CloseDifferentRoot)
                .then(|| context.callback.take())
                .flatten()
        });
        if let Some(callback) = callback {
            callback();
            PostQuitMessage(0);
        }
    }

    const fn foreground_hook_flags() -> u32 {
        WINEVENT_OUTOFCONTEXT
    }

    fn foreground_decision(
        target_top_window: usize,
        internal_surface_roots: &[usize],
        observed_top_window: usize,
    ) -> ForegroundDecision {
        if target_top_window == 0 || observed_top_window == 0 {
            ForegroundDecision::IgnoreInvalid
        } else if observed_top_window == target_top_window {
            ForegroundDecision::KeepTarget
        } else if internal_surface_roots.contains(&observed_top_window) {
            ForegroundDecision::KeepInternalSurface
        } else {
            ForegroundDecision::CloseDifferentRoot
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn target_and_internal_surface_roots_are_kept_but_other_roots_close() {
            assert_eq!(
                foreground_decision(42, &[90, 91], 42),
                ForegroundDecision::KeepTarget
            );
            assert_eq!(
                foreground_decision(42, &[90, 91], 90),
                ForegroundDecision::KeepInternalSurface
            );
            // PID is deliberately absent from the decision. A same-process ODT
            // main window is still a different root and must close the group.
            assert_eq!(
                foreground_decision(42, &[90, 91], 77),
                ForegroundDecision::CloseDifferentRoot
            );
            assert_eq!(
                foreground_decision(42, &[90, 91], 0),
                ForegroundDecision::IgnoreInvalid
            );
            assert_eq!(
                foreground_decision(0, &[90, 91], 77),
                ForegroundDecision::IgnoreInvalid
            );
        }

        #[test]
        fn hook_includes_own_process_foreground_events() {
            use windows_sys::Win32::UI::WindowsAndMessaging::WINEVENT_SKIPOWNPROCESS;

            assert_eq!(foreground_hook_flags(), WINEVENT_OUTOFCONTEXT);
            assert_eq!(foreground_hook_flags() & WINEVENT_SKIPOWNPROCESS, 0);
        }

        #[test]
        fn completed_worker_is_joined_without_posting_to_a_dead_message_queue() {
            let finished = Arc::new(AtomicBool::new(true));
            let join = thread::spawn(|| {});
            let mut slot = Some(MonitorWorker {
                thread_id: u32::MAX,
                finished,
                join,
            });
            stop_locked(&mut slot).unwrap();
            assert!(slot.is_none());
        }

        #[test]
        fn stop_is_idempotent_when_no_worker_is_running() {
            let mut slot = None;
            stop_locked(&mut slot).unwrap();
            stop_locked(&mut slot).unwrap();
            assert!(slot.is_none());
        }
    }
}

#[cfg(windows)]
pub use platform::{start, stop, ForegroundMonitorError};

#[cfg(not(windows))]
mod platform_fallback {
    use thiserror::Error;

    #[derive(Debug, Error)]
    #[error("clipboard foreground monitoring is unavailable on this platform")]
    pub struct ForegroundMonitorError;

    pub fn start(
        _target_top_window: usize,
        _internal_surface_roots: Vec<usize>,
        _callback: impl FnOnce() + Send + 'static,
    ) -> Result<(), ForegroundMonitorError> {
        Err(ForegroundMonitorError)
    }

    pub fn stop() -> Result<(), ForegroundMonitorError> {
        Ok(())
    }
}

#[cfg(not(windows))]
pub use platform_fallback::{start, stop, ForegroundMonitorError};
