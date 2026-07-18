use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};

use serde::Serialize;
use thiserror::Error;

const CAPTURE_CHANNEL_CAPACITY: usize = 64;
const SESSION_ID_PREFIX: &str = "hotkey-capture-";

const VK_BACK: u32 = 0x08;
const VK_TAB: u32 = 0x09;
const VK_RETURN: u32 = 0x0D;
const VK_SHIFT: u32 = 0x10;
const VK_CONTROL: u32 = 0x11;
const VK_MENU: u32 = 0x12;
const VK_ESCAPE: u32 = 0x1B;
const VK_SPACE: u32 = 0x20;
const VK_PRIOR: u32 = 0x21;
const VK_NEXT: u32 = 0x22;
const VK_END: u32 = 0x23;
const VK_HOME: u32 = 0x24;
const VK_LEFT: u32 = 0x25;
const VK_UP: u32 = 0x26;
const VK_RIGHT: u32 = 0x27;
const VK_DOWN: u32 = 0x28;
const VK_SNAPSHOT: u32 = 0x2C;
const VK_INSERT: u32 = 0x2D;
const VK_DELETE: u32 = 0x2E;
const VK_0: u32 = 0x30;
const VK_9: u32 = 0x39;
const VK_A: u32 = 0x41;
const VK_Z: u32 = 0x5A;
const VK_LWIN: u32 = 0x5B;
const VK_RWIN: u32 = 0x5C;
const VK_F1: u32 = 0x70;
const VK_F24: u32 = 0x87;
const VK_LSHIFT: u32 = 0xA0;
const VK_RSHIFT: u32 = 0xA1;
const VK_LCONTROL: u32 = 0xA2;
const VK_RCONTROL: u32 = 0xA3;
const VK_LMENU: u32 = 0xA4;
const VK_RMENU: u32 = 0xA5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyCaptureSession {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyCaptureStopResult {
    pub session_id: String,
    pub stopped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyCaptureEvent {
    pub session_id: String,
    pub token: String,
}

#[derive(Debug, Error)]
pub enum HotkeyCaptureError {
    #[cfg(not(windows))]
    #[error("hotkey capture is only supported on Windows")]
    UnsupportedPlatform,
    #[error("failed to start hotkey capture thread: {0}")]
    ThreadStart(#[from] std::io::Error),
    #[error("hotkey capture worker stopped before initialization")]
    WorkerDisconnected,
    #[error("another low-level keyboard hook is already active")]
    HookAlreadyActive,
    #[error("SetWindowsHookExW failed with Win32 error {0}")]
    HookInstall(u32),
    #[error("failed to stop hotkey capture worker with Win32 error {0}")]
    StopSignal(u32),
    #[error("hotkey capture worker panicked")]
    WorkerPanicked,
    #[error("hotkey capture state lock is poisoned")]
    StateLockPoisoned,
}

#[derive(Debug)]
struct ActiveSession {
    session_id: String,
    armed: Arc<AtomicBool>,
    #[cfg(windows)]
    hook_thread_id: u32,
    hook_thread: Option<JoinHandle<()>>,
    event_thread: Option<JoinHandle<()>>,
}

impl ActiveSession {
    fn disarm(&self) {
        self.armed.store(false, Ordering::SeqCst);
    }

    fn signal_stop(&self) -> Result<(), HotkeyCaptureError> {
        // Safety comes before cleanup: once disarmed, the callback is pass-through
        // even if posting WM_QUIT or unhooking subsequently fails.
        self.disarm();
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::GetLastError;
            use windows_sys::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};

            let posted = unsafe { PostThreadMessageW(self.hook_thread_id, WM_QUIT, 0, 0) };
            if posted == 0 {
                let error = unsafe { GetLastError() };
                // A worker that already exited has nothing left to unhook. Joining it
                // below still provides the deterministic cleanup barrier.
                if self
                    .hook_thread
                    .as_ref()
                    .is_some_and(|thread| !thread.is_finished())
                {
                    return Err(HotkeyCaptureError::StopSignal(error));
                }
            }
        }
        Ok(())
    }

    fn join(mut self) -> Result<(), HotkeyCaptureError> {
        if self
            .hook_thread
            .take()
            .is_some_and(|thread| thread.join().is_err())
        {
            return Err(HotkeyCaptureError::WorkerPanicked);
        }
        if self
            .event_thread
            .take()
            .is_some_and(|thread| thread.join().is_err())
        {
            return Err(HotkeyCaptureError::WorkerPanicked);
        }
        Ok(())
    }

    fn stop(self) -> Result<(), HotkeyCaptureError> {
        self.signal_stop()?;
        self.join()
    }
}

#[derive(Debug, Default)]
pub struct HotkeyCaptureManager {
    next_session_id: AtomicU64,
    lifecycle: Mutex<()>,
    active: Mutex<Option<ActiveSession>>,
}

impl HotkeyCaptureManager {
    pub fn start<F>(
        &self,
        target_window: usize,
        event_sink: F,
    ) -> Result<HotkeyCaptureSession, HotkeyCaptureError>
    where
        F: Fn(HotkeyCaptureEvent) + Send + Sync + 'static,
    {
        #[cfg(not(windows))]
        {
            let _ = event_sink;
            return Err(HotkeyCaptureError::UnsupportedPlatform);
        }

        #[cfg(windows)]
        {
            let _lifecycle = self
                .lifecycle
                .lock()
                .map_err(|_| HotkeyCaptureError::StateLockPoisoned)?;
            let previous = {
                let mut active = self
                    .active
                    .lock()
                    .map_err(|_| HotkeyCaptureError::StateLockPoisoned)?;
                if let Some(session) = active.as_ref() {
                    session.signal_stop()?;
                }
                active.take()
            };
            if let Some(previous) = previous {
                previous.join()?;
            }

            let mut active = self
                .active
                .lock()
                .map_err(|_| HotkeyCaptureError::StateLockPoisoned)?;

            let sequence = self.next_session_id.fetch_add(1, Ordering::Relaxed) + 1;
            let session_id = format!("{SESSION_ID_PREFIX}{sequence}");
            let (event_tx, event_rx) = mpsc::sync_channel(CAPTURE_CHANNEL_CAPACITY);
            let armed = Arc::new(AtomicBool::new(true));
            let event_thread = thread::Builder::new()
                .name("hotkey-capture-events".to_owned())
                .spawn(move || {
                    while let Ok(event) = event_rx.recv() {
                        event_sink(event);
                    }
                })?;

            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            let worker_session_id = session_id.clone();
            let worker_armed = Arc::clone(&armed);
            let hook_thread = match thread::Builder::new()
                .name("hotkey-capture-hook".to_owned())
                .spawn(move || {
                    run_windows_hook(
                        worker_session_id,
                        target_window,
                        worker_armed,
                        event_tx,
                        ready_tx,
                    )
                }) {
                Ok(thread) => thread,
                Err(error) => {
                    event_thread.join().ok();
                    return Err(HotkeyCaptureError::ThreadStart(error));
                }
            };

            let hook_thread_id = match ready_rx.recv() {
                Ok(Ok(thread_id)) => thread_id,
                Ok(Err(error)) => {
                    hook_thread.join().ok();
                    event_thread.join().ok();
                    return Err(error);
                }
                Err(_) => {
                    hook_thread.join().ok();
                    event_thread.join().ok();
                    return Err(HotkeyCaptureError::WorkerDisconnected);
                }
            };
            *active = Some(ActiveSession {
                session_id: session_id.clone(),
                armed,
                hook_thread_id,
                hook_thread: Some(hook_thread),
                event_thread: Some(event_thread),
            });
            Ok(HotkeyCaptureSession { session_id })
        }
    }

    pub fn stop(&self, session_id: &str) -> Result<HotkeyCaptureStopResult, HotkeyCaptureError> {
        let _lifecycle = self
            .lifecycle
            .lock()
            .map_err(|_| HotkeyCaptureError::StateLockPoisoned)?;
        let session = {
            let mut active = self
                .active
                .lock()
                .map_err(|_| HotkeyCaptureError::StateLockPoisoned)?;
            if !active_session_matches(&active, session_id) {
                return Ok(HotkeyCaptureStopResult {
                    session_id: session_id.to_owned(),
                    stopped: false,
                });
            }
            active
                .as_ref()
                .expect("matching active session")
                .signal_stop()?;
            active.take().expect("matching active session")
        };
        session.join()?;
        Ok(HotkeyCaptureStopResult {
            session_id: session_id.to_owned(),
            stopped: true,
        })
    }

    pub fn stop_active(&self) -> Result<(), HotkeyCaptureError> {
        let _lifecycle = self
            .lifecycle
            .lock()
            .map_err(|_| HotkeyCaptureError::StateLockPoisoned)?;
        let session = {
            let mut active = self
                .active
                .lock()
                .map_err(|_| HotkeyCaptureError::StateLockPoisoned)?;
            let Some(session) = active.as_ref() else {
                return Ok(());
            };
            session.signal_stop()?;
            active.take().expect("active session was checked")
        };
        session.join()
    }
}

fn active_session_matches(active: &Option<ActiveSession>, session_id: &str) -> bool {
    active
        .as_ref()
        .is_some_and(|session| session.session_id == session_id)
}

impl Drop for HotkeyCaptureManager {
    fn drop(&mut self) {
        let session = self.active.get_mut().ok().and_then(Option::take);
        if let Some(session) = session {
            let _ = session.stop();
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyTransition {
    Down,
    Up,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CaptureDecision {
    token: Option<String>,
    suppress: bool,
}

#[derive(Debug, Default)]
struct KeyCaptureState {
    pressed: HashSet<u32>,
    captured_keys: HashSet<u32>,
}

impl KeyCaptureState {
    fn clear(&mut self) {
        self.pressed.clear();
        self.captured_keys.clear();
    }

    fn handle(
        &mut self,
        virtual_key: u32,
        transition: KeyTransition,
        target_is_foreground: bool,
    ) -> CaptureDecision {
        if !target_is_foreground {
            self.clear();
            return CaptureDecision {
                token: None,
                suppress: false,
            };
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
            return CaptureDecision {
                token: None,
                // Only Win is owned by the native capture path. Other modifiers
                // continue into the WebView and are merged with native events there.
                suppress: is_win(virtual_key),
            };
        }

        if matches!(virtual_key, VK_TAB | VK_ESCAPE) {
            return CaptureDecision {
                token: None,
                suppress: false,
            };
        }

        match transition {
            KeyTransition::Down => {
                if !self.pressed.insert(virtual_key) {
                    return CaptureDecision {
                        token: None,
                        suppress: self.captured_keys.contains(&virtual_key),
                    };
                }
                if !self.pressed.iter().any(|key| is_win(*key)) {
                    return CaptureDecision {
                        token: None,
                        suppress: false,
                    };
                }
                self.captured_keys.insert(virtual_key);
                CaptureDecision {
                    token: normalized_token(&self.pressed, virtual_key),
                    suppress: true,
                }
            }
            KeyTransition::Up => {
                self.pressed.remove(&virtual_key);
                let captured = self.captured_keys.remove(&virtual_key);
                CaptureDecision {
                    token: None,
                    suppress: captured,
                }
            }
        }
    }
}

fn handle_armed_key(
    state: &mut KeyCaptureState,
    armed: &AtomicBool,
    virtual_key: u32,
    transition: KeyTransition,
    target_is_foreground: bool,
) -> CaptureDecision {
    if !armed.load(Ordering::SeqCst) {
        state.clear();
        return CaptureDecision {
            token: None,
            suppress: false,
        };
    }
    state.handle(virtual_key, transition, target_is_foreground)
}

fn normalized_token(pressed: &HashSet<u32>, virtual_key: u32) -> Option<String> {
    let key = key_name(virtual_key)?;
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

fn key_name(virtual_key: u32) -> Option<String> {
    if (VK_A..=VK_Z).contains(&virtual_key) || (VK_0..=VK_9).contains(&virtual_key) {
        return char::from_u32(virtual_key).map(|key| key.to_string());
    }
    if (VK_F1..=VK_F24).contains(&virtual_key) {
        return Some(format!("F{}", virtual_key - VK_F1 + 1));
    }
    let key = match virtual_key {
        VK_BACK => "Backspace",
        VK_RETURN => "Enter",
        VK_SPACE => "Space",
        VK_PRIOR => "PageUp",
        VK_NEXT => "PageDown",
        VK_END => "End",
        VK_HOME => "Home",
        VK_LEFT => "ArrowLeft",
        VK_UP => "ArrowUp",
        VK_RIGHT => "ArrowRight",
        VK_DOWN => "ArrowDown",
        VK_SNAPSHOT => "PrintScreen",
        VK_INSERT => "Insert",
        VK_DELETE => "Delete",
        _ => return None,
    };
    Some(key.to_owned())
}

fn is_modifier(virtual_key: u32) -> bool {
    is_shift(virtual_key) || is_control(virtual_key) || is_alt(virtual_key) || is_win(virtual_key)
}

fn is_shift(virtual_key: u32) -> bool {
    matches!(virtual_key, VK_SHIFT | VK_LSHIFT | VK_RSHIFT)
}

fn is_control(virtual_key: u32) -> bool {
    matches!(virtual_key, VK_CONTROL | VK_LCONTROL | VK_RCONTROL)
}

fn is_alt(virtual_key: u32) -> bool {
    matches!(virtual_key, VK_MENU | VK_LMENU | VK_RMENU)
}

fn is_win(virtual_key: u32) -> bool {
    matches!(virtual_key, VK_LWIN | VK_RWIN)
}

#[cfg(windows)]
type ReadySender = SyncSender<Result<u32, HotkeyCaptureError>>;

#[cfg(windows)]
#[derive(Debug)]
struct HookContext {
    session_id: String,
    target_window: usize,
    armed: Arc<AtomicBool>,
    state: Mutex<KeyCaptureState>,
    event_tx: SyncSender<HotkeyCaptureEvent>,
}

#[cfg(windows)]
static HOOK_CONTEXT: OnceLock<Mutex<Option<Arc<HookContext>>>> = OnceLock::new();

#[cfg(windows)]
fn hook_context_slot() -> &'static Mutex<Option<Arc<HookContext>>> {
    HOOK_CONTEXT.get_or_init(|| Mutex::new(None))
}

#[cfg(windows)]
fn run_windows_hook(
    session_id: String,
    target_window: usize,
    armed: Arc<AtomicBool>,
    event_tx: SyncSender<HotkeyCaptureEvent>,
    ready_tx: ReadySender,
) {
    use std::ptr;

    use windows_sys::Win32::Foundation::GetLastError;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, PeekMessageW, SetWindowsHookExW, TranslateMessage,
        UnhookWindowsHookEx, HHOOK, MSG, PM_NOREMOVE, WH_KEYBOARD_LL,
    };

    let thread_id = unsafe { GetCurrentThreadId() };
    let mut message: MSG = unsafe { std::mem::zeroed() };
    unsafe {
        PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_NOREMOVE);
    }

    let context = Arc::new(HookContext {
        session_id: session_id.clone(),
        target_window,
        armed: Arc::clone(&armed),
        state: Mutex::new(KeyCaptureState::default()),
        event_tx,
    });
    {
        let Ok(mut slot) = hook_context_slot().lock() else {
            let _ = ready_tx.send(Err(HotkeyCaptureError::StateLockPoisoned));
            return;
        };
        if slot.is_some() {
            let _ = ready_tx.send(Err(HotkeyCaptureError::HookAlreadyActive));
            return;
        }
        *slot = Some(context);
    }

    let hook = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(low_level_keyboard_proc),
            ptr::null_mut(),
            0,
        )
    };
    if hook.is_null() {
        clear_hook_context(&session_id);
        let _ = ready_tx.send(Err(HotkeyCaptureError::HookInstall(unsafe {
            GetLastError()
        })));
        return;
    }

    struct HookGuard {
        hook: HHOOK,
        session_id: String,
        armed: Arc<AtomicBool>,
    }
    impl Drop for HookGuard {
        fn drop(&mut self) {
            self.armed.store(false, Ordering::SeqCst);
            unsafe {
                UnhookWindowsHookEx(self.hook);
            }
            clear_hook_context(&self.session_id);
        }
    }
    let _guard = HookGuard {
        hook,
        session_id,
        armed,
    };
    if ready_tx.send(Ok(thread_id)).is_err() {
        return;
    }

    loop {
        let result = unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) };
        if result <= 0 {
            break;
        }
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

#[cfg(windows)]
fn clear_hook_context(session_id: &str) {
    if let Ok(mut slot) = hook_context_slot().lock() {
        if slot
            .as_ref()
            .is_some_and(|context| context.session_id == session_id)
        {
            *slot = None;
        }
    }
}

#[cfg(windows)]
unsafe extern "system" fn low_level_keyboard_proc(
    code: i32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    lparam: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use std::ptr;

    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetForegroundWindow, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED,
        WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    if code != HC_ACTION as i32 {
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

    let context = hook_context_slot()
        .lock()
        .ok()
        .and_then(|slot| slot.clone());
    if let Some(context) = context {
        if let Ok(mut state) = context.state.lock() {
            let target_is_foreground =
                unsafe { GetForegroundWindow() } as usize == context.target_window;
            let decision = handle_armed_key(
                &mut state,
                context.armed.as_ref(),
                keyboard.vkCode,
                transition,
                target_is_foreground,
            );
            if !context.armed.load(Ordering::SeqCst) {
                state.clear();
                return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
            }
            if let Some(token) = decision.token {
                let _ = context.event_tx.try_send(HotkeyCaptureEvent {
                    session_id: context.session_id.clone(),
                    token,
                });
            }
            if decision.suppress {
                return 1;
            }
        }
    }
    unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_supported_virtual_keys_to_hotkey_binding_tokens() {
        let cases = [
            (VK_A, "A"),
            (VK_Z, "Z"),
            (VK_0, "0"),
            (VK_9, "9"),
            (VK_F1, "F1"),
            (VK_F24, "F24"),
            (VK_SPACE, "Space"),
            (VK_DELETE, "Delete"),
            (VK_SNAPSHOT, "PrintScreen"),
            (VK_LEFT, "ArrowLeft"),
        ];
        for (virtual_key, expected) in cases {
            assert_eq!(key_name(virtual_key).as_deref(), Some(expected));
        }
    }

    #[test]
    fn emits_one_normalized_shell_combo_and_suppresses_repeats() {
        let mut state = KeyCaptureState::default();
        assert_eq!(
            state.handle(VK_LWIN, KeyTransition::Down, true),
            CaptureDecision {
                token: None,
                suppress: true
            }
        );
        assert!(!state.handle(VK_LSHIFT, KeyTransition::Down, true).suppress);

        let first = state.handle('S' as u32, KeyTransition::Down, true);
        assert_eq!(first.token.as_deref(), Some("Shift+Win+S"));
        assert!(first.suppress);

        let repeat = state.handle('S' as u32, KeyTransition::Down, true);
        assert_eq!(repeat.token, None);
        assert!(repeat.suppress);

        state.handle('S' as u32, KeyTransition::Up, true);
        let next_press = state.handle('S' as u32, KeyTransition::Down, true);
        assert_eq!(next_press.token.as_deref(), Some("Shift+Win+S"));
    }

    #[test]
    fn win_v_is_captured_as_one_token() {
        let mut state = KeyCaptureState::default();
        state.handle(VK_RWIN, KeyTransition::Down, true);

        let decision = state.handle('V' as u32, KeyTransition::Down, true);

        assert_eq!(decision.token.as_deref(), Some("Win+V"));
        assert!(decision.suppress);
    }

    #[test]
    fn tab_shift_tab_and_escape_remain_webview_keys() {
        let mut state = KeyCaptureState::default();

        let tab = state.handle(VK_TAB, KeyTransition::Down, true);
        assert_eq!(tab.token, None);
        assert!(!tab.suppress);

        let shift = state.handle(VK_LSHIFT, KeyTransition::Down, true);
        assert_eq!(shift.token, None);
        assert!(!shift.suppress);
        let shift_tab = state.handle(VK_TAB, KeyTransition::Down, true);
        assert_eq!(shift_tab.token, None);
        assert!(!shift_tab.suppress);

        let escape = state.handle(VK_ESCAPE, KeyTransition::Down, true);
        assert_eq!(escape.token, None);
        assert!(!escape.suppress);
    }

    #[test]
    fn modifiers_do_not_emit_standalone_tokens() {
        let mut state = KeyCaptureState::default();
        for modifier in [VK_LCONTROL, VK_RMENU, VK_LSHIFT, VK_LWIN] {
            assert_eq!(
                state.handle(modifier, KeyTransition::Down, true).token,
                None
            );
            assert_eq!(state.handle(modifier, KeyTransition::Up, true).token, None);
        }
    }

    #[test]
    fn ordinary_webview_editing_keys_are_not_owned_without_win() {
        let mut state = KeyCaptureState::default();
        for virtual_key in [VK_BACK, VK_DELETE, VK_RETURN, VK_SPACE, VK_A, VK_F1] {
            let down = state.handle(virtual_key, KeyTransition::Down, true);
            assert_eq!(down.token, None);
            assert!(!down.suppress);
            let up = state.handle(virtual_key, KeyTransition::Up, true);
            assert_eq!(up.token, None);
            assert!(!up.suppress);
        }
    }

    #[test]
    fn shift_before_win_and_win_before_shift_have_the_same_token() {
        for order in [[VK_LSHIFT, VK_LWIN], [VK_LWIN, VK_LSHIFT]] {
            let mut state = KeyCaptureState::default();
            state.handle(order[0], KeyTransition::Down, true);
            state.handle(order[1], KeyTransition::Down, true);

            let decision = state.handle('S' as u32, KeyTransition::Down, true);

            assert_eq!(decision.token.as_deref(), Some("Shift+Win+S"));
            assert!(decision.suppress);
        }
    }

    #[test]
    fn losing_foreground_resets_state_and_never_suppresses_global_input() {
        let mut state = KeyCaptureState::default();
        state.handle(VK_LWIN, KeyTransition::Down, true);

        let outside = state.handle('V' as u32, KeyTransition::Down, false);
        assert_eq!(outside.token, None);
        assert!(!outside.suppress);

        let foreground_without_win = state.handle('V' as u32, KeyTransition::Down, true);
        assert_eq!(foreground_without_win.token, None);
        assert!(!foreground_without_win.suppress);
    }

    #[test]
    fn disarming_immediately_clears_state_and_turns_capture_into_pass_through() {
        let armed = AtomicBool::new(true);
        let mut state = KeyCaptureState::default();
        let win_down = handle_armed_key(&mut state, &armed, VK_LWIN, KeyTransition::Down, true);
        assert!(win_down.suppress);

        armed.store(false, Ordering::SeqCst);
        let after_stop =
            handle_armed_key(&mut state, &armed, 'V' as u32, KeyTransition::Down, true);
        assert_eq!(after_stop.token, None);
        assert!(!after_stop.suppress);

        armed.store(true, Ordering::SeqCst);
        let no_stale_win =
            handle_armed_key(&mut state, &armed, 'V' as u32, KeyTransition::Down, true);
        assert_eq!(no_stale_win.token, None);
        assert!(!no_stale_win.suppress);
    }

    #[test]
    fn stale_session_ids_cannot_match_a_new_active_session() {
        let active = Some(ActiveSession {
            session_id: "hotkey-capture-2".to_owned(),
            armed: Arc::new(AtomicBool::new(true)),
            #[cfg(windows)]
            hook_thread_id: 1,
            hook_thread: None,
            event_thread: None,
        });

        assert!(!active_session_matches(&active, "hotkey-capture-1"));
        assert!(active_session_matches(&active, "hotkey-capture-2"));
        let armed = Arc::clone(&active.as_ref().unwrap().armed);
        active.as_ref().unwrap().disarm();
        assert!(!armed.load(Ordering::SeqCst));
    }

    #[test]
    fn stopping_an_absent_session_is_idempotent() {
        let manager = HotkeyCaptureManager::default();

        let first = manager.stop("hotkey-capture-1").unwrap();
        let second = manager.stop("hotkey-capture-1").unwrap();

        assert_eq!(
            first,
            HotkeyCaptureStopResult {
                session_id: "hotkey-capture-1".to_owned(),
                stopped: false
            }
        );
        assert_eq!(second, first);
    }

    #[cfg(windows)]
    #[test]
    fn failed_stop_signal_still_disarms_before_returning() {
        let armed = Arc::new(AtomicBool::new(true));
        let (release_tx, release_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let _ = release_rx.recv();
        });
        let mut session = ActiveSession {
            session_id: "hotkey-capture-failure-test".to_owned(),
            armed: Arc::clone(&armed),
            // Thread id zero is never a valid PostThreadMessageW target while
            // the separate worker remains alive, forcing the failure branch.
            hook_thread_id: 0,
            hook_thread: Some(worker),
            event_thread: None,
        };

        let result = session.signal_stop();

        assert!(matches!(result, Err(HotkeyCaptureError::StopSignal(_))));
        assert!(!armed.load(Ordering::SeqCst));
        release_tx.send(()).unwrap();
        session.hook_thread.take().unwrap().join().unwrap();
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "installs a process-global Windows keyboard hook; run explicitly and serially"]
    fn windows_hook_install_replace_and_stop_smoke_test() {
        let manager = HotkeyCaptureManager::default();
        // HWND 0 can never match a real foreground window, so this smoke test
        // validates hook lifecycle without suppressing user input.
        let first = manager.start(0, |_| {}).unwrap();
        let second = manager.start(0, |_| {}).unwrap();

        assert_ne!(first.session_id, second.session_id);
        assert!(!manager.stop(&first.session_id).unwrap().stopped);
        assert!(manager.stop(&second.session_id).unwrap().stopped);
    }
}
