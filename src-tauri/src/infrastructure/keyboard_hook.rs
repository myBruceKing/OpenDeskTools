use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use thiserror::Error;

const EVENT_CAPACITY: usize = 64;
const VK_TAB: u32 = 0x09;
const VK_ESCAPE: u32 = 0x1b;
const VK_V: u32 = 0x56;
const VK_LWIN: u32 = 0x5b;
const VK_RWIN: u32 = 0x5c;
const VK_SHIFT: u32 = 0x10;
const VK_CONTROL: u32 = 0x11;
const VK_MENU: u32 = 0x12;
const VK_LSHIFT: u32 = 0xa0;
const VK_RSHIFT: u32 = 0xa1;
const VK_LCONTROL: u32 = 0xa2;
const VK_RCONTROL: u32 = 0xa3;
const VK_LMENU: u32 = 0xa4;
const VK_RMENU: u32 = 0xa5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyTransition {
    Down,
    Up,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeHotkeyPhase {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeHotkeyEvent {
    pub generation: u64,
    pub phase: RuntimeHotkeyPhase,
    /// Foreground HWND observed in the hook callback before OpenDeskTools focuses a window.
    pub foreground_window: Option<usize>,
    pub foreground_process_id: Option<u32>,
}

type RuntimeSink = Arc<dyn Fn(RuntimeHotkeyEvent) + Send + Sync + 'static>;
type CaptureSink = Arc<dyn Fn(u64, String) + Send + Sync + 'static>;
type SurfaceEscapeSink = Arc<dyn Fn(u64) + Send + Sync + 'static>;

#[derive(Debug, Error)]
pub enum KeyboardHookError {
    #[cfg(not(windows))]
    #[error("low-level keyboard hooks are only supported on Windows")]
    UnsupportedPlatform,
    #[error("keyboard hook state lock is poisoned")]
    LockPoisoned,
    #[error("failed to start keyboard hook thread: {0}")]
    ThreadStart(#[from] std::io::Error),
    #[error("keyboard hook worker stopped before initialization")]
    WorkerDisconnected,
    #[error("SetWindowsHookExW failed with Win32 error {0}")]
    HookInstall(u32),
    #[error("failed to stop keyboard hook worker with Win32 error {0}")]
    StopSignal(u32),
    #[error("keyboard hook worker panicked")]
    WorkerPanicked,
}

#[derive(Clone)]
struct RuntimeRegistration {
    generation: u64,
    sink: RuntimeSink,
}

#[derive(Clone)]
struct CaptureRegistration {
    session_id: u64,
    target_window: usize,
    sink: CaptureSink,
}

#[derive(Clone)]
struct SurfaceEscapeRegistration {
    generation: u64,
    sink: SurfaceEscapeSink,
}

enum BrokerEvent {
    Runtime(RuntimeHotkeyEvent),
    Capture { session_id: u64, token: String },
    SurfaceEscape { generation: u64 },
}

#[derive(Debug, Default)]
struct RuntimeKeyState {
    pressed_modifiers: HashSet<u32>,
    v_latched: bool,
    latched_generation: u64,
}

impl RuntimeKeyState {
    fn clear(&mut self) {
        self.pressed_modifiers.clear();
        self.v_latched = false;
        self.latched_generation = 0;
    }

    fn handle(
        &mut self,
        registration: &RuntimeRegistration,
        virtual_key: u32,
        transition: KeyTransition,
        foreground_window: Option<usize>,
        foreground_process_id: Option<u32>,
        events: &SyncSender<BrokerEvent>,
    ) -> bool {
        if is_modifier(virtual_key) {
            match transition {
                KeyTransition::Down => {
                    self.pressed_modifiers.insert(virtual_key);
                }
                KeyTransition::Up => {
                    self.pressed_modifiers.remove(&virtual_key);
                }
            }
            return false;
        }
        if virtual_key != VK_V {
            return false;
        }
        match transition {
            KeyTransition::Down => {
                if self.v_latched {
                    return self.latched_generation == registration.generation;
                }
                if !self.exact_win_modifier() {
                    return false;
                }
                let event = RuntimeHotkeyEvent {
                    generation: registration.generation,
                    phase: RuntimeHotkeyPhase::Pressed,
                    foreground_window,
                    foreground_process_id,
                };
                // Fail open: if the bounded event path is unavailable, Windows keeps
                // ownership and receives the original keystroke.
                if events.try_send(BrokerEvent::Runtime(event)).is_err() {
                    return false;
                }
                self.v_latched = true;
                self.latched_generation = registration.generation;
                true
            }
            KeyTransition::Up => {
                if !self.v_latched || self.latched_generation != registration.generation {
                    return false;
                }
                self.v_latched = false;
                self.latched_generation = 0;
                events
                    .try_send(BrokerEvent::Runtime(RuntimeHotkeyEvent {
                        generation: registration.generation,
                        phase: RuntimeHotkeyPhase::Released,
                        foreground_window,
                        foreground_process_id,
                    }))
                    .is_ok()
            }
        }
    }

    fn exact_win_modifier(&self) -> bool {
        self.pressed_modifiers.iter().any(|key| is_win(*key))
            && !self
                .pressed_modifiers
                .iter()
                .any(|key| is_shift(*key) || is_control(*key) || is_alt(*key))
    }
}

#[derive(Debug, Default)]
struct CaptureKeyState {
    pressed: HashSet<u32>,
    captured_keys: HashSet<u32>,
}

impl CaptureKeyState {
    fn clear(&mut self) {
        self.pressed.clear();
        self.captured_keys.clear();
    }

    fn handle(
        &mut self,
        virtual_key: u32,
        extended: bool,
        transition: KeyTransition,
        target_is_foreground: bool,
    ) -> (Option<String>, bool) {
        if !target_is_foreground {
            self.clear();
            return (None, false);
        }
        if is_modifier(virtual_key) {
            match transition {
                KeyTransition::Down => {
                    self.pressed.insert(virtual_key);
                }
                KeyTransition::Up => {
                    self.pressed.remove(&virtual_key);
                }
            }
            return (None, is_win(virtual_key));
        }
        if matches!(virtual_key, VK_TAB | VK_ESCAPE) {
            return (None, false);
        }
        match transition {
            KeyTransition::Down => {
                if !self.pressed.insert(virtual_key) {
                    return (None, self.captured_keys.contains(&virtual_key));
                }
                if !self.pressed.iter().any(|key| is_win(*key)) {
                    return (None, false);
                }
                self.captured_keys.insert(virtual_key);
                (normalized_token(&self.pressed, virtual_key, extended), true)
            }
            KeyTransition::Up => {
                self.pressed.remove(&virtual_key);
                (None, self.captured_keys.remove(&virtual_key))
            }
        }
    }
}

#[derive(Debug, Default)]
struct SurfaceEscapeKeyState {
    pressed_modifiers: HashSet<u32>,
    latched_generation: Option<u64>,
}

impl SurfaceEscapeKeyState {
    fn clear(&mut self) {
        self.pressed_modifiers.clear();
        self.latched_generation = None;
    }

    fn handle(
        &mut self,
        registration: &SurfaceEscapeRegistration,
        virtual_key: u32,
        transition: KeyTransition,
        events: &SyncSender<BrokerEvent>,
    ) -> bool {
        if is_modifier(virtual_key) {
            match transition {
                KeyTransition::Down => {
                    self.pressed_modifiers.insert(virtual_key);
                }
                KeyTransition::Up => {
                    self.pressed_modifiers.remove(&virtual_key);
                }
            }
            return false;
        }
        if virtual_key != VK_ESCAPE {
            return false;
        }
        match transition {
            KeyTransition::Down => {
                if self.latched_generation == Some(registration.generation) {
                    return true;
                }
                if !self.pressed_modifiers.is_empty()
                    || events
                        .try_send(BrokerEvent::SurfaceEscape {
                            generation: registration.generation,
                        })
                        .is_err()
                {
                    return false;
                }
                self.latched_generation = Some(registration.generation);
                true
            }
            KeyTransition::Up => {
                if self.latched_generation != Some(registration.generation) {
                    return false;
                }
                self.latched_generation = None;
                true
            }
        }
    }
}

#[derive(Default)]
struct BrokerState {
    runtime: Option<RuntimeRegistration>,
    capture: Option<CaptureRegistration>,
    surface_escape: Option<SurfaceEscapeRegistration>,
    runtime_keys: RuntimeKeyState,
    capture_keys: CaptureKeyState,
    surface_escape_keys: SurfaceEscapeKeyState,
}

impl std::fmt::Debug for BrokerState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrokerState")
            .field(
                "runtime_generation",
                &self.runtime.as_ref().map(|r| r.generation),
            )
            .field(
                "capture_session",
                &self.capture.as_ref().map(|r| r.session_id),
            )
            .field("runtime_keys", &self.runtime_keys)
            .field("capture_keys", &self.capture_keys)
            .field(
                "surface_escape_generation",
                &self.surface_escape.as_ref().map(|r| r.generation),
            )
            .field("surface_escape_keys", &self.surface_escape_keys)
            .finish()
    }
}

#[derive(Debug, Default)]
struct WorkerState {
    hook_thread_id: u32,
    hook_thread: Option<JoinHandle<()>>,
    event_thread: Option<JoinHandle<()>>,
    event_sender: Option<SyncSender<BrokerEvent>>,
}

#[derive(Debug)]
struct BrokerInner {
    state: Mutex<BrokerState>,
    lifecycle: Mutex<WorkerState>,
    next_generation: AtomicU64,
    next_session: AtomicU64,
    next_surface_escape_generation: AtomicU64,
}

impl Default for BrokerInner {
    fn default() -> Self {
        Self {
            state: Mutex::new(BrokerState::default()),
            lifecycle: Mutex::new(WorkerState::default()),
            next_generation: AtomicU64::new(0),
            next_session: AtomicU64::new(0),
            next_surface_escape_generation: AtomicU64::new(0),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct KeyboardHookBroker {
    inner: Arc<BrokerInner>,
}

impl KeyboardHookBroker {
    pub fn register_surface_escape<F>(&self, sink: F) -> Result<u64, KeyboardHookError>
    where
        F: Fn(u64) + Send + Sync + 'static,
    {
        self.ensure_started()?;
        let generation = self
            .inner
            .next_surface_escape_generation
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| KeyboardHookError::LockPoisoned)?;
        state.surface_escape_keys.clear();
        state.surface_escape = Some(SurfaceEscapeRegistration {
            generation,
            sink: Arc::new(sink),
        });
        Ok(generation)
    }

    pub fn unregister_surface_escape(&self, generation: u64) -> Result<bool, KeyboardHookError> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| KeyboardHookError::LockPoisoned)?;
        if state.surface_escape.as_ref().map(|entry| entry.generation) != Some(generation) {
            return Ok(false);
        }
        state.surface_escape = None;
        state.surface_escape_keys.clear();
        Ok(true)
    }

    pub fn stop_surface_escape(&self) -> Result<(), KeyboardHookError> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| KeyboardHookError::LockPoisoned)?;
        state.surface_escape = None;
        state.surface_escape_keys.clear();
        Ok(())
    }

    pub fn register_win_v<F>(&self, sink: F) -> Result<u64, KeyboardHookError>
    where
        F: Fn(RuntimeHotkeyEvent) + Send + Sync + 'static,
    {
        self.ensure_started()?;
        let generation = self.inner.next_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| KeyboardHookError::LockPoisoned)?;
        state.runtime_keys.clear();
        state.runtime = Some(RuntimeRegistration {
            generation,
            sink: Arc::new(sink),
        });
        Ok(generation)
    }

    pub fn unregister_win_v(&self, generation: u64) -> Result<bool, KeyboardHookError> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| KeyboardHookError::LockPoisoned)?;
        if state.runtime.as_ref().map(|entry| entry.generation) != Some(generation) {
            return Ok(false);
        }
        state.runtime = None;
        state.runtime_keys.clear();
        Ok(true)
    }

    pub fn start_capture<F>(&self, target_window: usize, sink: F) -> Result<u64, KeyboardHookError>
    where
        F: Fn(u64, String) + Send + Sync + 'static,
    {
        self.ensure_started()?;
        let session_id = self.inner.next_session.fetch_add(1, Ordering::Relaxed) + 1;
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| KeyboardHookError::LockPoisoned)?;
        state.capture_keys.clear();
        state.runtime_keys.clear();
        state.surface_escape_keys.clear();
        state.capture = Some(CaptureRegistration {
            session_id,
            target_window,
            sink: Arc::new(sink),
        });
        Ok(session_id)
    }

    pub fn stop_capture(&self, session_id: u64) -> Result<bool, KeyboardHookError> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| KeyboardHookError::LockPoisoned)?;
        if state.capture.as_ref().map(|entry| entry.session_id) != Some(session_id) {
            return Ok(false);
        }
        state.capture = None;
        state.capture_keys.clear();
        state.runtime_keys.clear();
        state.surface_escape_keys.clear();
        Ok(true)
    }

    pub fn stop_active_capture(&self) -> Result<(), KeyboardHookError> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| KeyboardHookError::LockPoisoned)?;
        state.capture = None;
        state.capture_keys.clear();
        state.runtime_keys.clear();
        state.surface_escape_keys.clear();
        Ok(())
    }

    pub fn shutdown(&self) -> Result<(), KeyboardHookError> {
        {
            let mut state = self
                .inner
                .state
                .lock()
                .map_err(|_| KeyboardHookError::LockPoisoned)?;
            state.runtime = None;
            state.capture = None;
            state.surface_escape = None;
            state.runtime_keys.clear();
            state.capture_keys.clear();
            state.surface_escape_keys.clear();
        }
        let mut worker = self
            .inner
            .lifecycle
            .lock()
            .map_err(|_| KeyboardHookError::LockPoisoned)?;
        if let Some(hook_thread) = worker.hook_thread.take() {
            #[cfg(windows)]
            if !hook_thread.is_finished() {
                use windows_sys::Win32::Foundation::GetLastError;
                use windows_sys::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
                if unsafe { PostThreadMessageW(worker.hook_thread_id, WM_QUIT, 0, 0) } == 0 {
                    worker.hook_thread = Some(hook_thread);
                    return Err(KeyboardHookError::StopSignal(unsafe { GetLastError() }));
                }
            }
            if hook_thread.join().is_err() {
                return Err(KeyboardHookError::WorkerPanicked);
            }
        }
        worker.event_sender = None;
        if worker
            .event_thread
            .take()
            .is_some_and(|thread| thread.join().is_err())
        {
            return Err(KeyboardHookError::WorkerPanicked);
        }
        worker.hook_thread_id = 0;
        Ok(())
    }

    fn ensure_started(&self) -> Result<(), KeyboardHookError> {
        #[cfg(not(windows))]
        return Err(KeyboardHookError::UnsupportedPlatform);

        #[cfg(windows)]
        {
            let mut worker = self
                .inner
                .lifecycle
                .lock()
                .map_err(|_| KeyboardHookError::LockPoisoned)?;
            if worker.hook_thread.is_some() {
                return Ok(());
            }
            let (event_sender, event_receiver) = mpsc::sync_channel(EVENT_CAPACITY);
            let weak_for_events = Arc::downgrade(&self.inner);
            let event_thread = std::thread::Builder::new()
                .name("keyboard-hook-events".to_owned())
                .spawn(move || {
                    while let Ok(event) = event_receiver.recv() {
                        let Some(inner) = weak_for_events.upgrade() else {
                            break;
                        };
                        dispatch_event(&inner, event);
                    }
                })?;
            let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
            let weak_for_hook = Arc::downgrade(&self.inner);
            let callback_sender = event_sender.clone();
            let hook_thread = match std::thread::Builder::new()
                .name("keyboard-hook".to_owned())
                .spawn(move || run_windows_hook(weak_for_hook, callback_sender, ready_sender))
            {
                Ok(thread) => thread,
                Err(error) => {
                    drop(event_sender);
                    event_thread.join().ok();
                    return Err(KeyboardHookError::ThreadStart(error));
                }
            };
            let thread_id = match ready_receiver.recv() {
                Ok(Ok(thread_id)) => thread_id,
                Ok(Err(error)) => {
                    hook_thread.join().ok();
                    drop(event_sender);
                    event_thread.join().ok();
                    return Err(error);
                }
                Err(_) => {
                    hook_thread.join().ok();
                    drop(event_sender);
                    event_thread.join().ok();
                    return Err(KeyboardHookError::WorkerDisconnected);
                }
            };
            worker.hook_thread_id = thread_id;
            worker.hook_thread = Some(hook_thread);
            worker.event_thread = Some(event_thread);
            worker.event_sender = Some(event_sender);
            Ok(())
        }
    }
}

