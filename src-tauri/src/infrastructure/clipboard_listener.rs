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

    fn source_metadata(&mut self) -> ClipboardSourceMetadata {
        ClipboardSourceMetadata::default()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ClipboardSourceMetadata {
    source_application: Option<String>,
    source_process: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListenerRecordOutcome {
    Recorded { retained: bool },
    PermanentReject,
    RetryableFailure,
}

trait ClipboardRecordTarget {
    fn record_listener_text(
        &self,
        text: String,
        captured_at_ms: u64,
        source: ClipboardSourceMetadata,
    ) -> ListenerRecordOutcome;
}

impl ClipboardRecordTarget for ClipboardService {
    fn record_listener_text(
        &self,
        text: String,
        captured_at_ms: u64,
        source: ClipboardSourceMetadata,
    ) -> ListenerRecordOutcome {
        match self.record_text(
            text,
            ClipboardCaptureMetadata {
                captured_at_ms,
                source_application: source.source_application,
                source_process: source.source_process,
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
    let source = reader.source_metadata();
    let final_sequence = reader.sequence_number();
    let Some(sequence) = stable_sequence(sequence, final_sequence) else {
        return;
    };
    let Some(text) = text.filter(|text| !text.is_empty()) else {
        commit_sequence(last_sequence, sequence);
        return;
    };

    match target.record_listener_text(text, captured_at_ms, source) {
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

fn stable_sequence(initial: u32, final_sequence: u32) -> Option<u32> {
    if initial != 0 && final_sequence != 0 {
        (initial == final_sequence).then_some(initial)
    } else if final_sequence != 0 {
        Some(final_sequence)
    } else {
        Some(initial)
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

    use windows_sys::Win32::Foundation::{
        CloseHandle, HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM,
    };
    use windows_sys::Win32::System::DataExchange::{
        AddClipboardFormatListener, CloseClipboard, GetClipboardData, GetClipboardOwner,
        GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
        RemoveClipboardFormatListener,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
    use windows_sys::Win32::System::Threading::{
        GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
        GetWindowLongPtrW, GetWindowThreadProcessId, PeekMessageW, PostMessageW, PostQuitMessage,
        PostThreadMessageW, RegisterClassW, SetWindowLongPtrW, TranslateMessage, UnregisterClassW,
        CREATESTRUCTW, GWLP_USERDATA, HWND_MESSAGE, MSG, PM_NOREMOVE, WM_APP, WM_CLIPBOARDUPDATE,
        WM_NCCREATE, WM_QUIT, WNDCLASSW,
    };

    use super::*;
    use crate::infrastructure::clipboard::{
        MAX_SOURCE_APPLICATION_CHARS, MAX_SOURCE_PROCESS_CHARS, MAX_TEXT_BYTES,
    };

    const STOP_MESSAGE: u32 = WM_APP + 0x31;
    const OPEN_ATTEMPTS: usize = 5;
    const OPEN_RETRY_DELAY: Duration = Duration::from_millis(12);
    const STARTUP_TIMEOUT: Duration = Duration::from_secs(1);
    const PROCESS_PATH_CAPACITY: usize = 32_768;
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
        let mut reader = WindowsClipboardReader::default();
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

    #[derive(Default)]
    struct WindowsClipboardReader {
        source: ClipboardSourceMetadata,
    }

    impl ClipboardReader for WindowsClipboardReader {
        fn sequence_number(&mut self) -> u32 {
            unsafe { GetClipboardSequenceNumber() }
        }

        fn read_text(&mut self) -> Result<Option<String>, ()> {
            self.source = resolve_clipboard_source(&mut SystemProcessSourceApi);
            read_text_with_retries(&mut SystemClipboardApi, || thread::sleep(OPEN_RETRY_DELAY))
        }

        fn source_metadata(&mut self) -> ClipboardSourceMetadata {
            mem::take(&mut self.source)
        }
    }

    trait ProcessSourceApi {
        fn clipboard_owner(&mut self) -> Option<usize>;
        fn process_id(&mut self, owner: usize) -> Option<u32>;
        fn open_process(&mut self, process_id: u32) -> Option<usize>;
        fn query_process_image_path(&mut self, process: usize, buffer: &mut [u16])
            -> Option<usize>;
        fn close_process(&mut self, process: usize);
    }

    struct SystemProcessSourceApi;

    impl ProcessSourceApi for SystemProcessSourceApi {
        fn clipboard_owner(&mut self) -> Option<usize> {
            let owner = unsafe { GetClipboardOwner() };
            (!owner.is_null()).then_some(owner as usize)
        }

        fn process_id(&mut self, owner: usize) -> Option<u32> {
            let mut process_id = 0_u32;
            let thread_id = unsafe { GetWindowThreadProcessId(owner as HWND, &mut process_id) };
            (thread_id != 0 && process_id != 0).then_some(process_id)
        }

        fn open_process(&mut self, process_id: u32) -> Option<usize> {
            let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
            (!process.is_null()).then_some(process as usize)
        }

        fn query_process_image_path(
            &mut self,
            process: usize,
            buffer: &mut [u16],
        ) -> Option<usize> {
            let mut size = u32::try_from(buffer.len()).ok()?;
            let succeeded = (unsafe {
                QueryFullProcessImageNameW(process as HANDLE, 0, buffer.as_mut_ptr(), &mut size)
            }) != 0;
            succeeded.then_some(size as usize)
        }

        fn close_process(&mut self, process: usize) {
            unsafe {
                CloseHandle(process as HANDLE);
            }
        }
    }

    struct ProcessHandleGuard<'a, A: ProcessSourceApi> {
        api: &'a mut A,
        handle: usize,
    }

    impl<'a, A: ProcessSourceApi> ProcessHandleGuard<'a, A> {
        fn try_open(api: &'a mut A, process_id: u32) -> Option<Self> {
            let handle = api.open_process(process_id)?;
            Some(Self { api, handle })
        }
    }

    impl<A: ProcessSourceApi> Drop for ProcessHandleGuard<'_, A> {
        fn drop(&mut self) {
            self.api.close_process(self.handle);
        }
    }

    fn resolve_clipboard_source<A: ProcessSourceApi>(api: &mut A) -> ClipboardSourceMetadata {
        let Some(owner) = api.clipboard_owner() else {
            return ClipboardSourceMetadata::default();
        };
        resolve_source_for_owner(api, owner)
    }

    fn resolve_source_for_owner<A: ProcessSourceApi>(
        api: &mut A,
        owner: usize,
    ) -> ClipboardSourceMetadata {
        let Some(process_id) = api.process_id(owner) else {
            return ClipboardSourceMetadata::default();
        };
        let Some(process) = ProcessHandleGuard::try_open(api, process_id) else {
            return ClipboardSourceMetadata::default();
        };
        let mut buffer = vec![0_u16; PROCESS_PATH_CAPACITY];
        let Some(written) = process
            .api
            .query_process_image_path(process.handle, &mut buffer)
        else {
            return ClipboardSourceMetadata::default();
        };
        if written == 0 || written >= buffer.len() {
            return ClipboardSourceMetadata::default();
        }
        let Ok(path) = String::from_utf16(&buffer[..written]) else {
            return ClipboardSourceMetadata::default();
        };
        source_metadata_from_image_path(&path)
    }

    #[cfg(test)]
    pub(super) fn resolve_system_source_for_owner(owner: usize) -> ClipboardSourceMetadata {
        resolve_source_for_owner(&mut SystemProcessSourceApi, owner)
    }

    fn source_metadata_from_image_path(path: &str) -> ClipboardSourceMetadata {
        let normalized = path.replace('\\', "/");
        let Some(file_name) = normalized.rsplit('/').next().filter(|name| {
            !name.is_empty() && *name != "." && *name != ".." && !name.chars().any(char::is_control)
        }) else {
            return ClipboardSourceMetadata::default();
        };
        let application = file_name
            .rsplit_once('.')
            .filter(|(stem, extension)| !stem.is_empty() && !extension.is_empty())
            .map_or(file_name, |(stem, _)| stem);
        if file_name.chars().count() > MAX_SOURCE_PROCESS_CHARS
            || application.is_empty()
            || application.chars().count() > MAX_SOURCE_APPLICATION_CHARS
        {
            return ClipboardSourceMetadata::default();
        }
        ClipboardSourceMetadata {
            source_application: Some(application.to_owned()),
            source_process: Some(file_name.to_owned()),
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

        struct FakeProcessSourceApi {
            owner: Option<usize>,
            process_id: Option<u32>,
            process: Option<usize>,
            path: Option<Vec<u16>>,
            reported_length: Option<usize>,
            owner_calls: usize,
            process_id_calls: usize,
            open_calls: usize,
            query_calls: usize,
            close_calls: usize,
        }

        impl FakeProcessSourceApi {
            fn success(path: &str) -> Self {
                Self {
                    owner: Some(11),
                    process_id: Some(22),
                    process: Some(33),
                    path: Some(path.encode_utf16().collect()),
                    reported_length: None,
                    owner_calls: 0,
                    process_id_calls: 0,
                    open_calls: 0,
                    query_calls: 0,
                    close_calls: 0,
                }
            }
        }

        impl ProcessSourceApi for FakeProcessSourceApi {
            fn clipboard_owner(&mut self) -> Option<usize> {
                self.owner_calls += 1;
                self.owner
            }

            fn process_id(&mut self, _owner: usize) -> Option<u32> {
                self.process_id_calls += 1;
                self.process_id
            }

            fn open_process(&mut self, _process_id: u32) -> Option<usize> {
                self.open_calls += 1;
                self.process
            }

            fn query_process_image_path(
                &mut self,
                _process: usize,
                buffer: &mut [u16],
            ) -> Option<usize> {
                self.query_calls += 1;
                let path = self.path.as_ref()?;
                let copied = path.len().min(buffer.len().saturating_sub(1));
                buffer[..copied].copy_from_slice(&path[..copied]);
                buffer[copied] = 0;
                Some(self.reported_length.unwrap_or(path.len()))
            }

            fn close_process(&mut self, _process: usize) {
                self.close_calls += 1;
            }
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

        #[test]
        fn source_resolution_returns_only_safe_file_name_and_display_stem() {
            for path in [
                r"C:\Program Files\Notes\notepad.exe",
                "C:/Program Files/OpenDesk/Desk Tool.exe",
            ] {
                let mut api = FakeProcessSourceApi::success(path);

                let source = resolve_clipboard_source(&mut api);

                let expected_process = path
                    .replace('\\', "/")
                    .rsplit('/')
                    .next()
                    .unwrap()
                    .to_owned();
                let expected_application =
                    expected_process.strip_suffix(".exe").unwrap().to_owned();
                assert_eq!(source.source_process, Some(expected_process));
                assert_eq!(source.source_application, Some(expected_application));
                assert_eq!(api.close_calls, 1);
            }
        }

        #[test]
        fn no_owner_pid_failure_and_open_denial_are_harmless_and_do_not_close_invalid_handles() {
            let mut no_owner = FakeProcessSourceApi::success(r"C:\App\one.exe");
            no_owner.owner = None;
            assert_eq!(
                resolve_clipboard_source(&mut no_owner),
                ClipboardSourceMetadata::default()
            );
            assert_eq!(
                (
                    no_owner.process_id_calls,
                    no_owner.open_calls,
                    no_owner.close_calls
                ),
                (0, 0, 0)
            );

            let mut no_pid = FakeProcessSourceApi::success(r"C:\App\two.exe");
            no_pid.process_id = None;
            assert_eq!(
                resolve_clipboard_source(&mut no_pid),
                ClipboardSourceMetadata::default()
            );
            assert_eq!(
                (
                    no_pid.process_id_calls,
                    no_pid.open_calls,
                    no_pid.close_calls
                ),
                (1, 0, 0)
            );

            let mut denied = FakeProcessSourceApi::success(r"C:\App\three.exe");
            denied.process = None;
            assert_eq!(
                resolve_clipboard_source(&mut denied),
                ClipboardSourceMetadata::default()
            );
            assert_eq!(
                (denied.open_calls, denied.query_calls, denied.close_calls),
                (1, 0, 0)
            );
        }

        #[test]
        fn query_failure_empty_truncated_and_unsafe_paths_return_none_and_always_close_handle() {
            let mut cases = Vec::new();

            let mut query_failure = FakeProcessSourceApi::success(r"C:\App\failure.exe");
            query_failure.path = None;
            cases.push(query_failure);

            let mut empty = FakeProcessSourceApi::success("");
            empty.reported_length = Some(0);
            cases.push(empty);

            let mut truncated = FakeProcessSourceApi::success(r"C:\App\truncated.exe");
            truncated.reported_length = Some(PROCESS_PATH_CAPACITY);
            cases.push(truncated);

            let mut invalid_utf16 = FakeProcessSourceApi::success(r"C:\App\invalid.exe");
            invalid_utf16.path = Some(vec![0xD800]);
            cases.push(invalid_utf16);

            let control_name = format!("bad{}name.exe", '\u{1f}');
            cases.push(FakeProcessSourceApi::success(&format!(
                r"C:\App\{control_name}"
            )));

            for mut api in cases {
                assert_eq!(
                    resolve_clipboard_source(&mut api),
                    ClipboardSourceMetadata::default()
                );
                assert_eq!(api.close_calls, 1);
            }
        }

        #[test]
        fn source_name_limits_reject_values_that_storage_would_not_accept() {
            let long_process = format!("{}.exe", "p".repeat(MAX_SOURCE_PROCESS_CHARS));
            let mut process_api = FakeProcessSourceApi::success(&format!(r"C:\App\{long_process}"));
            assert_eq!(
                resolve_clipboard_source(&mut process_api),
                ClipboardSourceMetadata::default()
            );
            assert_eq!(process_api.close_calls, 1);

            let long_application = format!("{}.exe", "a".repeat(MAX_SOURCE_APPLICATION_CHARS + 1));
            let mut app_api = FakeProcessSourceApi::success(&format!(r"C:\App\{long_application}"));
            assert_eq!(
                resolve_clipboard_source(&mut app_api),
                ClipboardSourceMetadata::default()
            );
            assert_eq!(app_api.close_calls, 1);
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

    #[cfg(windows)]
    static WINDOWS_SMOKE_SERIAL: Mutex<()> = Mutex::new(());

    struct FakeReader {
        sequences: VecDeque<u32>,
        values: VecDeque<Result<Option<String>, ()>>,
        source: ClipboardSourceMetadata,
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
            _source: ClipboardSourceMetadata,
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

        fn source_metadata(&mut self) -> ClipboardSourceMetadata {
            std::mem::take(&mut self.source)
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
            sequences: VecDeque::from([10, 10, 10]),
            values: VecDeque::from([Ok(Some("captured".to_owned()))]),
            source: ClipboardSourceMetadata::default(),
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
    fn resolved_source_is_persisted_without_any_full_executable_path() {
        let (_temp, service) = service();
        let sink: ClipboardHistoryEventSink = Arc::new(|| {});
        let mut reader = FakeReader {
            sequences: VecDeque::from([77, 77]),
            values: VecDeque::from([Ok(Some("with-source".to_owned()))]),
            source: ClipboardSourceMetadata {
                source_application: Some("Code".to_owned()),
                source_process: Some("Code.exe".to_owned()),
            },
            reads: 0,
        };
        let mut last_sequence = None;

        process_clipboard_notification(&mut reader, &mut last_sequence, &service, &sink, 123);

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
        assert_eq!(item.source_application.as_deref(), Some("Code"));
        assert_eq!(item.source_process.as_deref(), Some("Code.exe"));
        assert!(!item
            .source_process
            .as_deref()
            .unwrap()
            .contains(['\\', '/']));
    }

    #[test]
    fn sequence_change_during_text_and_source_read_discards_mismatched_snapshot() {
        let target = FakeRecordTarget {
            outcomes: Mutex::new(VecDeque::new()),
            calls: AtomicUsize::new(0),
        };
        let sink: ClipboardHistoryEventSink = Arc::new(|| panic!("must not emit"));
        let mut reader = FakeReader {
            sequences: VecDeque::from([100, 101]),
            values: VecDeque::from([Ok(Some("stale".to_owned()))]),
            source: ClipboardSourceMetadata {
                source_application: Some("OldOwner".to_owned()),
                source_process: Some("old.exe".to_owned()),
            },
            reads: 0,
        };
        let mut last_sequence = None;

        process_clipboard_notification(&mut reader, &mut last_sequence, &target, &sink, 1);

        assert_eq!(target.calls.load(Ordering::SeqCst), 0);
        assert_eq!(last_sequence, None);
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
            sequences: VecDeque::from([5, 5, 5, 0, 0]),
            values: VecDeque::from([Err(()), Ok(None), Ok(Some(String::new()))]),
            source: ClipboardSourceMetadata::default(),
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
            sequences: VecDeque::from([44, 44, 44, 44]),
            values: VecDeque::from([Ok(Some("retry".to_owned())), Ok(Some("retry".to_owned()))]),
            source: ClipboardSourceMetadata::default(),
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
            sequences: VecDeque::from([9, 9]),
            values: VecDeque::from([Ok(Some("too-large-or-invalid".to_owned()))]),
            source: ClipboardSourceMetadata::default(),
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
        let _serial = WINDOWS_SMOKE_SERIAL.lock().unwrap();
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

    #[cfg(windows)]
    #[test]
    #[ignore = "queries a real controlled HWND and process; run explicitly and serially"]
    fn windows_known_owner_source_resolver_smoke_test() {
        let _serial = WINDOWS_SMOKE_SERIAL.lock().unwrap();
        let temp = tempdir().unwrap();
        let storage = Arc::new(StorageService::initialize(temp.path()).unwrap());
        let service = Arc::new(ClipboardService::initialize(storage));
        let manager = ClipboardListenerManager::default();

        manager.start(service, Arc::new(|| {})).unwrap();
        let window = manager
            .control
            .lock()
            .unwrap()
            .as_ref()
            .expect("running listener must own a message-only window")
            .window;
        let source = platform::resolve_system_source_for_owner(window);
        manager.stop().unwrap();

        let current_exe = std::env::current_exe().unwrap();
        let expected_process = current_exe.file_name().unwrap().to_string_lossy();
        let expected_application = current_exe.file_stem().unwrap().to_string_lossy();
        assert_eq!(
            source.source_process.as_deref(),
            Some(expected_process.as_ref())
        );
        assert_eq!(
            source.source_application.as_deref(),
            Some(expected_application.as_ref())
        );
        assert_eq!(manager.status(), ClipboardListenerStatus::Stopped);
    }
}
