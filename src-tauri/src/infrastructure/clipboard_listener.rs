use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use thiserror::Error;

use super::clipboard::{
    ClipboardCaptureMetadata, ClipboardError, ClipboardService, JS_MAX_SAFE_INTEGER,
};
use super::image::ImageError;

const STATUS_STOPPED: u8 = 0;
const STATUS_RUNNING: u8 = 1;
const STATUS_UNAVAILABLE: u8 = 2;
const MAX_SUPPRESSED_SEQUENCES: usize = 64;

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
    suppressed_sequences: Arc<Mutex<VecDeque<u32>>>,
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
            suppressed_sequences: Arc::new(Mutex::new(VecDeque::new())),
            #[cfg(windows)]
            control: Mutex::new(None),
            #[cfg(not(windows))]
            control: Mutex::new(()),
        }
    }
}

impl ClipboardListenerManager {
    pub fn suppress_sequence(&self, sequence: u32) {
        if sequence == 0 {
            return;
        }
        if let Ok(mut sequences) = self.suppressed_sequences.lock() {
            if sequences.contains(&sequence) {
                return;
            }
            while sequences.len() >= MAX_SUPPRESSED_SEQUENCES {
                sequences.pop_front();
            }
            sequences.push_back(sequence);
        }
    }
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
    fn consume_suppressed_sequence(&mut self, _sequence: u32) -> bool {
        false
    }
    fn read_image(&mut self) -> Result<Option<ClipboardImage>, ()> {
        Ok(None)
    }
    fn read_files(&mut self) -> Result<Option<ClipboardFiles>, ()> {
        Ok(None)
    }
    fn read_text(&mut self) -> Result<Option<String>, ()>;