fn dispatch_event(inner: &BrokerInner, event: BrokerEvent) {
    match event {
        BrokerEvent::Runtime(event) => {
            let sink = inner.state.lock().ok().and_then(|state| {
                state
                    .runtime
                    .as_ref()
                    .filter(|registration| registration.generation == event.generation)
                    .map(|registration| Arc::clone(&registration.sink))
            });
            if let Some(sink) = sink {
                sink(event);
            }
        }
        BrokerEvent::Capture { session_id, token } => {
            let sink = inner.state.lock().ok().and_then(|state| {
                state
                    .capture
                    .as_ref()
                    .filter(|registration| registration.session_id == session_id)
                    .map(|registration| Arc::clone(&registration.sink))
            });
            if let Some(sink) = sink {
                sink(session_id, token);
            }
        }
        BrokerEvent::SurfaceEscape { generation } => {
            let sink = inner.state.lock().ok().and_then(|state| {
                state
                    .surface_escape
                    .as_ref()
                    .filter(|registration| registration.generation == generation)
                    .map(|registration| Arc::clone(&registration.sink))
            });
            if let Some(sink) = sink {
                sink(generation);
            }
        }
    }
}

fn normalized_token(pressed: &HashSet<u32>, virtual_key: u32, extended: bool) -> Option<String> {
    let key = key_name(virtual_key, extended)?;
    let mut parts = Vec::with_capacity(5);
    if pressed.iter().any(|key| is_control(*key)) {
        parts.push("Ctrl");
    }
    if pressed.iter().any(|key| is_alt(*key)) {
        parts.push("Alt");
    }
    if pressed.iter().any(|key| is_shift(*key)) {
        parts.push("Shift");
    }
    if pressed.iter().any(|key| is_win(*key)) {
        parts.push("Win");
    }
    parts.push(key.as_str());
    Some(parts.join("+"))
}

