#[cfg(windows)]
mod platform {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::thread::{self, JoinHandle};

    use thiserror::Error;
    use windows_sys::Win32::Foundation::{HWND, POINT};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON, VK_XBUTTON1, VK_XBUTTON2,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetAncestor, GetCursorPos, GetMessageW, KillTimer,
        PeekMessageW, PostQuitMessage, PostThreadMessageW, SetTimer, SetWindowsHookExW,
        TranslateMessage, UnhookWindowsHookEx, WindowFromPoint, GA_ROOT, HC_ACTION, MSG,
        MSLLHOOKSTRUCT, PM_NOREMOVE, WH_MOUSE_LL, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_QUIT,
        WM_RBUTTONDOWN, WM_XBUTTONDOWN,
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
        #[error("Windows could not install the clipboard outside-pointer polling fallback")]
        InstallPollingFallback,
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
        polled_buttons_down: u8,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PointerObservation {
        pub message: u32,
        pub point_x: i32,
        pub point_y: i32,
        pub observed_root: usize,
        pub backend: &'static str,
    }

    const POINTER_POLL_INTERVAL_MS: u32 = 16;
    const BUTTON_LEFT: u8 = 1 << 0;
    const BUTTON_RIGHT: u8 = 1 << 1;
    const BUTTON_MIDDLE: u8 = 1 << 2;
    const BUTTON_X1: u8 = 1 << 3;
    const BUTTON_X2: u8 = 1 << 4;

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
            "outside pointer monitor start internal_roots={internal_surface_roots:x?} hook=WH_MOUSE_LL fallback=GetAsyncKeyState interval_ms={POINTER_POLL_INTERVAL_MS} pass_through=true"
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
                        // A button already held when the Surface opens is not
                        // an outside press. Only a new down edge may dismiss.
                        polled_buttons_down: current_button_mask(),
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
                // WH_MOUSE_LL can be absent for clicks routed to a higher-
                // integrity target such as Task Manager. Poll physical button
                // down edges on the same monitor thread as a non-invasive
                // fallback. Both paths consume the same one-shot callback.
                let poll_timer = unsafe {
                    SetTimer(
                        std::ptr::null_mut(),
                        0,
                        POINTER_POLL_INTERVAL_MS,
                        Some(pointer_poll_timer_callback),
                    )
                };
                if poll_timer == 0 {
                    unsafe {
                        UnhookWindowsHookEx(hook);
                    }
                    HOOK_CONTEXT.with(|context| *context.borrow_mut() = None);
                    let _ = started_tx.send(Err(PointerMonitorError::InstallPollingFallback));
                    worker_finished.store(true, Ordering::Release);
                    return;
                }
                if started_tx.send(Ok(thread_id)).is_err() {
                    unsafe {
                        KillTimer(std::ptr::null_mut(), poll_timer);
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
                    KillTimer(std::ptr::null_mut(), poll_timer);
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
            backend: "WH_MOUSE_LL",
        };
        let callback = take_outside_callback(message, observed_root);
        if let Some(callback) = callback {
            callback(observation);
            PostQuitMessage(0);
        }
        next
    }

    unsafe extern "system" fn pointer_poll_timer_callback(
        _window: HWND,
        _message: u32,
        _timer_id: usize,
        _time: u32,
    ) {
        let pressed = HOOK_CONTEXT.with(|context| {
            let mut context = context.borrow_mut();
            let context = context.as_mut()?;
            let current = current_button_mask();
            let pressed = new_button_presses(context.polled_buttons_down, current);
            context.polled_buttons_down = current;
            first_button_message(pressed)
        });
        let Some(message) = pressed else {
            return;
        };
        let mut point = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut point) == 0 {
            return;
        }
        let pointed_window = WindowFromPoint(point);
        let observed_root = if pointed_window.is_null() {
            0
        } else {
            GetAncestor(pointed_window, GA_ROOT) as usize
        };
        let observation = PointerObservation {
            message,
            point_x: point.x,
            point_y: point.y,
            observed_root,
            backend: "GetAsyncKeyState",
        };
        if let Some(callback) = take_outside_callback(message, observed_root) {
            callback(observation);
            PostQuitMessage(0);
        }
    }

    fn take_outside_callback(
        message: u32,
        observed_root: usize,
    ) -> Option<Box<dyn FnOnce(PointerObservation) + Send>> {
        HOOK_CONTEXT.with(|context| {
            let mut context = context.borrow_mut();
            let context = context.as_mut()?;
            let decision =
                pointer_decision(message, &context.internal_surface_roots, observed_root);
            (decision == PointerDecision::CloseOutside)
                .then(|| context.callback.take())
                .flatten()
        })
    }

    fn current_button_mask() -> u8 {
        let mut mask = 0;
        for (button, bit) in [
            (VK_LBUTTON, BUTTON_LEFT),
            (VK_RBUTTON, BUTTON_RIGHT),
            (VK_MBUTTON, BUTTON_MIDDLE),
            (VK_XBUTTON1, BUTTON_X1),
            (VK_XBUTTON2, BUTTON_X2),
        ] {
            // The high bit catches a currently held button. The low bit also
            // catches a very short press-and-release between two 16 ms polls;
            // it is only a supplement because Windows does not reserve that
            // bit exclusively for this process.
            if unsafe { GetAsyncKeyState(button as i32) } as u16 & 0x8001 != 0 {
                mask |= bit;
            }
        }
        mask
    }

    const fn new_button_presses(previous: u8, current: u8) -> u8 {
        current & !previous
    }

    const fn first_button_message(pressed: u8) -> Option<u32> {
        if pressed & BUTTON_LEFT != 0 {
            Some(WM_LBUTTONDOWN)
        } else if pressed & BUTTON_RIGHT != 0 {
            Some(WM_RBUTTONDOWN)
        } else if pressed & BUTTON_MIDDLE != 0 {
            Some(WM_MBUTTONDOWN)
        } else if pressed & (BUTTON_X1 | BUTTON_X2) != 0 {
            Some(WM_XBUTTONDOWN)
        } else {
            None
        }
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
        fn polling_fallback_only_reports_new_button_down_edges() {
            assert_eq!(new_button_presses(0, BUTTON_LEFT), BUTTON_LEFT);
            assert_eq!(new_button_presses(BUTTON_LEFT, BUTTON_LEFT), 0);
            assert_eq!(new_button_presses(BUTTON_LEFT, 0), 0);
            assert_eq!(
                new_button_presses(BUTTON_LEFT, BUTTON_LEFT | BUTTON_RIGHT),
                BUTTON_RIGHT
            );
        }

        #[test]
        fn polling_button_mask_maps_to_the_same_pointer_messages_as_the_hook() {
            assert_eq!(first_button_message(BUTTON_LEFT), Some(WM_LBUTTONDOWN));
            assert_eq!(first_button_message(BUTTON_RIGHT), Some(WM_RBUTTONDOWN));
            assert_eq!(first_button_message(BUTTON_MIDDLE), Some(WM_MBUTTONDOWN));
            assert_eq!(first_button_message(BUTTON_X1), Some(WM_XBUTTONDOWN));
            assert_eq!(first_button_message(BUTTON_X2), Some(WM_XBUTTONDOWN));
            assert_eq!(first_button_message(0), None);
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
        pub backend: &'static str,
    }
}

#[cfg(not(windows))]
pub use platform_fallback::{start, stop, PointerMonitorError, PointerObservation};