    fn source_metadata(&mut self) -> ClipboardSourceMetadata {
        ClipboardSourceMetadata::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClipboardImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClipboardFiles {
    paths: Vec<Vec<u16>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ClipboardSourceMetadata {
    source_application: Option<String>,
    source_process: Option<String>,
    source_executable_path: Option<std::path::PathBuf>,
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

    fn record_listener_image(
        &self,
        _image: ClipboardImage,
        _captured_at_ms: u64,
        _source: ClipboardSourceMetadata,
    ) -> ListenerRecordOutcome {
        ListenerRecordOutcome::PermanentReject
    }

    fn record_listener_files(
        &self,
        _files: ClipboardFiles,
        _captured_at_ms: u64,
        _source: ClipboardSourceMetadata,
    ) -> ListenerRecordOutcome {
        ListenerRecordOutcome::PermanentReject
    }
}

impl ClipboardRecordTarget for ClipboardService {
    fn record_listener_text(
        &self,
        text: String,
        captured_at_ms: u64,
        source: ClipboardSourceMetadata,
    ) -> ListenerRecordOutcome {
        let source_icon = source
            .source_executable_path
            .as_deref()
            .and_then(|path| self.cache_source_icon(path));
        match self.record_text(
            text,
            ClipboardCaptureMetadata {
                captured_at_ms,
                source_application: source.source_application,
                source_process: source.source_process,
            },
        ) {
            Ok(result) => {
                if let Some(item) = result.item.as_ref() {
                    let _ = self.attach_source_icon(item.id, source_icon.as_deref());
                }
                ListenerRecordOutcome::Recorded {
                    retained: result.retained,
                }
            }
            Err(
                ClipboardError::Storage(_)
                | ClipboardError::CorruptRecord
                | ClipboardError::LifecycleLockPoisoned,
            ) => ListenerRecordOutcome::RetryableFailure,
            Err(_) => ListenerRecordOutcome::PermanentReject,
        }
    }

    fn record_listener_image(
        &self,
        image: ClipboardImage,
        captured_at_ms: u64,
        source: ClipboardSourceMetadata,
    ) -> ListenerRecordOutcome {
        let source_icon = source
            .source_executable_path
            .as_deref()
            .and_then(|path| self.cache_source_icon(path));
        match self.record_image(
            image.width,
            image.height,
            image.rgba,
            ClipboardCaptureMetadata {
                captured_at_ms,
                source_application: source.source_application,
                source_process: source.source_process,
            },
        ) {
            Ok(result) => {
                if let Some(item) = result.item.as_ref() {
                    let _ = self.attach_source_icon(item.id, source_icon.as_deref());
                }
                ListenerRecordOutcome::Recorded {
                    retained: result.retained,
                }
            }
            Err(
                ClipboardError::Storage(_)
                | ClipboardError::CorruptRecord
                | ClipboardError::LifecycleLockPoisoned,
            ) => ListenerRecordOutcome::RetryableFailure,
            Err(ClipboardError::Image(ImageError::Io(_) | ImageError::Storage(_))) => {
                ListenerRecordOutcome::RetryableFailure
            }
            Err(_) => ListenerRecordOutcome::PermanentReject,
        }
    }

    fn record_listener_files(
        &self,
        files: ClipboardFiles,
        captured_at_ms: u64,
        source: ClipboardSourceMetadata,
    ) -> ListenerRecordOutcome {
        let source_icon = source
            .source_executable_path
            .as_deref()
            .and_then(|path| self.cache_source_icon(path));
        match self.record_files(
            files.paths,
            ClipboardCaptureMetadata {
                captured_at_ms,
                source_application: source.source_application,
                source_process: source.source_process,
            },
        ) {
            Ok(result) => {
                if let Some(item) = result.item.as_ref() {
                    let _ = self.attach_source_icon(item.id, source_icon.as_deref());
                }
                ListenerRecordOutcome::Recorded {
                    retained: result.retained,
                }
            }
            Err(
                ClipboardError::Storage(_)
                | ClipboardError::CorruptRecord
                | ClipboardError::LifecycleLockPoisoned,
            ) => ListenerRecordOutcome::RetryableFailure,
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
    if sequence != 0 && reader.consume_suppressed_sequence(sequence) {
        commit_sequence(last_sequence, sequence);
        return;
    }

    // File drops are the primary semantic payload. An Explorer-copied image file
    // remains a file rather than being silently converted to a bitmap preview.
    let files = reader.read_files().unwrap_or(None);
    let image = reader.read_image().unwrap_or(None);
    let text = match (files.is_some() || image.is_some(), reader.read_text()) {
        (_, Ok(text)) => text,
        (true, Err(())) => None,
        (false, Err(())) => return,
    };
    let source = reader.source_metadata();
    let final_sequence = reader.sequence_number();
    let Some(sequence) = stable_sequence(sequence, final_sequence) else {
        return;
    };
    let outcome = if let Some(files) = files {
        let files_outcome = target.record_listener_files(files, captured_at_ms, source.clone());
        if files_outcome == ListenerRecordOutcome::PermanentReject {
            fallback_image_or_text(target, image, text, captured_at_ms, source)
        } else {
            files_outcome
        }
    } else if let Some(image) = image {
        let image_outcome = target.record_listener_image(image, captured_at_ms, source.clone());
        if image_outcome == ListenerRecordOutcome::PermanentReject {
            if let Some(text) = text.filter(|text| !text.is_empty()) {
                target.record_listener_text(text, captured_at_ms, source)
            } else {
                image_outcome
            }
        } else {
            image_outcome
        }
    } else if let Some(text) = text.filter(|text| !text.is_empty()) {
        target.record_listener_text(text, captured_at_ms, source)
    } else {
        commit_sequence(last_sequence, sequence);
        return;
    };
    match outcome {
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

fn fallback_image_or_text<T: ClipboardRecordTarget>(
    target: &T,
    image: Option<ClipboardImage>,
    text: Option<String>,
    captured_at_ms: u64,
    source: ClipboardSourceMetadata,
) -> ListenerRecordOutcome {
    if let Some(image) = image {
        let outcome = target.record_listener_image(image, captured_at_ms, source.clone());
        if outcome != ListenerRecordOutcome::PermanentReject {
            return outcome;
        }
    }
    if let Some(text) = text.filter(|text| !text.is_empty()) {
        target.record_listener_text(text, captured_at_ms, source)
    } else {
        ListenerRecordOutcome::PermanentReject
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
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, GetDIBits, GetObjectW, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
        BI_RGB as GDI_BI_RGB, DIB_RGB_COLORS,
    };
    use windows_sys::Win32::System::DataExchange::{
        AddClipboardFormatListener, CloseClipboard, GetClipboardData, GetClipboardOwner,
        GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
        RegisterClipboardFormatW, RemoveClipboardFormatListener,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
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
        MAX_CLIPBOARD_FILES, MAX_CLIPBOARD_FILE_PATH_UNITS, MAX_SOURCE_APPLICATION_CHARS,
        MAX_SOURCE_PROCESS_CHARS, MAX_TEXT_BYTES,
    };
    use crate::infrastructure::image::{
        MAX_IMAGE_HEIGHT, MAX_IMAGE_PIXELS, MAX_IMAGE_WIDTH, MAX_RGBA_BYTES,
    };

    const STOP_MESSAGE: u32 = WM_APP + 0x31;
    const OPEN_ATTEMPTS: usize = 5;
    const OPEN_RETRY_DELAY: Duration = Duration::from_millis(12);
    const STARTUP_TIMEOUT: Duration = Duration::from_secs(1);
    const PROCESS_PATH_CAPACITY: usize = 32_768;
    const CF_DIB_FORMAT: u32 = 8;
    const CF_DIBV5_FORMAT: u32 = 17;
    const CF_BITMAP_FORMAT: u32 = 2;
    const CF_HDROP_FORMAT: u32 = 15;
    const MAX_CLIPBOARD_BLOCK_BYTES: usize = 128 * 1024 * 1024;
    const BI_RGB: u32 = 0;
    const BI_BITFIELDS: u32 = 3;
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
        let suppressed_sequences = Arc::clone(&manager.suppressed_sequences);
        let worker_thread = thread::Builder::new()
            .name("clipboard-history-worker".to_owned())
            .spawn(move || {
                run_worker(signal_receiver, service, sink, suppressed_sequences);
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
        suppressed_sequences: Arc<Mutex<VecDeque<u32>>>,
    ) {
        let mut reader = WindowsClipboardReader {
            source: ClipboardSourceMetadata::default(),
            suppressed_sequences,
        };
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
        fn is_format_available(&mut self, _format: u32) -> bool {
            false
        }
        fn get_format_data(&mut self, _format: u32) -> Option<usize> {
            None
        }
        fn global_lock_bytes(&mut self, handle: usize) -> Option<*const u8> {
            self.global_lock(handle).map(|pointer| pointer.cast())
        }
        fn png_format(&mut self) -> u32 {
            0
        }
        fn bitmap_image(&mut self) -> Result<Option<ClipboardImage>, ()> {
            Ok(None)
        }
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

        fn is_format_available(&mut self, format: u32) -> bool {
            (unsafe { IsClipboardFormatAvailable(format) }) != 0
        }

        fn get_format_data(&mut self, format: u32) -> Option<usize> {
            let handle = unsafe { GetClipboardData(format) };
            (!handle.is_null()).then_some(handle as usize)
        }

        fn global_lock_bytes(&mut self, handle: usize) -> Option<*const u8> {
            let data = unsafe { GlobalLock(handle as HGLOBAL) } as *const u8;
            (!data.is_null()).then_some(data)
        }

        fn png_format(&mut self) -> u32 {
            const PNG_FORMAT: &[u16] = &[80, 78, 71, 0];
            unsafe { RegisterClipboardFormatW(PNG_FORMAT.as_ptr()) }
        }

        fn bitmap_image(&mut self) -> Result<Option<ClipboardImage>, ()> {
            read_system_bitmap_image()
        }
    }

    fn read_system_bitmap_image() -> Result<Option<ClipboardImage>, ()> {
        let bitmap = unsafe { GetClipboardData(CF_BITMAP_FORMAT) };
        if bitmap.is_null() {
            return Ok(None);
        }
        let mut object: BITMAP = unsafe { mem::zeroed() };
        if unsafe {
            GetObjectW(
                bitmap,
                i32::try_from(mem::size_of::<BITMAP>()).map_err(|_| ())?,
                (&mut object as *mut BITMAP).cast(),
            )
        } == 0
        {
            return Err(());
        }
        if object.bmWidth <= 0 || object.bmHeight <= 0 {
            return Err(());
        }
        let width = u32::try_from(object.bmWidth).map_err(|_| ())?;
        let height = u32::try_from(object.bmHeight).map_err(|_| ())?;
        let pixels = u64::from(width).checked_mul(u64::from(height)).ok_or(())?;
        if width > MAX_IMAGE_WIDTH
            || height > MAX_IMAGE_HEIGHT
            || pixels > MAX_IMAGE_PIXELS
            || pixels.checked_mul(4).ok_or(())? > MAX_RGBA_BYTES as u64
        {
            return Err(());
        }
        let mut info: BITMAPINFO = unsafe { mem::zeroed() };
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: object.bmWidth,
            biHeight: -object.bmHeight,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: GDI_BI_RGB,
            ..unsafe { mem::zeroed() }
        };
        let mut bgra =
            vec![0_u8; usize::try_from(pixels.checked_mul(4).ok_or(())?).map_err(|_| ())?];
        let dc = unsafe { CreateCompatibleDC(std::ptr::null_mut()) };
        if dc.is_null() {
            return Err(());
        }
        let rows = unsafe {
            GetDIBits(
                dc,
                bitmap,
                0,
                height,
                bgra.as_mut_ptr().cast(),
                &mut info,
                DIB_RGB_COLORS,
            )
        };
        unsafe { DeleteDC(dc) };
        if rows != height as i32 {
            return Err(());
        }
        for pixel in bgra.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            if pixel[3] == 0 {
                pixel[3] = 255;
            }
        }
        Ok(Some(ClipboardImage {
            width,
            height,
            rgba: bgra,
        }))
    }

    struct WindowsClipboardReader {
        source: ClipboardSourceMetadata,
        suppressed_sequences: Arc<Mutex<VecDeque<u32>>>,
    }

    impl ClipboardReader for WindowsClipboardReader {
        fn sequence_number(&mut self) -> u32 {
            unsafe { GetClipboardSequenceNumber() }
        }

        fn consume_suppressed_sequence(&mut self, sequence: u32) -> bool {
            if clipboard_owned_by_current_process() {
                return true;
            }
            let Ok(mut sequences) = self.suppressed_sequences.lock() else {
                return false;
            };
            let Some(index) = sequences
                .iter()
                .position(|candidate| *candidate == sequence)
            else {
                return false;
            };
            sequences.remove(index);
            true
        }

        fn read_files(&mut self) -> Result<Option<ClipboardFiles>, ()> {
            self.source = resolve_clipboard_source(&mut SystemProcessSourceApi);
            read_files_with_retries(&mut SystemClipboardApi, || thread::sleep(OPEN_RETRY_DELAY))
        }

        fn read_image(&mut self) -> Result<Option<ClipboardImage>, ()> {
            self.source = resolve_clipboard_source(&mut SystemProcessSourceApi);
            read_image_with_retries(&mut SystemClipboardApi, || thread::sleep(OPEN_RETRY_DELAY))
        }

        fn read_text(&mut self) -> Result<Option<String>, ()> {
            if self.source == ClipboardSourceMetadata::default() {
                self.source = resolve_clipboard_source(&mut SystemProcessSourceApi);
            }
            read_text_with_retries(&mut SystemClipboardApi, || thread::sleep(OPEN_RETRY_DELAY))
        }

        fn source_metadata(&mut self) -> ClipboardSourceMetadata {
            mem::take(&mut self.source)
        }
    }

    fn clipboard_owned_by_current_process() -> bool {
        let owner = unsafe { GetClipboardOwner() };
        if owner.is_null() {
            return false;
        }
        let mut owner_process_id = 0_u32;
        let thread_id = unsafe { GetWindowThreadProcessId(owner, &mut owner_process_id) };
        thread_id != 0
            && owner_process_id != 0
            && owner_process_id == unsafe { GetCurrentProcessId() }
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
            source_executable_path: Some(std::path::PathBuf::from(path)),
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

    struct GlobalByteLockGuard<'a, A: ClipboardApi> {
        api: &'a mut A,
        handle: usize,
        pointer: *const u8,
    }

    impl<'a, A: ClipboardApi> GlobalByteLockGuard<'a, A> {
        fn try_lock(api: &'a mut A, handle: usize) -> Result<Self, ()> {
            let pointer = api.global_lock_bytes(handle).ok_or(())?;
            Ok(Self {
                api,
                handle,
                pointer,
            })
        }

        unsafe fn bytes(&self, count: usize) -> &[u8] {
            unsafe { std::slice::from_raw_parts(self.pointer, count) }
        }
    }

    impl<A: ClipboardApi> Drop for GlobalByteLockGuard<'_, A> {
        fn drop(&mut self) {
            self.api.global_unlock(self.handle);
        }
    }

    fn read_files_with_retries<A, F>(
        api: &mut A,
        mut wait_before_retry: F,
    ) -> Result<Option<ClipboardFiles>, ()>
    where
        A: ClipboardApi,
        F: FnMut(),
    {
        if !api.is_format_available(CF_HDROP_FORMAT) {
            return Ok(None);
        }
        for attempt in 0..OPEN_ATTEMPTS {
            match read_files_attempt(api) {
                Ok(Some(bytes)) => return Ok(parse_hdrop(&bytes).ok()),
                Ok(None) => return Ok(None),
                Err(()) if attempt + 1 < OPEN_ATTEMPTS => wait_before_retry(),
                Err(()) => return Err(()),
            }
        }
        Err(())
    }

    fn read_files_attempt<A: ClipboardApi>(api: &mut A) -> Result<Option<Vec<u8>>, ()> {
        let mut clipboard = ClipboardOpenGuard::try_open(api)?;
        let handle = clipboard.get_format_data(CF_HDROP_FORMAT).ok_or(())?;
        let size = clipboard.global_size(handle);
        if !(22..=MAX_CLIPBOARD_BLOCK_BYTES).contains(&size) {
            return Ok(None);
        }
        let locked = GlobalByteLockGuard::try_lock(&mut *clipboard, handle)?;
        Ok(Some(unsafe { locked.bytes(size) }.to_vec()))
    }

    fn parse_hdrop(bytes: &[u8]) -> Result<ClipboardFiles, ()> {
        if bytes.len() < 22 || bytes.len() > MAX_CLIPBOARD_BLOCK_BYTES {
            return Err(());
        }
        let offset = usize::try_from(read_u32(bytes, 0)?).map_err(|_| ())?;
        if offset < 20 || offset >= bytes.len() || offset % 2 != 0 || read_u32(bytes, 16)? == 0 {
            return Err(());
        }
        let payload = bytes.get(offset..).ok_or(())?;
        if payload.len() < 4 || payload.len() % 2 != 0 {
            return Err(());
        }
        let units = payload
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
            .collect::<Vec<_>>();
        if units.len() < 2 || units[units.len() - 2..] != [0, 0] {
            return Err(());
        }
        let mut paths = Vec::new();
        let mut start = 0_usize;
        while start < units.len() {
            let relative_end = units[start..]
                .iter()
                .position(|unit| *unit == 0)
                .ok_or(())?;
            if relative_end == 0 {
                break;
            }
            if relative_end > MAX_CLIPBOARD_FILE_PATH_UNITS || paths.len() >= MAX_CLIPBOARD_FILES {
                return Err(());
            }
            let end = start.checked_add(relative_end).ok_or(())?;
            paths.push(units[start..end].to_vec());
            start = end.checked_add(1).ok_or(())?;
        }
        if paths.is_empty() || start + 1 != units.len() {
            return Err(());
        }
        Ok(ClipboardFiles { paths })
    }

    fn read_image_with_retries<A, F>(
        api: &mut A,
        mut wait_before_retry: F,
    ) -> Result<Option<ClipboardImage>, ()>
    where
        A: ClipboardApi,
        F: FnMut(),
    {
        let png_format = api.png_format();
        if png_format != 0 && api.is_format_available(png_format) {
            if let Ok(Some(image)) =
                read_encoded_image_format_with_retries(api, png_format, &mut wait_before_retry)
            {
                return Ok(Some(image));
            }
        }
        for format in [CF_DIBV5_FORMAT, CF_DIB_FORMAT] {
            if !api.is_format_available(format) {
                continue;
            }
            match read_image_format_with_retries(api, format, &mut wait_before_retry) {
                Ok(Some(image)) => return Ok(Some(image)),
                Ok(None) | Err(()) => continue,
            }
        }
        if api.is_format_available(CF_BITMAP_FORMAT) {
            for attempt in 0..OPEN_ATTEMPTS {
                let result = match ClipboardOpenGuard::try_open(api) {
                    Ok(mut clipboard) => clipboard.bitmap_image(),
                    Err(()) => Err(()),
                };
                match result {
                    Ok(Some(image)) => return Ok(Some(image)),
                    Ok(None) => break,
                    Err(()) if attempt + 1 < OPEN_ATTEMPTS => wait_before_retry(),
                    Err(()) => break,
                }
            }
        }
        Ok(None)
    }

    fn read_encoded_image_format_with_retries<A, F>(
        api: &mut A,
        format: u32,
        wait_before_retry: &mut F,
    ) -> Result<Option<ClipboardImage>, ()>
    where
        A: ClipboardApi,
        F: FnMut(),
    {
        for attempt in 0..OPEN_ATTEMPTS {
            match read_image_attempt(api, format) {
                Ok(Some(bytes)) => return Ok(parse_png(&bytes).ok()),
                Ok(None) => return Ok(None),
                Err(()) if attempt + 1 < OPEN_ATTEMPTS => wait_before_retry(),
                Err(()) => return Err(()),
            }
        }
        Err(())
    }

    fn parse_png(bytes: &[u8]) -> Result<ClipboardImage, ()> {
        if bytes.len() < 8 || bytes.len() > MAX_CLIPBOARD_BLOCK_BYTES {
            return Err(());
        }
        let mut decoder = png::Decoder::new(bytes);
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().map_err(|_| ())?;
        let width = reader.info().width;
        let height = reader.info().height;
        let pixels = u64::from(width).checked_mul(u64::from(height)).ok_or(())?;
        if width > MAX_IMAGE_WIDTH || height > MAX_IMAGE_HEIGHT || pixels > MAX_IMAGE_PIXELS {
            return Err(());
        }
        let output_size = reader.output_buffer_size();
        if output_size > MAX_RGBA_BYTES {
            return Err(());
        }
        let mut decoded = vec![0_u8; output_size];
        let output = reader.next_frame(&mut decoded).map_err(|_| ())?;
        decoded.truncate(output.buffer_size());
        let rgba = match output.color_type {
            png::ColorType::Rgba => decoded,
            png::ColorType::Rgb => decoded
                .chunks_exact(3)
                .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
                .collect(),
            png::ColorType::Grayscale => decoded
                .into_iter()
                .flat_map(|value| [value, value, value, 255])
                .collect(),
            png::ColorType::GrayscaleAlpha => decoded
                .chunks_exact(2)
                .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
                .collect(),
            png::ColorType::Indexed => return Err(()),
        };
        let expected = usize::try_from(pixels.checked_mul(4).ok_or(())?).map_err(|_| ())?;
        if rgba.len() != expected {
            return Err(());
        }
        Ok(ClipboardImage {
            width,
            height,
            rgba,
        })
    }

    fn read_image_format_with_retries<A, F>(
        api: &mut A,
        format: u32,
        wait_before_retry: &mut F,
    ) -> Result<Option<ClipboardImage>, ()>
    where
        A: ClipboardApi,
        F: FnMut(),
    {
        for attempt in 0..OPEN_ATTEMPTS {
            match read_image_attempt(api, format) {
                Ok(Some(bytes)) => return Ok(parse_dib(bytes).ok()),
                Ok(None) => return Ok(None),
                Err(()) if attempt + 1 < OPEN_ATTEMPTS => wait_before_retry(),
                Err(()) => return Err(()),
            }
        }
        Err(())
    }

    fn read_image_attempt<A: ClipboardApi>(
        api: &mut A,
        format: u32,
    ) -> Result<Option<Vec<u8>>, ()> {
        let mut clipboard = ClipboardOpenGuard::try_open(api)?;
        let handle = clipboard.get_format_data(format).ok_or(())?;
        let size = clipboard.global_size(handle);
        if !(8..=MAX_CLIPBOARD_BLOCK_BYTES).contains(&size) {
            return Ok(None);
        }
        let locked = GlobalByteLockGuard::try_lock(&mut *clipboard, handle)?;
        Ok(Some(unsafe { locked.bytes(size) }.to_vec()))
    }

    fn parse_dib(bytes: Vec<u8>) -> Result<ClipboardImage, ()> {
        if bytes.len() < 40 || bytes.len() > MAX_CLIPBOARD_BLOCK_BYTES {
            return Err(());
        }
        let header_size = usize::try_from(read_u32(&bytes, 0)?).map_err(|_| ())?;
        if !matches!(header_size, 40 | 52 | 56 | 108 | 124) || header_size > bytes.len() {
            return Err(());
        }
        let width_i = read_i32(&bytes, 4)?;
        let height_i = read_i32(&bytes, 8)?;
        if width_i <= 0 || height_i == 0 {
            return Err(());
        }
        let width = u32::try_from(width_i).map_err(|_| ())?;
        let height = height_i.unsigned_abs();
        if width > MAX_IMAGE_WIDTH || height > MAX_IMAGE_HEIGHT {
            return Err(());
        }
        let pixels = u64::from(width).checked_mul(u64::from(height)).ok_or(())?;
        if pixels > MAX_IMAGE_PIXELS {
            return Err(());
        }
        if read_u16(&bytes, 12)? != 1 {
            return Err(());
        }
        let bit_count = read_u16(&bytes, 14)?;
        if bit_count != 24 && bit_count != 32 {
            return Err(());
        }
        let compression = read_u32(&bytes, 16)?;
        if compression != BI_RGB && compression != BI_BITFIELDS {
            return Err(());
        }
        if read_u32(&bytes, 32)? != 0 {
            return Err(());
        }
        if header_size >= 124 {
            const PROFILE_LINKED: u32 = 0x4c49_4e4b;
            const PROFILE_EMBEDDED: u32 = 0x4d42_4544;
            let color_space_type = read_u32(&bytes, 56)?;
            let profile_offset = read_u32(&bytes, 112)?;
            let profile_size = read_u32(&bytes, 116)?;
            if matches!(color_space_type, PROFILE_LINKED | PROFILE_EMBEDDED)
                || profile_offset != 0
                || profile_size != 0
            {
                return Err(());
            }
        }

        let mut pixel_offset = header_size;
        let masks = if compression == BI_BITFIELDS {
            let masks_offset = 40;
            let required = if header_size >= 52 {
                52
            } else {
                header_size.checked_add(12).ok_or(())?
            };
            if bytes.len() < required {
                return Err(());
            }
            if header_size < 52 {
                pixel_offset = required;
            }
            let red = read_u32(&bytes, masks_offset)?;
            let green = read_u32(&bytes, masks_offset + 4)?;
            let blue = read_u32(&bytes, masks_offset + 8)?;
            let alpha = if header_size >= 56 {
                read_u32(&bytes, masks_offset + 12)?
            } else {
                0
            };
            validate_masks(red, green, blue, alpha, bit_count)?;
            Some((red, green, blue, alpha))
        } else {
            None
        };
        let row_bits = usize::try_from(width)
            .map_err(|_| ())?
            .checked_mul(bit_count as usize)
            .ok_or(())?;
        let stride = row_bits
            .checked_add(31)
            .ok_or(())?
            .checked_div(32)
            .ok_or(())?
            .checked_mul(4)
            .ok_or(())?;
        let data_size = stride.checked_mul(height as usize).ok_or(())?;
        let end = pixel_offset.checked_add(data_size).ok_or(())?;
        if end > bytes.len() {
            return Err(());
        }
        let rgba_len = usize::try_from(pixels.checked_mul(4).ok_or(())?).map_err(|_| ())?;
        if rgba_len > MAX_RGBA_BYTES {
            return Err(());
        }
        let mut rgba = vec![0_u8; rgba_len];
        let source = &bytes[pixel_offset..end];
        let bytes_per_pixel = usize::from(bit_count / 8);
        for output_y in 0..height as usize {
            let source_y = if height_i > 0 {
                height as usize - 1 - output_y
            } else {
                output_y
            };
            let row = source_y.checked_mul(stride).ok_or(())?;
            for x in 0..width as usize {
                let source_offset = row
                    .checked_add(x.checked_mul(bytes_per_pixel).ok_or(())?)
                    .ok_or(())?;
                let target_offset = output_y
                    .checked_mul(width as usize)
                    .and_then(|v| v.checked_add(x))
                    .and_then(|v| v.checked_mul(4))
                    .ok_or(())?;
                if let Some((red, green, blue, alpha)) = masks {
                    let raw = if bit_count == 32 {
                        read_u32(source, source_offset)?
                    } else {
                        u32::from(source[source_offset])
                            | (u32::from(source[source_offset + 1]) << 8)
                            | (u32::from(source[source_offset + 2]) << 16)
                    };
                    rgba[target_offset] = mask_component(raw, red)?;
                    rgba[target_offset + 1] = mask_component(raw, green)?;
                    rgba[target_offset + 2] = mask_component(raw, blue)?;
                    rgba[target_offset + 3] = if alpha == 0 {
                        255
                    } else {
                        mask_component(raw, alpha)?
                    };
                } else {
                    rgba[target_offset] = source[source_offset + 2];
                    rgba[target_offset + 1] = source[source_offset + 1];
                    rgba[target_offset + 2] = source[source_offset];
                    rgba[target_offset + 3] = 255;
                }
            }
        }
        Ok(ClipboardImage {
            width,
            height,
            rgba,
        })
    }

    fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, ()> {
        let value = bytes
            .get(offset..offset.checked_add(2).ok_or(())?)
            .ok_or(())?;
        Ok(u16::from_le_bytes([value[0], value[1]]))
    }
    fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, ()> {
        let value = bytes
            .get(offset..offset.checked_add(4).ok_or(())?)
            .ok_or(())?;
        Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
    }
    fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, ()> {
        read_u32(bytes, offset).map(|value| i32::from_le_bytes(value.to_le_bytes()))
    }
    fn validate_masks(red: u32, green: u32, blue: u32, alpha: u32, bits: u16) -> Result<(), ()> {
        if red == 0
            || green == 0
            || blue == 0
            || red & green != 0
            || red & blue != 0
            || green & blue != 0
            || alpha & (red | green | blue) != 0
        {
            return Err(());
        }
        let valid_bits = if bits == 32 {
            u32::MAX
        } else {
            (1_u32 << bits) - 1
        };
        if (red | green | blue | alpha) & !valid_bits != 0 {
            return Err(());
        }
        for mask in [red, green, blue, alpha] {
            if mask != 0 {
                let shifted = mask >> mask.trailing_zeros();
                if shifted & shifted.wrapping_add(1) != 0 {
                    return Err(());
                }
            }
        }
        Ok(())
    }
    fn mask_component(raw: u32, mask: u32) -> Result<u8, ()> {
        if mask == 0 {
            return Err(());
        }
        let shift = mask.trailing_zeros();
        let maximum = mask >> shift;
        let value = (raw & mask) >> shift;
        u8::try_from((u64::from(value) * 255 + u64::from(maximum) / 2) / u64::from(maximum))
            .map_err(|_| ())
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

        fn hdrop(paths: &[&str]) -> Vec<u8> {
            let mut bytes = vec![0_u8; 20];
            bytes[0..4].copy_from_slice(&20_u32.to_le_bytes());
            bytes[16..20].copy_from_slice(&1_u32.to_le_bytes());
            for path in paths {
                for unit in path.encode_utf16().chain(std::iter::once(0)) {
                    bytes.extend_from_slice(&unit.to_le_bytes());
                }
            }
            bytes.extend_from_slice(&0_u16.to_le_bytes());
            bytes
        }

        #[test]
        fn wide_hdrop_parser_accepts_single_and_multiple_files_and_rejects_malformed_blocks() {
            let single = parse_hdrop(&hdrop(&[r"C:\notes.txt"])).unwrap();
            assert_eq!(
                single.paths,
                vec![r"C:\notes.txt".encode_utf16().collect::<Vec<_>>()]
            );
            let multiple = parse_hdrop(&hdrop(&[r"C:\one.txt", r"D:\two.png"])).unwrap();
            assert_eq!(multiple.paths.len(), 2);

            let mut ansi = hdrop(&[r"C:\one.txt"]);
            ansi[16..20].copy_from_slice(&0_u32.to_le_bytes());
            assert!(parse_hdrop(&ansi).is_err());
            let mut bad_offset = hdrop(&[r"C:\one.txt"]);
            bad_offset[0..4].copy_from_slice(&21_u32.to_le_bytes());
            assert!(parse_hdrop(&bad_offset).is_err());
            let mut unterminated = hdrop(&[r"C:\one.txt"]);
            unterminated.truncate(unterminated.len() - 2);
            assert!(parse_hdrop(&unterminated).is_err());
            assert!(parse_hdrop(&[0_u8; 20]).is_err());
        }

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

        fn dib_header(
            width: i32,
            height: i32,
            bits: u16,
            compression: u32,
            header_size: u32,
        ) -> Vec<u8> {
            let mut bytes = vec![0_u8; header_size as usize];
            bytes[0..4].copy_from_slice(&header_size.to_le_bytes());
            bytes[4..8].copy_from_slice(&width.to_le_bytes());
            bytes[8..12].copy_from_slice(&height.to_le_bytes());
            bytes[12..14].copy_from_slice(&1_u16.to_le_bytes());
            bytes[14..16].copy_from_slice(&bits.to_le_bytes());
            bytes[16..20].copy_from_slice(&compression.to_le_bytes());
            bytes
        }

        #[test]
        fn dib_24_stride_and_bottom_up_rows_decode_to_top_down_rgba() {
            let mut dib = dib_header(2, 2, 24, BI_RGB, 40);
            dib.extend_from_slice(&[255, 0, 0, 255, 255, 255, 0, 0]);
            dib.extend_from_slice(&[0, 0, 255, 0, 255, 0, 0, 0]);
            let image = parse_dib(dib).unwrap();
            assert_eq!((image.width, image.height), (2, 2));
            assert_eq!(
                image.rgba,
                vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255]
            );
        }

        #[test]
        fn dib_top_down_and_32_bitfields_masks_decode_correctly() {
            let mut top_down = dib_header(1, -2, 24, BI_RGB, 40);
            top_down.extend_from_slice(&[0, 0, 255, 0]);
            top_down.extend_from_slice(&[255, 0, 0, 0]);
            assert_eq!(
                parse_dib(top_down).unwrap().rgba,
                vec![255, 0, 0, 255, 0, 0, 255, 255]
            );

            let mut masked = dib_header(1, 1, 32, BI_BITFIELDS, 56);
            masked[40..44].copy_from_slice(&0x0000_00ff_u32.to_le_bytes());
            masked[44..48].copy_from_slice(&0x0000_ff00_u32.to_le_bytes());
            masked[48..52].copy_from_slice(&0x00ff_0000_u32.to_le_bytes());
            masked[52..56].copy_from_slice(&0xff00_0000_u32.to_le_bytes());
            masked.extend_from_slice(&0x8040_2010_u32.to_le_bytes());
            assert_eq!(parse_dib(masked).unwrap().rgba, vec![16, 32, 64, 128]);
        }

        #[test]
        fn dib_rejects_short_headers_bad_masks_and_dimension_bombs_without_allocating() {
            assert!(parse_dib(vec![0; 39]).is_err());
            for (width, height) in [(16_385, 1), (1, 16_385), (16_384, 16_384)] {
                assert!(parse_dib(dib_header(width, height, 32, BI_RGB, 40)).is_err());
            }
            let mut bad_masks = dib_header(1, 1, 32, BI_BITFIELDS, 56);
            bad_masks[40..44].copy_from_slice(&0xff_u32.to_le_bytes());
            bad_masks[44..48].copy_from_slice(&0xff_u32.to_le_bytes());
            bad_masks[48..52].copy_from_slice(&0xff0000_u32.to_le_bytes());
            bad_masks.extend_from_slice(&0_u32.to_le_bytes());
            assert!(parse_dib(bad_masks).is_err());
            let mut truncated = dib_header(100, 100, 24, BI_RGB, 40);
            truncated.extend_from_slice(&[0; 8]);
            assert!(parse_dib(truncated).is_err());

            let mut forged_color_table = dib_header(1, 1, 24, BI_RGB, 40);
            forged_color_table[32..36].copy_from_slice(&1_u32.to_le_bytes());
            forged_color_table.extend_from_slice(&[10, 20, 30, 0, 1, 2, 3, 0]);
            assert!(parse_dib(forged_color_table).is_err());

            let mut forged_profile = dib_header(1, 1, 32, BI_RGB, 124);
            forged_profile[112..116].copy_from_slice(&124_u32.to_le_bytes());
            forged_profile[116..120].copy_from_slice(&4_u32.to_le_bytes());
            forged_profile.extend_from_slice(&[9, 9, 9, 9, 3, 2, 1, 0]);
            assert!(parse_dib(forged_profile).is_err());

            let mut linked_profile = dib_header(1, 1, 32, BI_RGB, 124);
            linked_profile[56..60].copy_from_slice(&0x4c49_4e4b_u32.to_le_bytes());
            linked_profile.extend_from_slice(&[3, 2, 1, 0]);
            assert!(parse_dib(linked_profile).is_err());
        }

        struct FakeImageApi {
            dib_v5: bool,
            dib: bool,
            bytes: Vec<u8>,
            opens: usize,
            closes: usize,
            unlocks: usize,
            requested: Vec<u32>,
        }

        impl ClipboardApi for FakeImageApi {
            fn is_unicode_text_available(&mut self) -> bool {
                false
            }
            fn open(&mut self) -> bool {
                self.opens += 1;
                true
            }
            fn close(&mut self) {
                self.closes += 1;
            }
            fn get_unicode_text_data(&mut self) -> Option<usize> {
                None
            }
            fn global_size(&mut self, _handle: usize) -> usize {
                self.bytes.len()
            }
            fn global_lock(&mut self, _handle: usize) -> Option<*const u16> {
                Some(self.bytes.as_ptr().cast())
            }
            fn global_unlock(&mut self, _handle: usize) {
                self.unlocks += 1;
            }
            fn is_format_available(&mut self, format: u32) -> bool {
                (format == CF_DIBV5_FORMAT && self.dib_v5) || (format == CF_DIB_FORMAT && self.dib)
            }
            fn get_format_data(&mut self, format: u32) -> Option<usize> {
                self.requested.push(format);
                Some(9)
            }
            fn global_lock_bytes(&mut self, _handle: usize) -> Option<*const u8> {
                Some(self.bytes.as_ptr())
            }
        }

        #[test]
        fn dibv5_is_preferred_and_clipboard_open_lock_resources_are_balanced() {
            let mut bytes = dib_header(1, 1, 32, BI_RGB, 124);
            bytes.extend_from_slice(&[3, 2, 1, 0]);
            let mut api = FakeImageApi {
                dib_v5: true,
                dib: true,
                bytes,
                opens: 0,
                closes: 0,
                unlocks: 0,
                requested: Vec::new(),
            };
            let image = read_image_with_retries(&mut api, || {}).unwrap().unwrap();
            assert_eq!(image.rgba, vec![1, 2, 3, 255]);
            assert_eq!(api.requested, vec![CF_DIBV5_FORMAT]);
            assert_eq!((api.opens, api.closes, api.unlocks), (1, 1, 1));
        }

        #[test]
        fn invalid_dib_returns_none_so_same_snapshot_can_fallback_to_unicode_text() {
            let mut api = FakeImageApi {
                dib_v5: false,
                dib: true,
                bytes: vec![0; 40],
                opens: 0,
                closes: 0,
                unlocks: 0,
                requested: Vec::new(),
            };
            assert_eq!(read_image_with_retries(&mut api, || {}), Ok(None));
            assert_eq!((api.opens, api.closes, api.unlocks), (1, 1, 1));
        }

        fn one_pixel_png(rgba: [u8; 4]) -> Vec<u8> {
            let mut bytes = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header().unwrap();
                writer.write_image_data(&rgba).unwrap();
            }
            bytes
        }

        #[test]
        fn registered_png_decoder_preserves_alpha() {
            let image = parse_png(&one_pixel_png([7, 8, 9, 10])).unwrap();
            assert_eq!((image.width, image.height), (1, 1));
            assert_eq!(image.rgba, vec![7, 8, 9, 10]);
        }

        struct PngAndBitmapApi {
            png: Vec<u8>,
            bitmap: Option<ClipboardImage>,
            png_available: bool,
            dib: Option<Vec<u8>>,
            active: u32,
            requested: Vec<u32>,
        }

        impl ClipboardApi for PngAndBitmapApi {
            fn is_unicode_text_available(&mut self) -> bool {
                false
            }
            fn open(&mut self) -> bool {
                true
            }
            fn close(&mut self) {}
            fn get_unicode_text_data(&mut self) -> Option<usize> {
                None
            }
            fn global_size(&mut self, _handle: usize) -> usize {
                if self.active == 0xc001 {
                    self.png.len()
                } else {
                    self.dib.as_ref().map_or(0, Vec::len)
                }
            }
            fn global_lock(&mut self, _handle: usize) -> Option<*const u16> {
                self.global_lock_bytes(1).map(|p| p.cast())
            }
            fn global_unlock(&mut self, _handle: usize) {}
            fn png_format(&mut self) -> u32 {
                0xc001
            }
            fn is_format_available(&mut self, format: u32) -> bool {
                (format == 0xc001 && self.png_available)
                    || (format == CF_DIB_FORMAT && self.dib.is_some())
                    || (format == CF_BITMAP_FORMAT && self.bitmap.is_some())
            }
            fn get_format_data(&mut self, format: u32) -> Option<usize> {
                self.active = format;
                self.requested.push(format);
                Some(1)
            }
            fn global_lock_bytes(&mut self, _handle: usize) -> Option<*const u8> {
                if self.active == 0xc001 {
                    Some(self.png.as_ptr())
                } else {
                    self.dib.as_ref().map(|bytes| bytes.as_ptr())
                }
            }
            fn bitmap_image(&mut self) -> Result<Option<ClipboardImage>, ()> {
                Ok(self.bitmap.clone())
            }
        }

        #[test]
        fn registered_png_precedes_dib_and_bitmap_and_bad_png_falls_back() {
            let mut dib = dib_header(1, 1, 32, BI_RGB, 40);
            dib.extend_from_slice(&[30, 20, 10, 0]);
            let bitmap = ClipboardImage {
                width: 1,
                height: 1,
                rgba: vec![90, 91, 92, 255],
            };
            let mut preferred = PngAndBitmapApi {
                png: one_pixel_png([1, 2, 3, 4]),
                bitmap: Some(bitmap.clone()),
                png_available: true,
                dib: Some(dib.clone()),
                active: 0,
                requested: Vec::new(),
            };
            assert_eq!(
                read_image_with_retries(&mut preferred, || {})
                    .unwrap()
                    .unwrap()
                    .rgba,
                vec![1, 2, 3, 4]
            );
            assert_eq!(preferred.requested, vec![0xc001]);

            let mut fallback = PngAndBitmapApi {
                png: vec![0; 40],
                bitmap: Some(bitmap),
                png_available: true,
                dib: Some(dib),
                active: 0,
                requested: Vec::new(),
            };
            assert_eq!(
                read_image_with_retries(&mut fallback, || {})
                    .unwrap()
                    .unwrap()
                    .rgba,
                vec![10, 20, 30, 255]
            );
            assert_eq!(fallback.requested, vec![0xc001, CF_DIB_FORMAT]);
        }

        #[test]
        fn cf_bitmap_is_used_after_absent_higher_priority_formats() {
            let expected = ClipboardImage {
                width: 1,
                height: 1,
                rgba: vec![9, 8, 7, 255],
            };
            let mut api = PngAndBitmapApi {
                png: Vec::new(),
                bitmap: Some(expected.clone()),
                png_available: false,
                dib: None,
                active: 0,
                requested: Vec::new(),
            };
            assert_eq!(
                read_image_with_retries(&mut api, || {}).unwrap(),
                Some(expected)
            );
        }

        #[test]
        #[ignore = "mutates the real Windows clipboard; run explicitly and serially"]
        fn windows_registered_png_real_clipboard_smoke() {
            use windows_sys::Win32::System::DataExchange::{
                CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW,
                SetClipboardData,
            };
            use windows_sys::Win32::System::Memory::{
                GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
            };
            const PNG_NAME: &[u16] = &[80, 78, 71, 0];
            let png = one_pixel_png([11, 22, 33, 44]);
            unsafe {
                assert_ne!(OpenClipboard(std::ptr::null_mut()), 0);
                assert_ne!(EmptyClipboard(), 0);
                let memory = GlobalAlloc(GMEM_MOVEABLE, png.len());
                assert!(!memory.is_null());
                let pointer = GlobalLock(memory);
                assert!(!pointer.is_null());
                std::ptr::copy_nonoverlapping(png.as_ptr(), pointer.cast(), png.len());
                GlobalUnlock(memory);
                let format = RegisterClipboardFormatW(PNG_NAME.as_ptr());
                assert_ne!(format, 0);
                assert!(!SetClipboardData(format, memory).is_null());
                assert_ne!(CloseClipboard(), 0);
            }
            let image = read_image_with_retries(&mut SystemClipboardApi, || {})
                .unwrap()
                .unwrap();
            assert_eq!(image.rgba, vec![11, 22, 33, 44]);
        }

        #[test]
        #[ignore = "mutates the real Windows clipboard; run explicitly and serially"]
        fn windows_cf_bitmap_real_clipboard_smoke() {
            use windows_sys::Win32::Graphics::Gdi::{
                CreateDIBSection, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
            };
            use windows_sys::Win32::System::DataExchange::{
                CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
            };
            let mut info: BITMAPINFO = unsafe { mem::zeroed() };
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: 1,
                biHeight: -1,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..unsafe { mem::zeroed() }
            };
            let mut pixels = std::ptr::null_mut();
            let bitmap = unsafe {
                CreateDIBSection(
                    std::ptr::null_mut(),
                    &info,
                    DIB_RGB_COLORS,
                    &mut pixels,
                    std::ptr::null_mut(),
                    0,
                )
            };
            assert!(!bitmap.is_null());
            assert!(!pixels.is_null());
            unsafe {
                std::ptr::copy_nonoverlapping([33_u8, 22, 11, 255].as_ptr(), pixels.cast(), 4);
                assert_ne!(OpenClipboard(std::ptr::null_mut()), 0);
                assert_ne!(EmptyClipboard(), 0);
                assert!(!SetClipboardData(CF_BITMAP_FORMAT, bitmap).is_null());
                assert_ne!(CloseClipboard(), 0);
            }
            // Ownership of HBITMAP transfers to Windows after SetClipboardData.
            let image = read_image_with_retries(&mut SystemClipboardApi, || {})
                .unwrap()
                .unwrap();
            assert_eq!(image.rgba, vec![11, 22, 33, 255]);
        }

        struct PerFormatImageApi {
            dib_v5: Vec<u8>,
            dib: Vec<u8>,
            active_format: Option<u32>,
            requested: Vec<u32>,
            opens: usize,
            closes: usize,
            unlocks: usize,
        }

        impl ClipboardApi for PerFormatImageApi {
            fn is_unicode_text_available(&mut self) -> bool {
                false
            }
            fn open(&mut self) -> bool {
                self.opens += 1;
                true
            }
            fn close(&mut self) {
                self.closes += 1;
            }
            fn get_unicode_text_data(&mut self) -> Option<usize> {
                None
            }
            fn global_size(&mut self, _handle: usize) -> usize {
                match self.active_format {
                    Some(CF_DIBV5_FORMAT) => self.dib_v5.len(),
                    Some(CF_DIB_FORMAT) => self.dib.len(),
                    _ => 0,
                }
            }
            fn global_lock(&mut self, handle: usize) -> Option<*const u16> {
                self.global_lock_bytes(handle).map(|pointer| pointer.cast())
            }
            fn global_unlock(&mut self, _handle: usize) {
                self.unlocks += 1;
            }
            fn is_format_available(&mut self, format: u32) -> bool {
                matches!(format, CF_DIBV5_FORMAT | CF_DIB_FORMAT)
            }
            fn get_format_data(&mut self, format: u32) -> Option<usize> {
                self.active_format = Some(format);
                self.requested.push(format);
                Some(1)
            }
            fn global_lock_bytes(&mut self, _handle: usize) -> Option<*const u8> {
                match self.active_format {
                    Some(CF_DIBV5_FORMAT) => Some(self.dib_v5.as_ptr()),
                    Some(CF_DIB_FORMAT) => Some(self.dib.as_ptr()),
                    _ => None,
                }
            }
        }

        #[test]
        fn advertised_invalid_dibv5_falls_back_to_valid_dib_with_balanced_resources() {
            let invalid_v5 = vec![0_u8; 40];
            let mut valid_dib = dib_header(1, 1, 24, BI_RGB, 40);
            valid_dib.extend_from_slice(&[3, 2, 1, 0]);
            let mut api = PerFormatImageApi {
                dib_v5: invalid_v5,
                dib: valid_dib,
                active_format: None,
                requested: Vec::new(),
                opens: 0,
                closes: 0,
                unlocks: 0,
            };
            let image = read_image_with_retries(&mut api, || {}).unwrap().unwrap();
            assert_eq!(image.rgba, vec![1, 2, 3, 255]);
            assert_eq!(api.requested, vec![CF_DIBV5_FORMAT, CF_DIB_FORMAT]);
            assert_eq!((api.opens, api.closes, api.unlocks), (2, 2, 2));
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
        suppressed: VecDeque<u32>,
    }

    struct FakeRecordTarget {
        outcomes: Mutex<VecDeque<ListenerRecordOutcome>>,
        calls: AtomicUsize,
    }

    struct ImageReader {
        sequences: VecDeque<u32>,
        image: Result<Option<ClipboardImage>, ()>,
        text: Option<String>,
        text_reads: usize,
    }

    impl ClipboardReader for ImageReader {
        fn sequence_number(&mut self) -> u32 {
            self.sequences.pop_front().unwrap()
        }

        fn read_image(&mut self) -> Result<Option<ClipboardImage>, ()> {
            self.image.clone()
        }
        fn read_text(&mut self) -> Result<Option<String>, ()> {
            self.text_reads += 1;
            Ok(self.text.clone())
        }
    }

    struct ContentTarget {
        image_calls: AtomicUsize,
        text_calls: AtomicUsize,
        image_outcome: ListenerRecordOutcome,
    }

    struct FileReader {
        sequences: VecDeque<u32>,
        files: Option<ClipboardFiles>,
        image: Option<ClipboardImage>,
        text: Option<String>,
    }

    impl ClipboardReader for FileReader {
        fn sequence_number(&mut self) -> u32 {
            self.sequences.pop_front().unwrap()
        }

        fn read_files(&mut self) -> Result<Option<ClipboardFiles>, ()> {
            Ok(self.files.clone())
        }

        fn read_image(&mut self) -> Result<Option<ClipboardImage>, ()> {
            Ok(self.image.clone())
        }

        fn read_text(&mut self) -> Result<Option<String>, ()> {
            Ok(self.text.clone())
        }
    }

    struct FileTarget {
        files: AtomicUsize,
        images: AtomicUsize,
        texts: AtomicUsize,
        file_outcome: ListenerRecordOutcome,
    }

    impl ClipboardRecordTarget for FileTarget {
        fn record_listener_text(
            &self,
            _text: String,
            _captured_at_ms: u64,
            _source: ClipboardSourceMetadata,
        ) -> ListenerRecordOutcome {
            self.texts.fetch_add(1, Ordering::SeqCst);
            ListenerRecordOutcome::Recorded { retained: true }
        }

        fn record_listener_image(
            &self,
            _image: ClipboardImage,
            _captured_at_ms: u64,
            _source: ClipboardSourceMetadata,
        ) -> ListenerRecordOutcome {
            self.images.fetch_add(1, Ordering::SeqCst);
            ListenerRecordOutcome::Recorded { retained: true }
        }

        fn record_listener_files(
            &self,
            _files: ClipboardFiles,
            _captured_at_ms: u64,
            _source: ClipboardSourceMetadata,
        ) -> ListenerRecordOutcome {
            self.files.fetch_add(1, Ordering::SeqCst);
            self.file_outcome
        }
    }

    impl ClipboardRecordTarget for ContentTarget {
        fn record_listener_text(
            &self,
            _text: String,
            _captured_at_ms: u64,
            _source: ClipboardSourceMetadata,
        ) -> ListenerRecordOutcome {
            self.text_calls.fetch_add(1, Ordering::SeqCst);
            ListenerRecordOutcome::Recorded { retained: true }
        }
        fn record_listener_image(
            &self,
            _image: ClipboardImage,
            _captured_at_ms: u64,
            _source: ClipboardSourceMetadata,
        ) -> ListenerRecordOutcome {
            self.image_calls.fetch_add(1, Ordering::SeqCst);
            self.image_outcome
        }
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

        fn consume_suppressed_sequence(&mut self, sequence: u32) -> bool {
            let Some(index) = self
                .suppressed
                .iter()
                .position(|candidate| *candidate == sequence)
            else {
                return false;
            };
            self.suppressed.remove(index);
            true
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
    fn image_has_priority_and_absent_or_invalid_image_can_fallback_to_same_sequence_text() {
        let target = ContentTarget {
            image_calls: AtomicUsize::new(0),
            text_calls: AtomicUsize::new(0),
            image_outcome: ListenerRecordOutcome::Recorded { retained: true },
        };
        let sink: ClipboardHistoryEventSink = Arc::new(|| {});
        let mut image_reader = ImageReader {
            sequences: VecDeque::from([1, 1]),
            image: Ok(Some(ClipboardImage {
                width: 1,
                height: 1,
                rgba: vec![1, 2, 3, 255],
            })),
            text: Some("also-text".to_owned()),
            text_reads: 0,
        };
        process_clipboard_notification(&mut image_reader, &mut None, &target, &sink, 1);
        assert_eq!(target.image_calls.load(Ordering::SeqCst), 1);
        assert_eq!(target.text_calls.load(Ordering::SeqCst), 0);
        assert_eq!(image_reader.text_reads, 1);

        let mut fallback_reader = ImageReader {
            sequences: VecDeque::from([2, 2]),
            image: Ok(None),
            text: Some("fallback".to_owned()),
            text_reads: 0,
        };
        process_clipboard_notification(&mut fallback_reader, &mut None, &target, &sink, 2);
        assert_eq!(target.text_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_reader.text_reads, 1);

        let rejecting_target = ContentTarget {
            image_calls: AtomicUsize::new(0),
            text_calls: AtomicUsize::new(0),
            image_outcome: ListenerRecordOutcome::PermanentReject,
        };
        let mut permanent_reader = ImageReader {
            sequences: VecDeque::from([3, 3]),
            image: Ok(Some(ClipboardImage {
                width: 1,
                height: 1,
                rgba: vec![1, 2, 3, 255],
            })),
            text: Some("permanent-image-fallback".to_owned()),
            text_reads: 0,
        };
        let mut sequence = None;
        process_clipboard_notification(
            &mut permanent_reader,
            &mut sequence,
            &rejecting_target,
            &sink,
            3,
        );
        assert_eq!(rejecting_target.image_calls.load(Ordering::SeqCst), 1);
        assert_eq!(rejecting_target.text_calls.load(Ordering::SeqCst), 1);
        assert_eq!(sequence, Some(3));

        let retrying_target = ContentTarget {
            image_calls: AtomicUsize::new(0),
            text_calls: AtomicUsize::new(0),
            image_outcome: ListenerRecordOutcome::RetryableFailure,
        };
        let mut retry_reader = ImageReader {
            sequences: VecDeque::from([4, 4]),
            image: Ok(Some(ClipboardImage {
                width: 1,
                height: 1,
                rgba: vec![1, 2, 3, 255],
            })),
            text: Some("must-not-substitute".to_owned()),
            text_reads: 0,
        };
        let mut sequence = None;
        process_clipboard_notification(
            &mut retry_reader,
            &mut sequence,
            &retrying_target,
            &sink,
            4,
        );
        assert_eq!(retrying_target.text_calls.load(Ordering::SeqCst), 0);
        assert_eq!(sequence, None);
    }

    #[test]
    fn file_drop_has_priority_over_bitmap_and_text_with_permanent_reject_fallback() {
        let sink: ClipboardHistoryEventSink = Arc::new(|| {});
        let files = ClipboardFiles {
            paths: vec![r"C:\preview.png".encode_utf16().collect()],
        };
        let image = ClipboardImage {
            width: 1,
            height: 1,
            rgba: vec![1, 2, 3, 255],
        };
        let target = FileTarget {
            files: AtomicUsize::new(0),
            images: AtomicUsize::new(0),
            texts: AtomicUsize::new(0),
            file_outcome: ListenerRecordOutcome::Recorded { retained: true },
        };
        let mut reader = FileReader {
            sequences: VecDeque::from([11, 11]),
            files: Some(files.clone()),
            image: Some(image.clone()),
            text: Some("bitmap fallback".to_owned()),
        };
        let mut sequence = None;
        process_clipboard_notification(&mut reader, &mut sequence, &target, &sink, 1);
        assert_eq!(target.files.load(Ordering::SeqCst), 1);
        assert_eq!(target.images.load(Ordering::SeqCst), 0);
        assert_eq!(target.texts.load(Ordering::SeqCst), 0);
        assert_eq!(sequence, Some(11));

        let fallback = FileTarget {
            files: AtomicUsize::new(0),
            images: AtomicUsize::new(0),
            texts: AtomicUsize::new(0),
            file_outcome: ListenerRecordOutcome::PermanentReject,
        };
        let mut reader = FileReader {
            sequences: VecDeque::from([12, 12]),
            files: Some(files),
            image: Some(image),
            text: Some("text fallback".to_owned()),
        };
        process_clipboard_notification(&mut reader, &mut None, &fallback, &sink, 1);
        assert_eq!(fallback.files.load(Ordering::SeqCst), 1);
        assert_eq!(fallback.images.load(Ordering::SeqCst), 1);
        assert_eq!(fallback.texts.load(Ordering::SeqCst), 0);
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
            suppressed: VecDeque::new(),
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
                ..ClipboardSourceMetadata::default()
            },
            reads: 0,
            suppressed: VecDeque::new(),
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
                ..ClipboardSourceMetadata::default()
            },
            reads: 0,
            suppressed: VecDeque::new(),
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
            suppressed: VecDeque::new(),
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
            suppressed: VecDeque::new(),
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
            suppressed: VecDeque::new(),
        };
        let mut last_sequence = None;

        process_clipboard_notification(&mut reader, &mut last_sequence, &target, &sink, 1);

        assert_eq!(last_sequence, Some(9));
    }

    #[test]
    fn queued_internal_sequences_are_consumed_without_read_record_or_event() {
        let target = FakeRecordTarget {
            outcomes: Mutex::new(VecDeque::new()),
            calls: AtomicUsize::new(0),
        };
        let sink: ClipboardHistoryEventSink = Arc::new(|| panic!("must not emit"));
        let mut reader = FakeReader {
            sequences: VecDeque::from([70, 71]),
            values: VecDeque::new(),
            source: ClipboardSourceMetadata::default(),
            reads: 0,
            suppressed: VecDeque::from([70, 71]),
        };
        let mut last_sequence = None;

        process_clipboard_notification(&mut reader, &mut last_sequence, &target, &sink, 1);
        process_clipboard_notification(&mut reader, &mut last_sequence, &target, &sink, 2);

        assert_eq!(last_sequence, Some(71));
        assert_eq!(reader.reads, 0);
        assert_eq!(target.calls.load(Ordering::SeqCst), 0);
        assert!(reader.suppressed.is_empty());
    }

    #[test]
    fn suppression_queue_is_bounded_and_keeps_the_newest_sequences() {
        let manager = ClipboardListenerManager::default();
        for sequence in 1..=(MAX_SUPPRESSED_SEQUENCES as u32 + 5) {
            manager.suppress_sequence(sequence);
        }
        let sequences = manager.suppressed_sequences.lock().unwrap();
        assert_eq!(sequences.len(), MAX_SUPPRESSED_SEQUENCES);
        assert_eq!(sequences.front().copied(), Some(6));
        assert_eq!(
            sequences.back().copied(),
            Some(MAX_SUPPRESSED_SEQUENCES as u32 + 5)
        );
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