fn key_name(virtual_key: u32, extended: bool) -> Option<String> {
    if (0x30..=0x39).contains(&virtual_key) || (0x41..=0x5a).contains(&virtual_key) {
        return char::from_u32(virtual_key).map(|key| key.to_string());
    }
    if (0x70..=0x87).contains(&virtual_key) {
        return Some(format!("F{}", virtual_key - 0x70 + 1));
    }
    if !extended {
        let numpad_navigation = match virtual_key {
            0x2d => Some("Numpad0"),
            0x23 => Some("Numpad1"),
            0x28 => Some("Numpad2"),
            0x22 => Some("Numpad3"),
            0x25 => Some("Numpad4"),
            0x0c => Some("Numpad5"),
            0x27 => Some("Numpad6"),
            0x24 => Some("Numpad7"),
            0x26 => Some("Numpad8"),
            0x21 => Some("Numpad9"),
            0x2e => Some("NumpadDecimal"),
            _ => None,
        };
        if let Some(key) = numpad_navigation {
            return Some(key.to_owned());
        }
    }
    if (0x60..=0x69).contains(&virtual_key) {
        return Some(format!("Numpad{}", virtual_key - 0x60));
    }
    let key = match virtual_key {
        0x08 => "Backspace",
        0x0d => "Enter",
        0x13 => "Pause",
        0x14 => "CapsLock",
        0x20 => "Space",
        0x21 => "PageUp",
        0x22 => "PageDown",
        0x23 => "End",
        0x24 => "Home",
        0x25 => "ArrowLeft",
        0x26 => "ArrowUp",
        0x27 => "ArrowRight",
        0x28 => "ArrowDown",
        0x2c => "PrintScreen",
        0x2d => "Insert",
        0x2e => "Delete",
        0x6a => "NumpadMultiply",
        0x6b => "NumpadAdd",
        0x6d => "NumpadSubtract",
        0x6e => "NumpadDecimal",
        0x6f => "NumpadDivide",
        0x90 => "NumLock",
        0x91 => "ScrollLock",
        0xad => "AudioVolumeMute",
        0xae => "AudioVolumeDown",
        0xaf => "AudioVolumeUp",
        0xb0 => "MediaTrackNext",
        0xb1 => "MediaTrackPrevious",
        0xb2 => "MediaStop",
        0xb3 => "MediaPlayPause",
        0xba => "Semicolon",
        0xbb => "Equal",
        0xbc => "Comma",
        0xbd => "Minus",
        0xbe => "Period",
        0xbf => "Slash",
        0xc0 => "Backquote",
        0xdb => "BracketLeft",
        0xdc => "Backslash",
        0xdd => "BracketRight",
        0xde => "Quote",
        0xfa => "MediaPlay",
        _ => return None,
    };
    Some(key.to_owned())
}

