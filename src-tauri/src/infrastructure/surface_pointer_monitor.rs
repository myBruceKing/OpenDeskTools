#[cfg(windows)]
mod platform {
    use std::collections::HashMap;
    use std::mem::{size_of, zeroed};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::thread::{self, JoinHandle};

    use thiserror::Error;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::{
        GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE,
        RIDEV_INPUTSINK, RIDEV_REMOVE, RID_INPUT, RIM_TYPEMOUSE,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetAncestor,
        GetCursorPos, GetMessageW, PostThreadMessageW, RegisterClassW, TranslateMessage,
        UnregisterClassW, WindowFromPoint, GA_ROOT, MSG, RI_MOUSE_BUTTON_1_DOWN,
        RI_MOUSE_BUTTON_2_DOWN, RI_MOUSE_BUTTON_3_DOWN, RI_MOUSE_BUTTON_4_DOWN,
        RI_MOUSE_BUTTON_5_DOWN, WM_INPUT, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_QUIT, WM_RBUTTONDOWN,
        WM_XBUTTONDOWN, WNDCLASSW, WS_POPUP,
    };

    use super::super::debug_qa;

    const RAW_INPUT_CLASS_NAME: [u16; 37] = [
        79, 112, 101, 110, 68, 101, 115, 107, 84, 111, 111, 108, 115, 83, 117, 114, 102, 97, 99,
        101, 80, 111, 105, 110, 116, 101, 114, 77, 111, 110, 105, 116, 111, 114, 0, 0, 0,
    ];
    const HID_USAGE_PAGE_GENERIC_DESKTOP: u16 = 0x01;
    const HID_USAGE_GENERIC_MOUSE: u16 = 0x02;

