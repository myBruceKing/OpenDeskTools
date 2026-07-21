#[cfg(windows)]
mod platform {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::thread::{self, JoinHandle};

    use thiserror::Error;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetAncestor, GetMessageW, PeekMessageW, PostQuitMessage,
        PostThreadMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
        WindowFromPoint, GA_ROOT, HC_ACTION, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE, WH_MOUSE_LL,
        WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_QUIT, WM_RBUTTONDOWN, WM_XBUTTONDOWN,
    };

    use super::super::debug_qa;

    #[derive(Debug, Error)]
    pub enum PointerMonitorError {
        #[error("clipboard outside-pointer monitor lock is poisoned")]
        LockPoisoned,
        #[error("clipboard outside-pointer monitor worker could not start")]
        ThreadStart,
        #[error("Windows could not install the clipboard outside-pointer hook")]
        InstallHook,
        #[error("Windows could not stop the clipboard outside-pointer hook")]
        StopHook,
        #[error("clipboard outside-pointer monitor worker panicked")]
        WorkerPanicked,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum PointerDecision {
        IgnoreNonButton,
        KeepInternalSurface,
        CloseOutside,
    }

    struct HookContext {
        internal_surface_roots: Vec<usize>,
        callback: Option<Box<dyn FnOnce(PointerObservation) + Send>>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PointerObservation {
        pub message: u32,
        pub point_x: i32,
        pub point_y: i32,
        pub observed_root: usize,
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
        internal_surface_roots: Vec<usize>,
        callback: impl FnOnce(PointerObservation) + Send + 'static,
    ) -> Result<(), PointerMonitorError> {
        if internal_surface_roots.is_empty() || internal_surface_roots.contains(&0) {
            return Err(PointerMonitorError::InstallHook);
        }
        debug_qa::trace(format!(
            "outside pointer monitor start internal_roots={internal_surface_roots:x?} hook=WH_MOUSE_LL pass_through=true"
        ));
        let mut slot = worker_slot()
            .lock()
            .map_err(|_| PointerMonitorError::LockPoisoned)?;
        stop_locked(&mut slot)?;

        let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
        let finished = Arc::new(AtomicBool::new(false));
        let worker_finished = Arc::clone(&finished);
        let join = thread::Builder::new()
            .name("clipboard-outside-pointer-monitor".to_string())
            .spawn(move || {
                let thread_id = unsafe { GetCurrentThreadId() };
                let mut message: MSG = unsafe { std::mem::zeroed() };
                unsafe {
                    PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
                }
                HOOK_CONTEXT.with(|context| {
                    *context.borrow_mut() = Some(HookContext {
                        internal_surface_roots,
                        callback: Some(Box::new(callback)),
                    });
                });
                let hook = unsafe {
                    SetWindowsHookExW(
                        WH_MOUSE_LL,
                        Some(low_level_mouse_callback),
                        std::ptr::null_mut(),
                        0,
                    )
                };
                if hook.is_null() {
                    HOOK_CONTEXT.with(|context| *context.borrow_mut() = None);
                    let _ = started_tx.send(Err(PointerMonitorError::InstallHook));
                    worker_finished.store(true, Ordering::Release);
                    return;
                }
                if started_tx.send(Ok(thread_id)).is_err() {
                    unsafe {
                        UnhookWindowsHookEx(hook);
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
                    UnhookWindowsHookEx(hook);
                }
                HOOK_CONTEXT.with(|context| *context.borrow_mut() = None);
                worker_finished.store(true, Ordering::Release);
            })
            .map_err(|_| PointerMonitorError::ThreadStart)?;

        let thread_id = match started_rx.recv() {
            Ok(Ok(thread_id)) => thread_id,
            Ok(Err(error)) => {
                let _ = join.join();
                return Err(error);
            }
            Err(_) => {
                let _ = join.join();
                return Err(PointerMonitorError::ThreadStart);
            }
        };
        *slot = Some(MonitorWorker {
            thread_id,
            finished,
            join,
        });
        Ok(())
    }

    pub fn stop() -> Result<(), PointerMonitorError> {
        let mut slot = worker_slot()
            .lock()
            .map_err(|_| PointerMonitorError::LockPoisoned)?;
        stop_locked(&mut slot)
    }

    fn stop_locked(slot: &mut Option<MonitorWorker>) -> Result<(), PointerMonitorError> {
        let Some(worker) = slot.as_ref() else {
            return Ok(());
        };
        if !worker.finished.load(Ordering::Acquire)
            && unsafe { PostThreadMessageW(worker.thread_id, WM_QUIT, 0, 0) } == 0
        {
            return Err(PointerMonitorError::StopHook);
        }
        let worker = slot.take().expect("worker existence checked above");
        worker
            .join
            .join()
            .map_err(|_| PointerMonitorError::WorkerPanicked)
    }

    unsafe extern "system" fn low_level_mouse_callback(
        code: i32,
        w_param: usize,
        l_param: isize,
    ) -> isize {
        // Never suppress or rewrite the event. Calling the next hook first also
        // keeps the OpenDeskTools close request off the original click path.
        let next = CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param);
        if code < HC_ACTION as i32 || l_param == 0 {
            return next;
        }
        let message = w_param as u32;
        if !is_button_down_message(message) {
            return next;
        }
        let mouse = &*(l_param as *const MSLLHOOKSTRUCT);
        let pointed_window = WindowFromPoint(mouse.pt);
        let observed_root = if pointed_window.is_null() {
            0
        } else {
            GetAncestor(pointed_window, GA_ROOT) as usize
        };
        let observation = PointerObservation {
            message,
            point_x: mouse.pt.x,
            point_y: mouse.pt.y,
            observed_root,
        };
        let callback = HOOK_CONTEXT.with(|context| {
            let mut context = context.borrow_mut();
            let context = context.as_mut()?;
            let decision =
                pointer_decision(message, &context.internal_surface_roots, observed_root);
            (decision == PointerDecision::CloseOutside)
                .then(|| context.callback.take())
                .flatten()
        });
        if let Some(callback) = callback {
            callback(observation);
            PostQuitMessage(0);
        }
        next
    }

    const fn is_button_down_message(message: u32) -> bool {
        matches!(
            message,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
        )
    }

    fn pointer_decision(
        message: u32,
        internal_surface_roots: &[usize],
        observed_root: usize,
    ) -> PointerDecision {
        if !is_button_down_message(message) {
            PointerDecision::IgnoreNonButton
        } else if observed_root != 0 && internal_surface_roots.contains(&observed_root) {
            PointerDecision::KeepInternalSurface
        } else {
            // A null hit or any non-group root is outside. WindowFromPoint
            // resolves target windows, the desktop and taskbar without focus.
            PointerDecision::CloseOutside
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_LBUTTONUP, WM_MOUSEMOVE};

        #[test]
        fn every_button_down_closes_outside_but_keeps_all_internal_roots() {
            for message in [
                WM_LBUTTONDOWN,
                WM_RBUTTONDOWN,
                WM_MBUTTONDOWN,
                WM_XBUTTONDOWN,
            ] {
                assert_eq!(
                    pointer_decision(message, &[10, 20], 10),
                    PointerDecision::KeepInternalSurface
                );
                assert_eq!(
                    pointer_decision(message, &[10, 20], 20),
                    PointerDecision::KeepInternalSurface
                );
                assert_eq!(
                    pointer_decision(message, &[10, 20], 30),
                    PointerDecision::CloseOutside
                );
                assert_eq!(
                    pointer_decision(message, &[10, 20], 0),
                    PointerDecision::CloseOutside
                );
            }
            for message in [WM_MOUSEMOVE, WM_LBUTTONUP] {
                assert_eq!(
                    pointer_decision(message, &[10, 20], 30),
                    PointerDecision::IgnoreNonButton
                );
            }
        }

        #[test]
        fn completed_worker_is_joined_without_posting_to_dead_queue() {
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
        fn stop_is_idempotent_without_worker() {
            let mut slot = None;
            stop_locked(&mut slot).unwrap();
            stop_locked(&mut slot).unwrap();
            assert!(slot.is_none());
        }
    }
}

#[cfg(windows)]
pub use platform::{start, stop, PointerMonitorError, PointerObservation};

#[cfg(not(windows))]
mod platform_fallback {
    use thiserror::Error;

    #[derive(Debug, Error)]
    #[error("clipboard outside-pointer monitoring is unavailable on this platform")]
    pub struct PointerMonitorError;

    pub fn start(
        _internal_surface_roots: Vec<usize>,
        _callback: impl FnOnce(PointerObservation) + Send + 'static,
    ) -> Result<(), PointerMonitorError> {
        Err(PointerMonitorError)
    }

    pub fn stop() -> Result<(), PointerMonitorError> {
        Ok(())
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PointerObservation {
        pub message: u32,
        pub point_x: i32,
        pub point_y: i32,
        pub observed_root: usize,
    }
}

#[cfg(not(windows))]
pub use platform_fallback::{start, stop, PointerMonitorError, PointerObservation};