fn is_modifier(key: u32) -> bool {
    is_win(key) || is_shift(key) || is_control(key) || is_alt(key)
}
fn is_win(key: u32) -> bool {
    matches!(key, VK_LWIN | VK_RWIN)
}
fn is_shift(key: u32) -> bool {
    matches!(key, VK_SHIFT | VK_LSHIFT | VK_RSHIFT)
}
fn is_control(key: u32) -> bool {
    matches!(key, VK_CONTROL | VK_LCONTROL | VK_RCONTROL)
}
fn is_alt(key: u32) -> bool {
    matches!(key, VK_MENU | VK_LMENU | VK_RMENU)
}

#[cfg(windows)]
type ReadySender = SyncSender<Result<u32, KeyboardHookError>>;

#[cfg(windows)]
#[derive(Clone)]
struct HookContext {
    inner: std::sync::Weak<BrokerInner>,
    events: SyncSender<BrokerEvent>,
}

#[cfg(windows)]
static HOOK_CONTEXT: std::sync::OnceLock<Mutex<Option<HookContext>>> = std::sync::OnceLock::new();

#[cfg(windows)]
fn hook_context_slot() -> &'static Mutex<Option<HookContext>> {
    HOOK_CONTEXT.get_or_init(|| Mutex::new(None))
}

