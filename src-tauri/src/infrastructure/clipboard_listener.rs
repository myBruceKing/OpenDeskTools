use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use thiserror::Error;

use super::clipboard::{
    ClipboardCaptureMetadata, ClipboardError, ClipboardService, JS_MAX_SAFE_INTEGER,
};

const STATUS_STOPPED: u8 = 0;
const STATUS_RUNNING: u8 = 1;
const STATUS_UNAVAILABLE: u8 = 2;

pub type ClipboardHistoryEventSink = Arc<dyn Fn() + Send + Sync + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardListenerStatus {
    Running,
    Unavailable,
    Stopped,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ClipboardListenerError {
    #[cfg(not(windows))]
    #[error("clipboard listener is unavailable on this platform")]
    UnsupportedPlatform,
    #[error("clipboard listener state lock is poisoned")]
    StateLockPoisoned,
    #[error("failed to start clipboard listener thread: {0}")]
    ThreadStart(&'static str),
    #[error("clipboard listener startup did not complete")]
    StartupChannel,
    #[error("Windows clipboard listener operation failed: {0}")]
    WindowsApi(&'static str),
    #[error("clipboard listener thread panicked: {0}")]
    ThreadPanicked(&'static str),
}

#[cfg(windows)]
struct ListenerControl {
    window: usize,
    message_thread_id: u32,
    message_thread: std::thread::JoinHandle<()>,
    worker_thread: std::thread::JoinHandle<()>,
}

pub struct ClipboardListenerManager {
    status: Arc<AtomicU8>,
    #[cfg(windows)]
    control: Mutex<Option<ListenerControl>>,
    #[cfg(not(windows))]
    control: Mutex<()>,
}

impl fmt::Debug for ClipboardListenerManager {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClipboardListenerManager")
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}

impl Default for ClipboardListenerManager {
    fn default() -> Self {
        Self {
            status: Arc::new(AtomicU8::new(STATUS_STOPPED)),
            #[cfg(windows)]
            control: Mutex::new(None),
            #[cfg(not(windows))]
            control: Mutex::new(()),
        }
    }
}

impl ClipboardListenerManager {
    pub fn status(&self) -> ClipboardListenerStatus {
        match self.status.load(Ordering::Acquire) {
            STATUS_RUNNING => ClipboardListenerStatus::Running,
            STATUS_UNAVAILABLE => ClipboardListenerStatus::Unavailable,
            _ => ClipboardListenerStatus::Stopped,
        }
    }

    pub fn start(
        &self,
        service: Arc<ClipboardService>,
        sink: ClipboardHistoryEventSink,
    ) -> Result<(), ClipboardListenerError> {
        #[cfg(windows)]
        {
            platform::start(self, service, sink)
        }
        #[cfg(not(windows))]
        {
            let _ = (service, sink);
            self.status.store(STATUS_UNAVAILABLE, Ordering::Release);
            Err(ClipboardListenerError::UnsupportedPlatform)
        }
    }

    pub fn stop(&self) -> Result<(), ClipboardListenerError> {
        #[cfg(windows)]
        {
            platform::stop(self)
        }
        #[cfg(not(windows))]
        {
            let _guard = self
                .control
                .lock()
                .map_err(|_| ClipboardListenerError::StateLockPoisoned)?;
            self.status.store(STATUS_STOPPED, Ordering::Release);
            Ok(())
        }
    }
}

impl Drop for ClipboardListenerManager {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn should_post_to_message_window(message_thread_finished: bool) -> bool {
    !message_thread_finished
}

fn join_listener_threads(
    message_thread: std::thread::JoinHandle<()>,
    worker_thread: std::thread::JoinHandle<()>,
) -> Result<(), ClipboardListenerError> {
    let message_result = message_thread
        .join()
        .map_err(|_| ClipboardListenerError::ThreadPanicked("message"));
    let worker_result = worker_thread
        .join()
        .map_err(|_| ClipboardListenerError::ThreadPanicked("worker"));
    message_result.and(worker_result)
}

fn detach_listener_reaper(
    message_thread: std::thread::JoinHandle<()>,
    worker_thread: std::thread::JoinHandle<()>,
) {
    let _ = std::thread::Builder::new()
        .name("clipboard-listener-reaper".to_owned())
        .spawn(move || {
            let _ = join_listener_threads(message_thread, worker_thread);
        });
}

trait ClipboardReader {
    fn sequence_number(&mut self) -> u32;
    fn read_text(&mut self) -> Result<Option<String>, ()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListenerRecordOutcome {
    Recorded { retained: bool },
    PermanentReject,
    RetryableFailure,
}

trait ClipboardRecordTarget {
    fn record_listener_text(&self, text: String, captured_at_ms: u64) -> ListenerRecordOutcome;
}

impl ClipboardRecordTarget for ClipboardService {
    fn record_listener_text(&self, text: String, captured_at_ms: u64) -> ListenerRecordOutcome {
        match self.record_text(
            text,
            ClipboardCaptureMetadata {
                captured_at_ms,
                source_application: None,
                source_process: None,
            },
        ) {
            Ok(result) => ListenerRecordOutcome::Recorded {
                retained: result.retained,
            },
            Err(ClipboardError::Storage(_) | ClipboardError::CorruptRecord) => {
                ListenerRecordOutcome::RetryableFailure
            }
            Err(_) => ListenerRecordOutcome::PermanentReject,
        }
    }
}

fn is_duplicate_sequence(last_sequence: Option<u32>, current_sequence: u32) -> bool {
    current_sequence != 0 && last_sequence == Some(current_sequence)
}

fn process_clipboard_notification<R: ClipboardReader, T: ClipboardRecordTarget>(
    reader: &mut R,
    last_sequence: &mut Option<u32>,
    target: &T,
    sink: &ClipboardHistoryEventSink,
    captured_at_ms: u64,
) {
    let sequence = reader.sequence_number();
    if is_duplicate_sequence(*last_sequence, sequence) {
        return;
    }

    let text = match reader.read_text() {
        Ok(text) => text,
        Err(()) => return,
    };
    let Some(text) = text.filter(|text| !text.is_empty()) else {
        commit_sequence(last_sequence, sequence);
        return;
    };

    match target.record_listener_text(text, captured_at_ms) {
        ListenerRecordOutcome::Recorded { retained } => {
            commit_sequence(last_sequence, sequence);
            if retained {
                sink();
            }
        }
        ListenerRecordOutcome::PermanentReject => commit_sequence(last_sequence, sequence),
        ListenerRecordOutcome::RetryableFailure => {}
    }
}

fn commit_sequence(last_sequence: &mut Option<u32>, sequence: u32) {
    if sequence != 0 {
        *last_sequence = Some(sequence);
    }
}

fn current_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis())
                .unwrap_or(JS_MAX_SAFE_INTEGER)
                .min(JS_MAX_SAFE_INTEGER)
        })
}

fn decode_null_terminated_utf16(units: &[u16]) -> Result<Option<String>, ()> {
    let terminator = units.iter().position(|unit| *unit == 0).ok_or(())?;
    if terminator == 0 {
        return Ok(None);
    }
    String::from_utf16(&units[..terminator])
        .map(Some)
        .map_err(|_| ())
}

#[cfg(windows)]
mod platform {
    use std::mem;
    use std::ptr::{null, null_mut};
    use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
    use std::thread;
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{HGLOBAL, HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::DataExchange::{
        AddClipboardFormatListener, CloseClipboard, GetClipboardData, GetClipboardSequenceNumber,
        IsClipboardFormatAvailable, OpenClipboard, RemoveClipboardFormatListener,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
        GetWindowLongPtrW, PeekMessageW, PostMessageW, PostQuitMessage, PostThreadMessageW,
        RegisterClassW, SetWindowLongPtrW, TranslateMessage, UnregisterClassW, CREATESTRUCTW,
        GWLP_USERDATA, HWND_MESSAGE, MSG, PM_NOREMOVE, WM_APP, WM_CLIPBOARDUPDATE, WM_NCCREATE,
        WM_QUIT, WNDCLASSW,
    };

    use super::*;
    use crate::infrastructure::clipboard::MAX_TEXT_BYTES;

    const STOP_MESSAGE: u32 = WM_APP + 0x31;
    const OPEN_ATTEMPTS: usize = 5;
    const OPEN_RETRY_DELAY: Duration = Duration::from_millis(12);
    const STARTUP_TIMEOUT: Duration = Duration::from_secs(1);
    const WINDOW_CLASS_NAME: &[u16] = &[
        79, 112, 101, 110, 68, 101, 115, 107, 84, 111, 111, 108, 115, 67, 108, 105, 112, 98, 111,
        97, 114, 100, 76, 105, 115, 116, 101, 110, 101, 114, 0,
    ];

    #[derive(Debug, Clone, Copy)]
    struct WindowIdentity {
        hwnd: usize,
        thread_id: u32,
    }

    pub(super) fn start(
        manager: &ClipboardListenerManager,
        service: Arc<ClipboardService>,
        sink: ClipboardHistoryEventSink,
    ) -> Result<(), ClipboardListenerError> {
        let mut control = manager
            .control
            .lock()
            .map_err(|_| ClipboardListenerError::StateLockPoisoned)?;
        if control.is_some() {
            return Ok(());
        }

        let (signal_sender, signal_receiver) = mpsc::sync_channel(1);
        let worker_status = Arc::clone(&manager.status);
        let worker_thread = thread::Builder::new()
            .name("clipboard-history-worker".to_owned())
            .spawn(move || {
                run_worker(signal_receiver, service, sink);
                let _ = worker_status.compare_exchange(
                    STATUS_RUNNING,
                    STATUS_UNAVAILABLE,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            })
            .map_err(|_| ClipboardListenerError::ThreadStart("worker"))?;

        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let message_thread_id = Arc::new(AtomicU32::new(0));
        let message_status = Arc::clone(&manager.status);
        let message_cancel = Arc::clone(&cancel);
        let published_thread_id = Arc::clone(&message_thread_id);
        let message_thread = match thread::Builder::new()
            .name("clipboard-message-window".to_owned())
            .spawn(move || {
                let _ = run_message_thread(
                    signal_sender,
                    ready_sender,
                    message_cancel,
                    published_thread_id,
                );
                let _ = message_status.compare_exchange(
                    STATUS_RUNNING,
                    STATUS_UNAVAILABLE,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
            }) {
            Ok(thread) => thread,
            Err(_) => {
                let _ = thread::Builder::new()
                    .name("clipboard-worker-reaper".to_owned())
                    .spawn(move || {
                        let _ = worker_thread.join();
                    });
                manager.status.store(STATUS_UNAVAILABLE, Ordering::Release);
                return Err(ClipboardListenerError::ThreadStart("message"));
            }
        };

        let identity = match ready_receiver.recv_timeout(STARTUP_TIMEOUT) {
            Ok(Ok(identity)) => identity,
            Ok(Err(error)) => {
                manager.status.store(STATUS_UNAVAILABLE, Ordering::Release);
                cancel_startup(&cancel, &message_thread_id, message_thread, worker_thread);
                return Err(error);
            }
            Err(_) => {
                manager.status.store(STATUS_UNAVAILABLE, Ordering::Release);
                cancel_startup(&cancel, &message_thread_id, message_thread, worker_thread);
                return Err(ClipboardListenerError::StartupChannel);
            }
        };

        manager.status.store(STATUS_RUNNING, Ordering::Release);
        if message_thread.is_finished() || worker_thread.is_finished() {
            manager.status.store(STATUS_UNAVAILABLE, Ordering::Release);
        }
        *control = Some(ListenerControl {
            window: identity.hwnd,
            message_thread_id: identity.thread_id,
            message_thread,
            worker_thread,
        });
        Ok(())
    }

    pub(super) fn stop(manager: &ClipboardListenerManager) -> Result<(), ClipboardListenerError> {
        let control = manager
            .control
            .lock()
            .map_err(|_| ClipboardListenerError::StateLockPoisoned)?
            .take();
        manager.status.store(STATUS_STOPPED, Ordering::Release);
        let Some(control) = control else {
            return Ok(());
        };

        if should_post_to_message_window(control.message_thread.is_finished()) {
            let posted = unsafe { PostMessageW(control.window as HWND, STOP_MESSAGE, 0, 0) } != 0;
            if !posted {
                unsafe {
                    PostThreadMessageW(control.message_thread_id, WM_QUIT, 0, 0);
                }
            }
        }
        join_listener_threads(control.message_thread, control.worker_thread)
    }

    fn cancel_startup(
        cancel: &AtomicBool,
        message_thread_id: &AtomicU32,
        message_thread: thread::JoinHandle<()>,
        worker_thread: thread::JoinHandle<()>,
    ) {
        cancel.store(true, Ordering::Release);
        let thread_id = message_thread_id.load(Ordering::Acquire);
        if thread_id != 0 && !message_thread.is_finished() {
            unsafe {
                PostThreadMessageW(thread_id, WM_QUIT, 0, 0);
            }
        }
        detach_listener_reaper(message_thread, worker_thread);
    }

    fn run_worker(
        signals: Receiver<()>,
        service: Arc<ClipboardService>,
        sink: ClipboardHistoryEventSink,
    ) {
        let mut reader = WindowsClipboardReader;
        let mut last_sequence = None;
        while signals.recv().is_ok() {
            process_clipboard_notification(
                &mut reader,
                &mut last_sequence,
                service.as_ref(),
                &sink,
                current_timestamp_ms(),
            );
        }
    }

    fn run_message_thread(
        signal_sender: SyncSender<()>,
        ready_sender: SyncSender<Result<WindowIdentity, ClipboardListenerError>>,
        cancel: Arc<AtomicBool>,
        published_thread_id: Arc<AtomicU32>,
    ) -> Result<(), ClipboardListenerError> {
        let mut queue_probe: MSG = unsafe { mem::zeroed() };
        unsafe {
            PeekMessageW(&mut queue_probe, null_mut(), 0, 0, PM_NOREMOVE);
        }
        let thread_id = unsafe { GetCurrentThreadId() };
        published_thread_id.store(thread_id, Ordering::Release);
        if cancel.load(Ordering::Acquire) {
            return Err(ClipboardListenerError::StartupChannel);
        }
        let sender = Box::new(signal_sender);
        let sender_pointer = (&*sender as *const SyncSender<()>).cast_mut().cast();
        let window = unsafe { create_listener_window(sender_pointer) };
        let window = match window {
            Ok(window) => window,
            Err(error) => {
                let _ = ready_sender.send(Err(error.clone()));
                return Err(error);
            }
        };
        let window = ListenerWindowGuard(window);
        if cancel.load(Ordering::Acquire) {
            return Err(ClipboardListenerError::StartupChannel);
        }
        if ready_sender
            .send(Ok(WindowIdentity {
                hwnd: window.0 as usize,
                thread_id,
            }))
            .is_err()
        {
            return Err(ClipboardListenerError::StartupChannel);
        }

        if cancel.load(Ordering::Acquire) {
            return Err(ClipboardListenerError::StartupChannel);
        }

        let message_result = unsafe { message_loop() };
        drop(window);
        message_result
    }

    unsafe fn create_listener_window(
        sender_pointer: *mut core::ffi::c_void,
    ) -> Result<HWND, ClipboardListenerError> {
        let module = unsafe { GetModuleHandleW(null()) };
        if module.is_null() {
            return Err(ClipboardListenerError::WindowsApi("GetModuleHandleW"));
        }
        let class = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: module,
            hIcon: null_mut(),
            hCursor: null_mut(),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: WINDOW_CLASS_NAME.as_ptr(),
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            return Err(ClipboardListenerError::WindowsApi("RegisterClassW"));
        }
        let window = unsafe {
            CreateWindowExW(
                0,
                WINDOW_CLASS_NAME.as_ptr(),
                WINDOW_CLASS_NAME.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                null_mut(),
                module,
                sender_pointer,
            )
        };
        if window.is_null() {
            unsafe { UnregisterClassW(WINDOW_CLASS_NAME.as_ptr(), module) };
            return Err(ClipboardListenerError::WindowsApi("CreateWindowExW"));
        }
        if unsafe { AddClipboardFormatListener(window) } == 0 {
            unsafe {
                DestroyWindow(window);
                UnregisterClassW(WINDOW_CLASS_NAME.as_ptr(), module);
            }
            return Err(ClipboardListenerError::WindowsApi(
                "AddClipboardFormatListener",
            ));
        }
        Ok(window)
    }

    unsafe fn message_loop() -> Result<(), ClipboardListenerError> {
        let mut message: MSG = unsafe { mem::zeroed() };
        loop {
            let result = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
            if result > 0 {
                unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            } else if result == 0 {
                return Ok(());
            } else {
                return Err(ClipboardListenerError::WindowsApi("GetMessageW"));
            }
        }
    }

    struct ListenerWindowGuard(HWND);

    impl Drop for ListenerWindowGuard {
        fn drop(&mut self) {
            unsafe { cleanup_listener_window(self.0) };
        }
    }

    unsafe fn cleanup_listener_window(window: HWND) {
        unsafe {
            RemoveClipboardFormatListener(window);
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
            DestroyWindow(window);
            let module = GetModuleHandleW(null());
            if !module.is_null() {
                UnregisterClassW(WINDOW_CLASS_NAME.as_ptr(), module);
            }
        }
    }

    unsafe extern "system" fn window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == WM_NCCREATE {
            let create = lparam as *const CREATESTRUCTW;
            if !create.is_null() {
                unsafe {
                    SetWindowLongPtrW(window, GWLP_USERDATA, (*create).lpCreateParams as isize);
                }
            }
        } else if message == WM_CLIPBOARDUPDATE {
            let sender =
                unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *const SyncSender<()>;
            if !sender.is_null() {
                match unsafe { &*sender }.try_send(()) {
                    Ok(()) | Err(TrySendError::Full(())) => {}
                    Err(TrySendError::Disconnected(())) => {}
                }
            }
            return 0;
        } else if message == STOP_MESSAGE {
            unsafe { PostQuitMessage(0) };
            return 0;
        }
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    trait ClipboardApi {
        fn is_unicode_text_available(&mut self) -> bool;
        fn open(&mut self) -> bool;
        fn close(&mut self);
        fn get_unicode_text_data(&mut self) -> Option<usize>;
        fn global_size(&mut self, handle: usize) -> usize;
        fn global_lock(&mut self, handle: usize) -> Option<*const u16>;
        fn global_unlock(&mut self, handle: usize);
    }

    struct SystemClipboardApi;

    impl ClipboardApi for SystemClipboardApi {
        fn is_unicode_text_available(&mut self) -> bool {
            (unsafe { IsClipboardFormatAvailable(u32::from(CF_UNICODETEXT)) }) != 0
        }

        fn open(&mut self) -> bool {
            (unsafe { OpenClipboard(null_mut()) }) != 0
        }

        fn close(&mut self) {
            unsafe {
                CloseClipboard();
            }
        }

        fn get_unicode_text_data(&mut self) -> Option<usize> {
            let handle = unsafe { GetClipboardData(u32::from(CF_UNICODETEXT)) };
            (!handle.is_null()).then_some(handle as usize)
        }

        fn global_size(&mut self, handle: usize) -> usize {
            unsafe { GlobalSize(handle as HGLOBAL) }
        }

        fn global_lock(&mut self, handle: usize) -> Option<*const u16> {
            let data = unsafe { GlobalLock(handle as HGLOBAL) } as *const u16;
            (!data.is_null()).then_some(data)
        }

        fn global_unlock(&mut self, handle: usize) {
            unsafe {
                GlobalUnlock(handle as HGLOBAL);
            }
        }
    }

    struct WindowsClipboardReader;

    impl ClipboardReader for WindowsClipboardReader {
        fn sequence_number(&mut self) -> u32 {
            unsafe { GetClipboardSequenceNumber() }
        }

        fn read_text(&mut self) -> Result<Option<String>, ()> {
            read_text_with_retries(&mut SystemClipboardApi, || thread::sleep(OPEN_RETRY_DELAY))
        }
    }

    struct ClipboardOpenGuard<'a, A: ClipboardApi> {
        api: &'a mut A,
    }

    impl<'a, A: ClipboardApi> ClipboardOpenGuard<'a, A> {
        fn try_open(api: &'a mut A) -> Result<Self, ()> {
            if api.open() {
                Ok(Self { api })
            } else {
                Err(())
            }
        }
    }

    impl<A: ClipboardApi> std::ops::Deref for ClipboardOpenGuard<'_, A> {
        type Target = A;

        fn deref(&self) -> &Self::Target {
            self.api
        }
    }

    impl<A: ClipboardApi> std::ops::DerefMut for ClipboardOpenGuard<'_, A> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            self.api
        }
    }

    impl<A: ClipboardApi> Drop for ClipboardOpenGuard<'_, A> {
        fn drop(&mut self) {
            self.api.close();
        }
    }

    struct GlobalLockGuard<'a, A: ClipboardApi> {
        api: &'a mut A,
        handle: usize,
        pointer: *const u16,
    }

    impl<'a, A: ClipboardApi> GlobalLockGuard<'a, A> {
        fn try_lock(api: &'a mut A, handle: usize) -> Result<Self, ()> {
            let pointer = api.global_lock(handle).ok_or(())?;
            Ok(Self {
                api,
                handle,
                pointer,
            })
        }

        unsafe fn units(&self, count: usize) -> &[u16] {
            unsafe { std::slice::from_raw_parts(self.pointer, count) }
        }
    }

    impl<A: ClipboardApi> Drop for GlobalLockGuard<'_, A> {
        fn drop(&mut self) {
            self.api.global_unlock(self.handle);
        }
    }

    fn read_text_with_retries<A, F>(
        api: &mut A,
        mut wait_before_retry: F,
    ) -> Result<Option<String>, ()>
    where
        A: ClipboardApi,
        F: FnMut(),
    {
        if !api.is_unicode_text_available() {
            return Ok(None);
        }

        for attempt in 0..OPEN_ATTEMPTS {
            if let Ok(text) = read_text_attempt(api) {
                return Ok(text);
            }
            if attempt + 1 < OPEN_ATTEMPTS {
                wait_before_retry();
            }
        }
        Err(())
    }

    fn read_text_attempt<A: ClipboardApi>(api: &mut A) -> Result<Option<String>, ()> {
        let mut clipboard = ClipboardOpenGuard::try_open(api)?;
        let handle = clipboard.get_unicode_text_data().ok_or(())?;
        let allocation_bytes = clipboard.global_size(handle);
        if allocation_bytes < mem::size_of::<u16>() {
            return Err(());
        }
        let available_units = allocation_bytes / mem::size_of::<u16>();
        let bounded_units = available_units.min(MAX_TEXT_BYTES.saturating_add(1));
        let locked = GlobalLockGuard::try_lock(&mut *clipboard, handle)?;
        let units = unsafe { locked.units(bounded_units) };
        decode_null_terminated_utf16(units)
    }

    #[cfg(test)]
    mod tests {
        use std::collections::VecDeque;

        use super::*;

        struct FakeClipboardApi {
            available: bool,
            open_results: VecDeque<bool>,
            data_results: VecDeque<Option<usize>>,
            size_results: VecDeque<usize>,
            lock_results: VecDeque<bool>,
            buffer: Vec<u16>,
            opens: usize,
            closes: usize,
            unlocks: usize,
        }

        impl FakeClipboardApi {
            fn text(value: &str) -> Self {
                let mut buffer = value.encode_utf16().collect::<Vec<_>>();
                buffer.push(0);
                Self {
                    available: true,
                    open_results: VecDeque::new(),
                    data_results: VecDeque::new(),
                    size_results: VecDeque::new(),
                    lock_results: VecDeque::new(),
                    buffer,
                    opens: 0,
                    closes: 0,
                    unlocks: 0,
                }
            }

            fn allocation_bytes(&self) -> usize {
                self.buffer.len() * mem::size_of::<u16>()
            }
        }

        impl ClipboardApi for FakeClipboardApi {
            fn is_unicode_text_available(&mut self) -> bool {
                self.available
            }

            fn open(&mut self) -> bool {
                self.opens += 1;
                self.open_results.pop_front().unwrap_or(true)
            }

            fn close(&mut self) {
                self.closes += 1;
            }

            fn get_unicode_text_data(&mut self) -> Option<usize> {
                self.data_results.pop_front().unwrap_or(Some(1))
            }

            fn global_size(&mut self, _handle: usize) -> usize {
                let fallback = self.allocation_bytes();
                self.size_results.pop_front().unwrap_or(fallback)
            }

            fn global_lock(&mut self, _handle: usize) -> Option<*const u16> {
                if self.lock_results.pop_front().unwrap_or(true) {
                    Some(self.buffer.as_ptr())
                } else {
                    None
                }
            }

            fn global_unlock(&mut self, _handle: usize) {
                self.unlocks += 1;
            }
        }

        fn read_without_wait(api: &mut FakeClipboardApi) -> (Result<Option<String>, ()>, usize) {
            let mut waits = 0;
            let result = read_text_with_retries(api, || waits += 1);
            (result, waits)
        }

        #[test]
        fn absent_format_is_a_terminal_none_without_open_or_retry() {
            let mut api = FakeClipboardApi::text("ignored");
            api.available = false;

            let (result, waits) = read_without_wait(&mut api);

            assert_eq!(result, Ok(None));
            assert_eq!(api.opens, 0);
            assert_eq!(api.closes, 0);
            assert_eq!(api.unlocks, 0);
            assert_eq!(waits, 0);
        }

        #[test]
        fn transient_get_data_failure_retries_full_opened_attempt_and_balances_close() {
            let mut api = FakeClipboardApi::text("captured");
            api.data_results = VecDeque::from([None, Some(1)]);

            let (result, waits) = read_without_wait(&mut api);

            assert_eq!(result, Ok(Some("captured".to_owned())));
            assert_eq!(api.opens, 2);
            assert_eq!(api.closes, 2);
            assert_eq!(api.unlocks, 1);
            assert_eq!(waits, 1);
        }

        #[test]
        fn transient_open_failure_retries_without_closing_an_unopened_clipboard() {
            let mut api = FakeClipboardApi::text("opened");
            api.open_results = VecDeque::from([false, true]);

            let (result, waits) = read_without_wait(&mut api);

            assert_eq!(result, Ok(Some("opened".to_owned())));
            assert_eq!(api.opens, 2);
            assert_eq!(api.closes, 1);
            assert_eq!(api.unlocks, 1);
            assert_eq!(waits, 1);
        }

        #[test]
        fn transient_size_and_lock_failures_each_retry_the_complete_attempt() {
            let mut size_api = FakeClipboardApi::text("size");
            let valid_size = size_api.allocation_bytes();
            size_api.size_results = VecDeque::from([0, 1, valid_size]);
            let (size_result, size_waits) = read_without_wait(&mut size_api);
            assert_eq!(size_result, Ok(Some("size".to_owned())));
            assert_eq!(
                (size_api.opens, size_api.closes, size_api.unlocks),
                (3, 3, 1)
            );
            assert_eq!(size_waits, 2);

            let mut lock_api = FakeClipboardApi::text("lock");
            lock_api.lock_results = VecDeque::from([false, true]);
            let (lock_result, lock_waits) = read_without_wait(&mut lock_api);
            assert_eq!(lock_result, Ok(Some("lock".to_owned())));
            assert_eq!(
                (lock_api.opens, lock_api.closes, lock_api.unlocks),
                (2, 2, 1)
            );
            assert_eq!(lock_waits, 1);
        }

        #[test]
        fn persistent_get_failure_exhausts_five_attempts_with_every_open_closed() {
            let mut api = FakeClipboardApi::text("never-read");
            api.data_results = VecDeque::from([None; OPEN_ATTEMPTS]);

            let (result, waits) = read_without_wait(&mut api);

            assert_eq!(result, Err(()));
            assert_eq!(api.opens, OPEN_ATTEMPTS);
            assert_eq!(api.closes, OPEN_ATTEMPTS);
            assert_eq!(api.unlocks, 0);
            assert_eq!(waits, OPEN_ATTEMPTS - 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tempfile::tempdir;

    use super::*;
    use crate::infrastructure::storage::StorageService;

    struct FakeReader {
        sequences: VecDeque<u32>,
        values: VecDeque<Result<Option<String>, ()>>,
        reads: usize,
    }

    struct FakeRecordTarget {
        outcomes: Mutex<VecDeque<ListenerRecordOutcome>>,
        calls: AtomicUsize,
    }

    impl ClipboardRecordTarget for FakeRecordTarget {
        fn record_listener_text(
            &self,
            _text: String,
            _captured_at_ms: u64,
        ) -> ListenerRecordOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.outcomes.lock().unwrap().pop_front().unwrap()
        }
    }

    impl ClipboardReader for FakeReader {
        fn sequence_number(&mut self) -> u32 {
            self.sequences.pop_front().unwrap()
        }

        fn read_text(&mut self) -> Result<Option<String>, ()> {
            self.reads += 1;
            self.values.pop_front().unwrap()
        }
    }

    fn service() -> (tempfile::TempDir, ClipboardService) {
        let temp = tempdir().unwrap();
        let storage = Arc::new(StorageService::initialize(temp.path()).unwrap());
        (temp, ClipboardService::initialize(storage))
    }

    #[test]
    fn duplicate_nonzero_sequence_is_skipped_but_zero_sequence_is_never_deduplicated() {
        assert!(is_duplicate_sequence(Some(7), 7));
        assert!(!is_duplicate_sequence(Some(7), 8));
        assert!(!is_duplicate_sequence(Some(0), 0));
        assert!(!is_duplicate_sequence(None, 0));
    }

    #[test]
    fn utf16_adapter_requires_termination_and_rejects_empty_or_invalid_text() {
        assert_eq!(
            decode_null_terminated_utf16(&['你' as u16, '好' as u16, 0, 99]),
            Ok(Some("你好".to_owned()))
        );
        assert_eq!(decode_null_terminated_utf16(&[0, 99]), Ok(None));
        assert_eq!(decode_null_terminated_utf16(&['a' as u16]), Err(()));
        assert_eq!(decode_null_terminated_utf16(&[0xD800, 0]), Err(()));
    }

    #[test]
    fn adapter_records_once_emits_only_after_retention_and_keeps_sources_unknown() {
        let (_temp, service) = service();
        let emitted = Arc::new(AtomicUsize::new(0));
        let emitted_for_sink = Arc::clone(&emitted);
        let sink: ClipboardHistoryEventSink = Arc::new(move || {
            emitted_for_sink.fetch_add(1, Ordering::SeqCst);
        });
        let mut reader = FakeReader {
            sequences: VecDeque::from([10, 10]),
            values: VecDeque::from([Ok(Some("captured".to_owned()))]),
            reads: 0,
        };
        let mut last_sequence = None;

        process_clipboard_notification(&mut reader, &mut last_sequence, &service, &sink, 123);
        process_clipboard_notification(&mut reader, &mut last_sequence, &service, &sink, 124);

        assert_eq!(reader.reads, 1);
        assert_eq!(emitted.load(Ordering::SeqCst), 1);
        let item = service
            .history(super::super::clipboard::ClipboardHistoryQuery {
                favorites_only: false,
                search: None,
                limit: 100,
            })
            .unwrap()
            .items
            .pop()
            .unwrap();
        assert_eq!(item.text_content.as_deref(), Some("captured"));
        assert_eq!(item.source_application, None);
        assert_eq!(item.source_process, None);
    }

    #[test]
    fn failed_read_is_retryable_for_the_same_sequence_and_empty_values_do_not_emit() {
        let (_temp, service) = service();
        let emitted = Arc::new(AtomicUsize::new(0));
        let sink: ClipboardHistoryEventSink = {
            let emitted = Arc::clone(&emitted);
            Arc::new(move || {
                emitted.fetch_add(1, Ordering::SeqCst);
            })
        };
        let mut reader = FakeReader {
            sequences: VecDeque::from([5, 5, 0]),
            values: VecDeque::from([Err(()), Ok(None), Ok(Some(String::new()))]),
            reads: 0,
        };
        let mut last_sequence = None;

        for timestamp in [1, 2, 3] {
            process_clipboard_notification(
                &mut reader,
                &mut last_sequence,
                &service,
                &sink,
                timestamp,
            );
        }

        assert_eq!(reader.reads, 3);
        assert_eq!(emitted.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn retryable_storage_failure_does_not_commit_sequence_and_same_sequence_can_succeed() {
        let target = FakeRecordTarget {
            outcomes: Mutex::new(VecDeque::from([
                ListenerRecordOutcome::RetryableFailure,
                ListenerRecordOutcome::Recorded { retained: true },
            ])),
            calls: AtomicUsize::new(0),
        };
        let emitted = Arc::new(AtomicUsize::new(0));
        let sink: ClipboardHistoryEventSink = {
            let emitted = Arc::clone(&emitted);
            Arc::new(move || {
                emitted.fetch_add(1, Ordering::SeqCst);
            })
        };
        let mut reader = FakeReader {
            sequences: VecDeque::from([44, 44]),
            values: VecDeque::from([Ok(Some("retry".to_owned())), Ok(Some("retry".to_owned()))]),
            reads: 0,
        };
        let mut last_sequence = None;

        process_clipboard_notification(&mut reader, &mut last_sequence, &target, &sink, 1);
        assert_eq!(last_sequence, None);
        process_clipboard_notification(&mut reader, &mut last_sequence, &target, &sink, 2);

        assert_eq!(last_sequence, Some(44));
        assert_eq!(target.calls.load(Ordering::SeqCst), 2);
        assert_eq!(emitted.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn permanent_record_rejection_commits_sequence_without_emitting() {
        let target = FakeRecordTarget {
            outcomes: Mutex::new(VecDeque::from([ListenerRecordOutcome::PermanentReject])),
            calls: AtomicUsize::new(0),
        };
        let sink: ClipboardHistoryEventSink = Arc::new(|| panic!("must not emit"));
        let mut reader = FakeReader {
            sequences: VecDeque::from([9]),
            values: VecDeque::from([Ok(Some("too-large-or-invalid".to_owned()))]),
            reads: 0,
        };
        let mut last_sequence = None;

        process_clipboard_notification(&mut reader, &mut last_sequence, &target, &sink, 1);

        assert_eq!(last_sequence, Some(9));
    }

    #[test]
    fn startup_timeout_path_returns_without_joining_and_reaper_eventually_joins_both_threads() {
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_exited = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = std::sync::mpsc::channel::<()>();
        let message_cancel = Arc::clone(&cancel);
        let message_thread = std::thread::spawn(move || {
            while !message_cancel.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            drop(sender);
        });
        let worker_exit = Arc::clone(&worker_exited);
        let worker_thread = std::thread::spawn(move || {
            let _ = receiver.recv();
            worker_exit.store(true, Ordering::Release);
        });

        let started = std::time::Instant::now();
        cancel.store(true, Ordering::Release);
        detach_listener_reaper(message_thread, worker_thread);
        assert!(started.elapsed() < std::time::Duration::from_millis(100));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !worker_exited.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(worker_exited.load(Ordering::Acquire));
    }

    #[test]
    fn message_panic_still_joins_worker_and_finished_thread_skips_stale_window_post() {
        let worker_exited = Arc::new(AtomicBool::new(false));
        let message_thread = std::thread::spawn(|| panic!("simulated message panic"));
        let worker_exit = Arc::clone(&worker_exited);
        let worker_thread = std::thread::spawn(move || {
            worker_exit.store(true, Ordering::Release);
        });

        let error = join_listener_threads(message_thread, worker_thread).unwrap_err();

        assert_eq!(error, ClipboardListenerError::ThreadPanicked("message"));
        assert!(worker_exited.load(Ordering::Acquire));
        assert!(!should_post_to_message_window(true));
        assert!(should_post_to_message_window(false));
    }

    #[test]
    fn manager_defaults_to_stopped_and_stop_is_idempotent_without_starting_os_listener() {
        let manager = ClipboardListenerManager::default();
        assert_eq!(manager.status(), ClipboardListenerStatus::Stopped);
        manager.stop().unwrap();
        manager.stop().unwrap();
        assert_eq!(manager.status(), ClipboardListenerStatus::Stopped);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "registers a real process clipboard listener; run explicitly and serially"]
    fn windows_listener_registration_and_stop_smoke_test() {
        static SERIAL: Mutex<()> = Mutex::new(());
        let _serial = SERIAL.lock().unwrap();
        let temp = tempdir().unwrap();
        let storage = Arc::new(StorageService::initialize(temp.path()).unwrap());
        let service = Arc::new(ClipboardService::initialize(storage));
        let manager = ClipboardListenerManager::default();

        manager.start(service, Arc::new(|| {})).unwrap();
        assert_eq!(manager.status(), ClipboardListenerStatus::Running);
        manager.stop().unwrap();
        assert_eq!(manager.status(), ClipboardListenerStatus::Stopped);
        manager.stop().unwrap();
    }
}