    #[derive(Debug, Error)]
    pub enum PointerMonitorError {
        #[error("surface outside-pointer monitor lock is poisoned")]
        LockPoisoned,
        #[error("surface outside-pointer monitor worker could not start")]
        ThreadStart,
        #[error("Windows could not create the surface outside-pointer monitor window")]
        CreateWindow,
        #[error("Windows could not register raw mouse input for surface monitoring")]
        RegisterRawInput,
        #[error("Windows could not stop the surface outside-pointer monitor")]
        StopWorker,
        #[error("surface outside-pointer monitor worker panicked")]
        WorkerPanicked,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum PointerMonitorOwner {
        Clipboard,
        ToolMenu,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum PointerDecision {
        IgnoreNonButton,
        KeepInternalSurface,
        CloseOutside,
    }

    struct Listener {
        internal_surface_roots: Vec<usize>,
        callback: Option<Box<dyn FnOnce(PointerObservation) + Send>>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PointerObservation {
        pub message: u32,
        pub point_x: i32,
        pub point_y: i32,
        pub observed_root: usize,
        pub backend: &'static str,
    }

    struct MonitorWorker {
        thread_id: u32,
        finished: Arc<AtomicBool>,
        join: JoinHandle<()>,
    }

    static LISTENERS: OnceLock<Mutex<HashMap<PointerMonitorOwner, Listener>>> = OnceLock::new();
    static MONITOR_WORKER: OnceLock<Mutex<Option<MonitorWorker>>> = OnceLock::new();

    fn listeners() -> &'static Mutex<HashMap<PointerMonitorOwner, Listener>> {
        LISTENERS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn worker_slot() -> &'static Mutex<Option<MonitorWorker>> {
        MONITOR_WORKER.get_or_init(|| Mutex::new(None))
    }

    pub fn start(
        owner: PointerMonitorOwner,
        internal_surface_roots: Vec<usize>,
        callback: impl FnOnce(PointerObservation) + Send + 'static,
    ) -> Result<(), PointerMonitorError> {
        if internal_surface_roots.is_empty() || internal_surface_roots.contains(&0) {
            return Err(PointerMonitorError::RegisterRawInput);
        }
        ensure_worker()?;
        listeners()
            .lock()
            .map_err(|_| PointerMonitorError::LockPoisoned)?
            .insert(
                owner,
                Listener {
                    internal_surface_roots: internal_surface_roots.clone(),
                    callback: Some(Box::new(callback)),
                },
            );
        debug_qa::trace(format!(
            "outside pointer monitor start owner={owner:?} internal_roots={internal_surface_roots:x?} backend=RawInput pass_through=true"
        ));
        Ok(())
    }

    pub fn stop(owner: PointerMonitorOwner) -> Result<(), PointerMonitorError> {
        listeners()
            .lock()
            .map_err(|_| PointerMonitorError::LockPoisoned)?
            .remove(&owner);
        Ok(())
    }

    pub fn stop_all() -> Result<(), PointerMonitorError> {
        listeners()
            .lock()
            .map_err(|_| PointerMonitorError::LockPoisoned)?
            .clear();
        let mut slot = worker_slot()
            .lock()
            .map_err(|_| PointerMonitorError::LockPoisoned)?;
        stop_worker(&mut slot)
    }

    fn ensure_worker() -> Result<(), PointerMonitorError> {
        let mut slot = worker_slot()
            .lock()
            .map_err(|_| PointerMonitorError::LockPoisoned)?;
        if slot
            .as_ref()
            .is_some_and(|worker| worker.finished.load(Ordering::Acquire))
        {
            stop_worker(&mut slot)?;
        }
        if slot.is_some() {
            return Ok(());
        }

        let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
        let finished = Arc::new(AtomicBool::new(false));
        let worker_finished = Arc::clone(&finished);
        let join = thread::Builder::new()
            .name("surface-outside-pointer-monitor".to_owned())
            .spawn(move || {
                let result = run_worker(started_tx);
                if let Err(error) = result {
                    debug_qa::trace(format!(
                        "outside pointer monitor worker exit result=error error={error}"
                    ));
                }
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

    fn run_worker(
        started_tx: std::sync::mpsc::SyncSender<Result<u32, PointerMonitorError>>,
    ) -> Result<(), PointerMonitorError> {
        let module = unsafe { GetModuleHandleW(std::ptr::null()) };
        if module.is_null() {
            let _ = started_tx.send(Err(PointerMonitorError::CreateWindow));
            return Err(PointerMonitorError::CreateWindow);
        }
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(pointer_window_proc),
            hInstance: module,
            lpszClassName: RAW_INPUT_CLASS_NAME.as_ptr(),
            ..unsafe { zeroed() }
        };
        let atom = unsafe { RegisterClassW(&window_class) };
        if atom == 0 {
            let _ = started_tx.send(Err(PointerMonitorError::CreateWindow));
            return Err(PointerMonitorError::CreateWindow);
        }
        let window = unsafe {
            CreateWindowExW(
                0,
                RAW_INPUT_CLASS_NAME.as_ptr(),
                RAW_INPUT_CLASS_NAME.as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                module,
                std::ptr::null(),
            )
        };
        if window.is_null() {
            unsafe {
                UnregisterClassW(RAW_INPUT_CLASS_NAME.as_ptr(), module);
            }
            let _ = started_tx.send(Err(PointerMonitorError::CreateWindow));
            return Err(PointerMonitorError::CreateWindow);
        }
        if !register_raw_mouse(window) {
            unsafe {
                DestroyWindow(window);
                UnregisterClassW(RAW_INPUT_CLASS_NAME.as_ptr(), module);
            }
            let _ = started_tx.send(Err(PointerMonitorError::RegisterRawInput));
            return Err(PointerMonitorError::RegisterRawInput);
        }

        let thread_id = unsafe { GetCurrentThreadId() };
        if started_tx.send(Ok(thread_id)).is_err() {
            unregister_raw_mouse();
            unsafe {
                DestroyWindow(window);
                UnregisterClassW(RAW_INPUT_CLASS_NAME.as_ptr(), module);
            }
            return Ok(());
        }

        let mut message: MSG = unsafe { zeroed() };
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
        unregister_raw_mouse();
        unsafe {
            DestroyWindow(window);
            UnregisterClassW(RAW_INPUT_CLASS_NAME.as_ptr(), module);
        }
        Ok(())
    }

    fn register_raw_mouse(window: HWND) -> bool {
        let device = RAWINPUTDEVICE {
            usUsagePage: HID_USAGE_PAGE_GENERIC_DESKTOP,
            usUsage: HID_USAGE_GENERIC_MOUSE,
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: window,
        };
        unsafe {
            RegisterRawInputDevices(
                &device,
                1,
                u32::try_from(size_of::<RAWINPUTDEVICE>()).unwrap_or_default(),
            ) != 0
        }
    }

    fn unregister_raw_mouse() {
        let device = RAWINPUTDEVICE {
            usUsagePage: HID_USAGE_PAGE_GENERIC_DESKTOP,
            usUsage: HID_USAGE_GENERIC_MOUSE,
            dwFlags: RIDEV_REMOVE,
            hwndTarget: std::ptr::null_mut(),
        };
        unsafe {
            let _ = RegisterRawInputDevices(
                &device,
                1,
                u32::try_from(size_of::<RAWINPUTDEVICE>()).unwrap_or_default(),
            );
        }
    }

    fn stop_worker(slot: &mut Option<MonitorWorker>) -> Result<(), PointerMonitorError> {
        let Some(worker) = slot.as_ref() else {
            return Ok(());
        };
        if !worker.finished.load(Ordering::Acquire)
            && unsafe { PostThreadMessageW(worker.thread_id, WM_QUIT, 0, 0) } == 0
        {
            return Err(PointerMonitorError::StopWorker);
        }
        let worker = slot.take().expect("worker existence checked above");
        worker
            .join
            .join()
            .map_err(|_| PointerMonitorError::WorkerPanicked)
    }

    unsafe extern "system" fn pointer_window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_INPUT {
            dispatch_raw_input(lparam as HRAWINPUT);
            return DefWindowProcW(window, message, wparam, lparam);
        }
        DefWindowProcW(window, message, wparam, lparam)
    }

    unsafe fn dispatch_raw_input(raw_handle: HRAWINPUT) {
        let Some(message) = raw_input_button_message(raw_handle) else {
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
            backend: "RawInput",
        };
        let callbacks = take_outside_callbacks(message, observed_root);
        for callback in callbacks {
            callback(observation);
        }
    }

    unsafe fn raw_input_button_message(raw_handle: HRAWINPUT) -> Option<u32> {
        let mut byte_count = 0u32;
        let header_size =
            u32::try_from(size_of::<windows_sys::Win32::UI::Input::RAWINPUTHEADER>()).ok()?;
        if GetRawInputData(
            raw_handle,
            RID_INPUT,
            std::ptr::null_mut(),
            &mut byte_count,
            header_size,
        ) == u32::MAX
            || byte_count < header_size
        {
            return None;
        }
        let word_count = usize::try_from(byte_count)
            .ok()?
            .div_ceil(size_of::<usize>());
        let mut storage = vec![0usize; word_count];
        let copied = GetRawInputData(
            raw_handle,
            RID_INPUT,
            storage.as_mut_ptr().cast(),
            &mut byte_count,
            header_size,
        );
        if copied == u32::MAX
            || usize::try_from(copied).ok()? < size_of::<RAWINPUT>()
            || usize::try_from(byte_count).ok()? < size_of::<RAWINPUT>()
        {
            return None;
        }
        let raw = &*storage.as_ptr().cast::<RAWINPUT>();
        if raw.header.dwType != RIM_TYPEMOUSE {
            return None;
        }
        let flags = raw.data.mouse.Anonymous.Anonymous.usButtonFlags as u32;
        first_button_message(flags)
    }

    fn take_outside_callbacks(
        message: u32,
        observed_root: usize,
    ) -> Vec<Box<dyn FnOnce(PointerObservation) + Send>> {
        let Ok(mut listeners) = listeners().lock() else {
            return Vec::new();
        };
        listeners
            .values_mut()
            .filter_map(|listener| {
                (pointer_decision(message, &listener.internal_surface_roots, observed_root)
                    == PointerDecision::CloseOutside)
                    .then(|| listener.callback.take())
                    .flatten()
            })
            .collect()
    }

    const fn first_button_message(flags: u32) -> Option<u32> {
        if flags & RI_MOUSE_BUTTON_1_DOWN != 0 {
            Some(WM_LBUTTONDOWN)
        } else if flags & RI_MOUSE_BUTTON_2_DOWN != 0 {
            Some(WM_RBUTTONDOWN)
        } else if flags & RI_MOUSE_BUTTON_3_DOWN != 0 {
            Some(WM_MBUTTONDOWN)
        } else if flags & (RI_MOUSE_BUTTON_4_DOWN | RI_MOUSE_BUTTON_5_DOWN) != 0 {
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
        fn raw_button_flags_map_to_pointer_messages() {
            assert_eq!(
                first_button_message(RI_MOUSE_BUTTON_1_DOWN),
                Some(WM_LBUTTONDOWN)
            );
            assert_eq!(
                first_button_message(RI_MOUSE_BUTTON_2_DOWN),
                Some(WM_RBUTTONDOWN)
            );
            assert_eq!(
                first_button_message(RI_MOUSE_BUTTON_3_DOWN),
                Some(WM_MBUTTONDOWN)
            );
            assert_eq!(
                first_button_message(RI_MOUSE_BUTTON_4_DOWN),
                Some(WM_XBUTTONDOWN)
            );
            assert_eq!(
                first_button_message(RI_MOUSE_BUTTON_5_DOWN),
                Some(WM_XBUTTONDOWN)
            );
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
            stop_worker(&mut slot).unwrap();
            assert!(slot.is_none());
        }

        #[test]
        fn stop_is_idempotent_without_worker() {
            let mut slot = None;
            stop_worker(&mut slot).unwrap();
            stop_worker(&mut slot).unwrap();
            assert!(slot.is_none());
        }
    }
}

#[cfg(windows)]
pub use platform::{
    start, stop, stop_all, PointerMonitorError, PointerMonitorOwner, PointerObservation,
};

#[cfg(not(windows))]
mod platform_fallback {
    use thiserror::Error;

    #[derive(Debug, Error)]
    #[error("surface outside-pointer monitoring is unavailable on this platform")]
    pub struct PointerMonitorError;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum PointerMonitorOwner {
        Clipboard,
        ToolMenu,
    }

    pub fn start(
        _owner: PointerMonitorOwner,
        _internal_surface_roots: Vec<usize>,
        _callback: impl FnOnce(PointerObservation) + Send + 'static,
    ) -> Result<(), PointerMonitorError> {
        Err(PointerMonitorError)
    }

    pub fn stop(_owner: PointerMonitorOwner) -> Result<(), PointerMonitorError> {
        Ok(())
    }

    pub fn stop_all() -> Result<(), PointerMonitorError> {
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
pub use platform_fallback::{
    start, stop, stop_all, PointerMonitorError, PointerMonitorOwner, PointerObservation,
};