#[cfg(windows)]
fn run_windows_hook(
    inner: std::sync::Weak<BrokerInner>,
    events: SyncSender<BrokerEvent>,
    ready: ReadySender,
) {
    use std::mem;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, PeekMessageW, SetWindowsHookExW, TranslateMessage,
        UnhookWindowsHookEx, MSG, PM_NOREMOVE, WH_KEYBOARD_LL,
    };

    let mut probe: MSG = unsafe { mem::zeroed() };
    unsafe { PeekMessageW(&mut probe, null_mut(), 0, 0, PM_NOREMOVE) };
    let thread_id = unsafe { GetCurrentThreadId() };
    let module = unsafe { GetModuleHandleW(null()) };
    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), module, 0) };
    if hook.is_null() {
        let _ = ready.send(Err(KeyboardHookError::HookInstall(unsafe {
            GetLastError()
        })));
        return;
    }
    let context = HookContext { inner, events };
    let installed = hook_context_slot()
        .lock()
        .ok()
        .and_then(|mut slot| {
            if slot.is_some() {
                None
            } else {
                *slot = Some(context);
                Some(())
            }
        })
        .is_some();
    if !installed {
        unsafe { UnhookWindowsHookEx(hook) };
        let _ = ready.send(Err(KeyboardHookError::HookInstall(0)));
        return;
    }
    if ready.send(Ok(thread_id)).is_err() {
        if let Ok(mut slot) = hook_context_slot().lock() {
            *slot = None;
        }
        unsafe { UnhookWindowsHookEx(hook) };
        return;
    }
    let mut message: MSG = unsafe { mem::zeroed() };
    while unsafe { GetMessageW(&mut message, null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    if let Ok(mut slot) = hook_context_slot().lock() {
        *slot = None;
    }
    unsafe {
        UnhookWindowsHookEx(hook);
    }
}

#[cfg(windows)]
unsafe extern "system" fn keyboard_proc(code: i32, wparam: usize, lparam: isize) -> isize {
    use std::ptr;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetForegroundWindow, GetWindowThreadProcessId, KBDLLHOOKSTRUCT,
        LLKHF_EXTENDED, LLKHF_INJECTED, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };
    if code < 0 {
        return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
    }
    let transition = match wparam as u32 {
        WM_KEYDOWN | WM_SYSKEYDOWN => KeyTransition::Down,
        WM_KEYUP | WM_SYSKEYUP => KeyTransition::Up,
        _ => return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) },
    };
    let keyboard = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
    if keyboard.flags & LLKHF_INJECTED != 0 {
        return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
    }
    let Some(context) = hook_context_slot()
        .try_lock()
        .ok()
        .and_then(|slot| slot.clone())
    else {
        return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
    };
    let Some(inner) = context.inner.upgrade() else {
        return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
    };
    let foreground = unsafe { GetForegroundWindow() };
    let foreground_window = (!foreground.is_null()).then_some(foreground as usize);
    let foreground_process_id = foreground_window.and_then(|_| {
        let mut pid = 0;
        let thread = unsafe { GetWindowThreadProcessId(foreground, &mut pid) };
        (thread != 0 && pid != 0).then_some(pid)
    });
    let Ok(mut state) = inner.state.try_lock() else {
        return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
    };
    if let Some(capture) = state.capture.clone() {
        let (token, suppress) = state.capture_keys.handle(
            keyboard.vkCode,
            keyboard.flags & LLKHF_EXTENDED != 0,
            transition,
            foreground_window == Some(capture.target_window),
        );
        if let Some(token) = token {
            if context
                .events
                .try_send(BrokerEvent::Capture {
                    session_id: capture.session_id,
                    token,
                })
                .is_err()
            {
                return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
            }
        }
        if suppress {
            return 1;
        }
        return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
    }
    if let Some(surface_escape) = state.surface_escape.clone() {
        if state.surface_escape_keys.handle(
            &surface_escape,
            keyboard.vkCode,
            transition,
            &context.events,
        ) {
            return 1;
        }
    } else {
        state.surface_escape_keys.clear();
    }
    if let Some(runtime) = state.runtime.clone() {
        if state.runtime_keys.handle(
            &runtime,
            keyboard.vkCode,
            transition,
            foreground_window,
            foreground_process_id,
            &context.events,
        ) {
            return 1;
        }
    } else {
        state.runtime_keys.clear();
    }
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registration(generation: u64) -> RuntimeRegistration {
        RuntimeRegistration {
            generation,
            sink: Arc::new(|_| {}),
        }
    }

    fn surface_escape_registration(generation: u64) -> SurfaceEscapeRegistration {
        SurfaceEscapeRegistration {
            generation,
            sink: Arc::new(|_| {}),
        }
    }

    #[test]
    fn surface_escape_latches_one_bare_press_and_suppresses_its_release() {
        let (tx, rx) = mpsc::sync_channel(4);
        let mut state = SurfaceEscapeKeyState::default();
        let registration = surface_escape_registration(9);

        assert!(state.handle(&registration, VK_ESCAPE, KeyTransition::Down, &tx));
        assert!(state.handle(&registration, VK_ESCAPE, KeyTransition::Down, &tx));
        assert!(state.handle(&registration, VK_ESCAPE, KeyTransition::Up, &tx));

        let events = rx.try_iter().collect::<Vec<_>>();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            BrokerEvent::SurfaceEscape { generation: 9 }
        ));
        assert_eq!(state.latched_generation, None);
    }

    #[test]
    fn surface_escape_passes_modified_escape_and_fails_open_when_queue_is_full() {
        for modifier in [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN] {
            let (tx, rx) = mpsc::sync_channel(2);
            let mut state = SurfaceEscapeKeyState::default();
            let registration = surface_escape_registration(3);
            assert!(!state.handle(&registration, modifier, KeyTransition::Down, &tx));
            assert!(!state.handle(&registration, VK_ESCAPE, KeyTransition::Down, &tx));
            assert!(rx.try_recv().is_err());
        }

        let (tx, _rx) = mpsc::sync_channel(0);
        let mut state = SurfaceEscapeKeyState::default();
        assert!(!state.handle(
            &surface_escape_registration(4),
            VK_ESCAPE,
            KeyTransition::Down,
            &tx,
        ));
        assert_eq!(state.latched_generation, None);
    }

    #[test]
    fn runtime_win_v_latches_one_press_and_one_release() {
        let (tx, rx) = mpsc::sync_channel(8);
        let mut state = RuntimeKeyState::default();
        state.handle(
            &registration(7),
            VK_LWIN,
            KeyTransition::Down,
            None,
            None,
            &tx,
        );
        assert!(state.handle(
            &registration(7),
            VK_V,
            KeyTransition::Down,
            Some(42),
            Some(9),
            &tx
        ));
        assert!(state.handle(
            &registration(7),
            VK_V,
            KeyTransition::Down,
            Some(42),
            Some(9),
            &tx
        ));
        assert!(state.handle(
            &registration(7),
            VK_V,
            KeyTransition::Up,
            Some(42),
            Some(9),
            &tx
        ));
        let events = rx.try_iter().collect::<Vec<_>>();
        assert_eq!(events.len(), 2);
        let BrokerEvent::Runtime(pressed) = &events[0] else {
            panic!()
        };
        assert_eq!(pressed.phase, RuntimeHotkeyPhase::Pressed);
        assert_eq!(pressed.foreground_window, Some(42));
        let BrokerEvent::Runtime(released) = &events[1] else {
            panic!()
        };
        assert_eq!(released.phase, RuntimeHotkeyPhase::Released);
    }

    #[test]
    fn runtime_override_is_exact_and_other_win_combinations_pass_through() {
        for extra in [VK_SHIFT, VK_CONTROL, VK_MENU] {
            let (tx, rx) = mpsc::sync_channel(2);
            let mut state = RuntimeKeyState::default();
            state.handle(
                &registration(1),
                VK_LWIN,
                KeyTransition::Down,
                None,
                None,
                &tx,
            );
            state.handle(
                &registration(1),
                extra,
                KeyTransition::Down,
                None,
                None,
                &tx,
            );
            assert!(!state.handle(&registration(1), VK_V, KeyTransition::Down, None, None, &tx));
            assert!(rx.try_recv().is_err());
        }
    }

    #[test]
    fn full_event_channel_fails_open_without_latching() {
        let (tx, _rx) = mpsc::sync_channel(0);
        let mut state = RuntimeKeyState::default();
        state.handle(
            &registration(2),
            VK_RWIN,
            KeyTransition::Down,
            None,
            None,
            &tx,
        );
        assert!(!state.handle(&registration(2), VK_V, KeyTransition::Down, None, None, &tx));
        assert!(!state.v_latched);
    }

    #[test]
    fn capture_has_priority_and_stale_generations_are_rejected() {
        let broker = KeyboardHookBroker::default();
        {
            let mut state = broker.inner.state.lock().unwrap();
            state.runtime = Some(registration(3));
            state.capture = Some(CaptureRegistration {
                session_id: 4,
                target_window: 5,
                sink: Arc::new(|_, _| {}),
            });
            state.surface_escape = Some(surface_escape_registration(6));
        }
        assert!(!broker.unregister_win_v(2).unwrap());
        assert!(broker.inner.state.lock().unwrap().runtime.is_some());
        assert!(!broker.stop_capture(3).unwrap());
        assert!(broker.inner.state.lock().unwrap().capture.is_some());
        assert!(broker.unregister_win_v(3).unwrap());
        assert!(broker.stop_capture(4).unwrap());
        assert!(!broker.unregister_surface_escape(5).unwrap());
        assert!(broker.unregister_surface_escape(6).unwrap());
    }

    #[test]
    fn native_capture_names_cover_the_safe_windows_key_contract() {
        for virtual_key in 0x30..=0x39 {
            assert_eq!(
                key_name(virtual_key, false),
                char::from_u32(virtual_key).map(|key| key.to_string())
            );
        }
        for virtual_key in 0x41..=0x5a {
            assert_eq!(
                key_name(virtual_key, false),
                char::from_u32(virtual_key).map(|key| key.to_string())
            );
        }
        for index in 0..24 {
            let expected = format!("F{}", index + 1);
            assert_eq!(key_name(0x70 + index, false), Some(expected));
        }
        for (virtual_key, extended, expected) in [
            (0x08, false, "Backspace"),
            (0x0d, false, "Enter"),
            (0x0d, true, "Enter"),
            (0x13, false, "Pause"),
            (0x14, false, "CapsLock"),
            (0x20, false, "Space"),
            (0x21, true, "PageUp"),
            (0x22, true, "PageDown"),
            (0x23, true, "End"),
            (0x24, true, "Home"),
            (0x25, true, "ArrowLeft"),
            (0x26, true, "ArrowUp"),
            (0x27, true, "ArrowRight"),
            (0x28, true, "ArrowDown"),
            (0x2c, true, "PrintScreen"),
            (0x2d, true, "Insert"),
            (0x2e, true, "Delete"),
            (0x2d, false, "Numpad0"),
            (0x23, false, "Numpad1"),
            (0x28, false, "Numpad2"),
            (0x22, false, "Numpad3"),
            (0x25, false, "Numpad4"),
            (0x0c, false, "Numpad5"),
            (0x27, false, "Numpad6"),
            (0x24, false, "Numpad7"),
            (0x26, false, "Numpad8"),
            (0x21, false, "Numpad9"),
            (0x2e, false, "NumpadDecimal"),
            (0x6a, false, "NumpadMultiply"),
            (0x6b, false, "NumpadAdd"),
            (0x6d, false, "NumpadSubtract"),
            (0x6e, false, "NumpadDecimal"),
            (0x6f, true, "NumpadDivide"),
            (0x90, true, "NumLock"),
            (0x91, false, "ScrollLock"),
            (0xad, true, "AudioVolumeMute"),
            (0xae, true, "AudioVolumeDown"),
            (0xaf, true, "AudioVolumeUp"),
            (0xb0, true, "MediaTrackNext"),
            (0xb1, true, "MediaTrackPrevious"),
            (0xb2, true, "MediaStop"),
            (0xb3, true, "MediaPlayPause"),
            (0xba, false, "Semicolon"),
            (0xbb, false, "Equal"),
            (0xbc, false, "Comma"),
            (0xbd, false, "Minus"),
            (0xbe, false, "Period"),
            (0xbf, false, "Slash"),
            (0xc0, false, "Backquote"),
            (0xdb, false, "BracketLeft"),
            (0xdc, false, "Backslash"),
            (0xdd, false, "BracketRight"),
            (0xde, false, "Quote"),
            (0xfa, true, "MediaPlay"),
        ] {
            assert_eq!(
                key_name(virtual_key, extended).as_deref(),
                Some(expected),
                "virtual key {virtual_key:#x}"
            );
        }
    }
}
