use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use thiserror::Error;

use super::debug_qa;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceInputTargetRequirement {
    ActiveTarget,
    FocusedDescendant,
}

impl SurfaceInputTargetRequirement {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ActiveTarget => "active_target",
            Self::FocusedDescendant => "focused_descendant",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SurfaceTarget {
    top_window: usize,
    process_id: u32,
    top_thread_id: u32,
    focus_window: Option<usize>,
    focus_thread_id: Option<u32>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SurfaceError {
    #[cfg(not(windows))]
    #[error("surface targeting is unavailable on this platform")]
    UnsupportedPlatform,
    #[error("surface target state lock is poisoned")]
    LockPoisoned,
    #[error("no valid external target window is available")]
    TargetUnavailable,
    #[error("Windows denied foreground restoration")]
    FocusDenied,
    #[error("Windows denied target input-thread attachment")]
    InputAttachmentDenied,
}

#[derive(Debug, Default)]
struct SurfaceState {
    target: Option<SurfaceTarget>,
    active: bool,
    generation: u64,
}

#[derive(Debug, Default)]
pub struct SurfaceManager {
    state: Mutex<SurfaceState>,
    input_handoff_generation: AtomicU64,
    focus_loss_grace_until_ms: AtomicU64,
}

pub struct SurfaceInputHandoff<'a> {
    manager: &'a SurfaceManager,
    generation: u64,
    requirement: SurfaceInputTargetRequirement,
}

impl Drop for SurfaceInputHandoff<'_> {
    fn drop(&mut self) {
        if self
            .manager
            .input_handoff_generation
            .compare_exchange(self.generation, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            // A native Focused(false) notification can be queued behind the
            // synchronous command and arrive just after this guard drops.
            self.manager
                .focus_loss_grace_until_ms
                .store(surface_clock_ms().saturating_add(500), Ordering::Release);
        }
    }
}

impl SurfaceManager {
    /// Opens a browse/copy-only surface session when no safe external input
    /// target exists. Session activity is independent from input availability.
    pub fn activate_without_target(&self) -> Result<u64, SurfaceError> {
        let mut state = self.state.lock().map_err(|_| SurfaceError::LockPoisoned)?;
        self.focus_loss_grace_until_ms.store(0, Ordering::Release);
        state.generation = state.generation.wrapping_add(1);
        state.target = None;
        state.active = true;
        Ok(state.generation)
    }

    pub fn capture_external_target(&self, own_window: usize) -> Result<(), SurfaceError> {
        let mut state = self.state.lock().map_err(|_| SurfaceError::LockPoisoned)?;
        self.focus_loss_grace_until_ms.store(0, Ordering::Release);
        state.target = None;
        state.active = false;
        state.generation = state.generation.wrapping_add(1);
        #[cfg(windows)]
        let target = capture_target(&mut SystemSurfaceApi, own_window)?;
        #[cfg(not(windows))]
        {
            let _ = own_window;
            return Err(SurfaceError::UnsupportedPlatform);
        }
        state.target = Some(target);
        state.active = true;
        Ok(())
    }

    /// Commits the foreground identity captured by the low-level hook before any
    /// OpenDeskTools window can steal focus. The HWND/PID pair is revalidated and
    /// rejected when it belongs to this process.
    pub fn capture_external_candidate(
        &self,
        candidate_window: usize,
        candidate_process_id: u32,
        own_window: usize,
    ) -> Result<u64, SurfaceError> {
        let mut state = self.state.lock().map_err(|_| SurfaceError::LockPoisoned)?;
        self.focus_loss_grace_until_ms.store(0, Ordering::Release);
        #[cfg(windows)]
        let target = validate_candidate(
            &mut SystemSurfaceApi,
            candidate_window,
            candidate_process_id,
            own_window,
        )?;
        #[cfg(not(windows))]
        {
            let _ = (candidate_window, candidate_process_id, own_window);
            return Err(SurfaceError::UnsupportedPlatform);
        }
        state.generation = state.generation.wrapping_add(1);
        state.target = Some(target);
        state.active = true;
        Ok(state.generation)
    }

    pub fn surface_active(&self) -> bool {
        self.state.lock().is_ok_and(|state| state.active)
    }

    pub fn input_available(&self) -> bool {
        let Ok(state) = self.state.lock() else {
            return false;
        };
        let Some(target) = state.target else {
            return false;
        };
        #[cfg(windows)]
        {
            target_is_valid(&mut SystemSurfaceApi, target)
        }
        #[cfg(not(windows))]
        {
            let _ = target;
            false
        }
    }

    pub(crate) fn target_top_window(&self) -> Option<usize> {
        self.state
            .lock()
            .ok()
            .and_then(|state| state.target.map(|target| target.top_window))
    }

    pub fn begin_input_handoff_for(
        &self,
        requirement: SurfaceInputTargetRequirement,
    ) -> Result<SurfaceInputHandoff<'_>, SurfaceError> {
        let state = self.state.lock().map_err(|_| SurfaceError::LockPoisoned)?;
        let Some(target) = state.target.filter(|_| state.active) else {
            return Err(SurfaceError::TargetUnavailable);
        };
        if !captured_focus_satisfies(target, requirement) {
            debug_qa::trace(format!(
                "surface input handoff rejected requirement={} target_top={:#x} captured_focus={:?} captured_focus_thread={:?}",
                requirement.as_str(),
                target.top_window,
                target.focus_window.map(|window| format!("{window:#x}")),
                target.focus_thread_id
            ));
            return Err(SurfaceError::FocusDenied);
        }
        let generation = state.generation;
        self.input_handoff_generation
            .compare_exchange(0, generation, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| SurfaceError::FocusDenied)?;
        self.focus_loss_grace_until_ms.store(0, Ordering::Release);
        Ok(SurfaceInputHandoff {
            manager: self,
            generation,
            requirement,
        })
    }

    /// Restores and revalidates the captured target while holding the target lock,
    /// then runs the operation before capture/clear can change that identity.
    pub fn restore_and_run<T, F>(
        &self,
        handoff: &SurfaceInputHandoff<'_>,
        operation: F,
    ) -> Result<(u64, T), SurfaceError>
    where
        F: FnOnce() -> T,
    {
        let state_guard = self.state.lock().map_err(|_| SurfaceError::LockPoisoned)?;
        let generation = state_guard.generation;
        if generation != handoff.generation
            || self.input_handoff_generation.load(Ordering::Acquire) != generation
        {
            return Err(SurfaceError::TargetUnavailable);
        }
        let target = state_guard
            .target
            .as_ref()
            .copied()
            .ok_or(SurfaceError::TargetUnavailable)?;
        #[cfg(windows)]
        {
            restore_and_run_locked(
                state_guard,
                &mut SystemSurfaceApi,
                target,
                generation,
                handoff.requirement,
                operation,
            )
        }
        #[cfg(not(windows))]
        {
            let _ = (target, operation, state_guard);
            Err(SurfaceError::UnsupportedPlatform)
        }
    }

    pub fn clear(&self) -> Result<(), SurfaceError> {
        let mut state = self.state.lock().map_err(|_| SurfaceError::LockPoisoned)?;
        self.input_handoff_generation.store(0, Ordering::Release);
        self.focus_loss_grace_until_ms.store(0, Ordering::Release);
        state.target = None;
        state.active = false;
        state.generation = state.generation.wrapping_add(1);
        Ok(())
    }

    pub fn clear_if_generation(&self, generation: u64) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state.generation != generation {
            return false;
        }
        state.target = None;
        state.active = false;
        state.generation = state.generation.wrapping_add(1);
        let _ = self.input_handoff_generation.compare_exchange(
            generation,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        self.focus_loss_grace_until_ms.store(0, Ordering::Release);
        true
    }

    /// Every focus loss during the complete handoff/send/settle transaction is
    /// expected. The handoff guard ends the protection explicitly.
    pub fn should_close_on_focus_loss(&self) -> bool {
        let grace_until = self.focus_loss_grace_until_ms.load(Ordering::Acquire);
        self.input_handoff_generation.load(Ordering::Acquire) == 0
            && (grace_until == 0 || surface_clock_ms() > grace_until)
    }
}

fn surface_clock_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    u64::try_from(START.get_or_init(Instant::now).elapsed().as_millis()).unwrap_or(u64::MAX)
}

trait SurfaceApi {
    fn foreground_window(&mut self) -> Option<usize>;
    fn is_window(&mut self, window: usize) -> bool;
    fn window_identity(&mut self, window: usize) -> Option<(u32, u32)>;
    fn active_window(&mut self, thread_id: u32) -> Option<usize>;
    fn focused_window(&mut self, thread_id: u32) -> Option<usize>;
    fn descendant_thread_ids(&mut self, _top_window: usize, _process_id: u32) -> Vec<u32> {
        Vec::new()
    }
    fn root_window(&mut self, window: usize) -> Option<usize>;
    fn current_thread_id(&mut self) -> u32;
    fn attach_thread_input(&mut self, from: u32, to: u32, attach: bool) -> bool;
    fn restore_window(&mut self, window: usize);
    fn set_foreground(&mut self, window: usize) -> bool;
    fn set_active_window(&mut self, window: usize);
    fn set_focus(&mut self, window: usize);
    fn wait_for_focus_settle(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

fn capture_target<A: SurfaceApi>(
    api: &mut A,
    own_window: usize,
) -> Result<SurfaceTarget, SurfaceError> {
    let own_process_id = api
        .window_identity(own_window)
        .map(|(_, process_id)| process_id)
        .ok_or(SurfaceError::TargetUnavailable)?;
    let top_window = api
        .foreground_window()
        .filter(|window| *window != 0 && *window != own_window)
        .ok_or(SurfaceError::TargetUnavailable)?;
    let (top_thread_id, process_id) = api
        .window_identity(top_window)
        .filter(|(thread_id, process_id)| *thread_id != 0 && *process_id != 0)
        .ok_or(SurfaceError::TargetUnavailable)?;
    if process_id == own_process_id || !api.is_window(top_window) {
        return Err(SurfaceError::TargetUnavailable);
    }
    Ok(build_target(api, top_window, process_id, top_thread_id))
}

fn validate_candidate<A: SurfaceApi>(
    api: &mut A,
    window: usize,
    process_id: u32,
    own_window: usize,
) -> Result<SurfaceTarget, SurfaceError> {
    if window == 0 || process_id == 0 || window == own_window || !api.is_window(window) {
        return Err(SurfaceError::TargetUnavailable);
    }
    let own_process_id = api
        .window_identity(own_window)
        .map(|(_, process_id)| process_id)
        .ok_or(SurfaceError::TargetUnavailable)?;
    let (top_thread_id, actual_process_id) = api
        .window_identity(window)
        .ok_or(SurfaceError::TargetUnavailable)?;
    if process_id == own_process_id || actual_process_id != process_id || top_thread_id == 0 {
        return Err(SurfaceError::TargetUnavailable);
    }
    Ok(build_target(api, window, process_id, top_thread_id))
}

fn build_target<A: SurfaceApi>(
    api: &mut A,
    top_window: usize,
    process_id: u32,
    top_thread_id: u32,
) -> SurfaceTarget {
    let focus = capture_focus(api, top_window, process_id, top_thread_id);
    let target = SurfaceTarget {
        top_window,
        process_id,
        top_thread_id,
        focus_window: focus.map(|(window, _)| window),
        focus_thread_id: focus.map(|(_, thread)| thread),
    };
    debug_qa::trace(format!(
        "surface target captured target_top={:#x} process_id={} top_thread={} focus={:?} focus_thread={:?} focus_kind={}",
        target.top_window,
        target.process_id,
        target.top_thread_id,
        target.focus_window.map(|window| format!("{window:#x}")),
        target.focus_thread_id,
        if target.focus_window.is_some_and(|window| window != target.top_window) {
            "descendant"
        } else {
            "top_or_none"
        }
    ));
    target
}

fn capture_focus<A: SurfaceApi>(
    api: &mut A,
    top_window: usize,
    process_id: u32,
    top_thread_id: u32,
) -> Option<(usize, u32)> {
    let top_thread_focus =
        active_thread_focus(api, top_thread_id, top_window, process_id, "top_thread");
    if top_thread_focus.is_some_and(|(window, _)| window != top_window) {
        return top_thread_focus;
    }
    for thread_id in api.descendant_thread_ids(top_window, process_id) {
        if thread_id == 0 || thread_id == top_thread_id {
            continue;
        }
        let focus =
            active_thread_focus(api, thread_id, top_window, process_id, "descendant_thread");
        if focus.is_some_and(|(window, _)| window != top_window) {
            return focus;
        }
    }
    top_thread_focus
}

fn active_thread_focus<A: SurfaceApi>(
    api: &mut A,
    thread_id: u32,
    top_window: usize,
    process_id: u32,
    source: &'static str,
) -> Option<(usize, u32)> {
    let active_window = api.active_window(thread_id);
    let active_root = active_window.and_then(|window| api.root_window(window));
    if active_root != Some(top_window) {
        debug_qa::trace(format!(
            "surface focus candidate rejected source={source} thread={thread_id} reason=inactive active={:?} active_root={:?} target_top={:#x}",
            active_window.map(|window| format!("{window:#x}")),
            active_root.map(|window| format!("{window:#x}")),
            top_window
        ));
        return None;
    }
    let focus_window = api.focused_window(thread_id)?;
    let focus = valid_focus(api, focus_window, top_window, process_id);
    debug_qa::trace(format!(
        "surface focus candidate source={source} thread={thread_id} focus={:#x} target_top={:#x} result={}",
        focus_window,
        top_window,
        if focus.is_some() { "selected" } else { "rejected" }
    ));
    focus
}

fn valid_focus<A: SurfaceApi>(
    api: &mut A,
    focus_window: usize,
    top_window: usize,
    process_id: u32,
) -> Option<(usize, u32)> {
    let (focus_thread_id, focus_process_id) = api.window_identity(focus_window)?;
    let root = api.root_window(focus_window)?;
    (focus_process_id == process_id && root == top_window && api.is_window(focus_window))
        .then_some((focus_window, focus_thread_id))
}

fn captured_focus_satisfies(
    target: SurfaceTarget,
    requirement: SurfaceInputTargetRequirement,
) -> bool {
    match requirement {
        SurfaceInputTargetRequirement::ActiveTarget => true,
        SurfaceInputTargetRequirement::FocusedDescendant => {
            target
                .focus_window
                .is_some_and(|focus_window| focus_window != target.top_window)
                && target
                    .focus_thread_id
                    .is_some_and(|focus_thread_id| focus_thread_id != 0)
        }
    }
}

fn target_is_valid<A: SurfaceApi>(api: &mut A, target: SurfaceTarget) -> bool {
    if !api.is_window(target.top_window)
        || api.window_identity(target.top_window) != Some((target.top_thread_id, target.process_id))
    {
        return false;
    }
    match (target.focus_window, target.focus_thread_id) {
        (Some(focus), Some(thread)) => {
            api.is_window(focus)
                && api.window_identity(focus) == Some((thread, target.process_id))
                && api.root_window(focus) == Some(target.top_window)
        }
        (None, None) => true,
        _ => false,
    }
}

fn restore_target<A: SurfaceApi>(
    api: &mut A,
    target: SurfaceTarget,
    requirement: SurfaceInputTargetRequirement,
) -> Result<bool, SurfaceError> {
    if !target_is_valid(api, target) {
        return Err(SurfaceError::TargetUnavailable);
    }
    // A WS_EX_NOACTIVATE surface leaves the external application foreground
    // and preserves its focused child. This is the normal path: verify and
    // proceed without ShowWindow, SetForegroundWindow or AttachThreadInput.
    if api.foreground_window() == Some(target.top_window)
        && target_focus_is_confirmed(api, target, requirement)
    {
        return Ok(false);
    }

    // Compatibility fallback for older/foreign window behavior that did move
    // focus while the surface was open.
    api.restore_window(target.top_window);
    let _ = api.set_foreground(target.top_window);
    for _ in 0..10 {
        if target_is_valid(api, target) && api.foreground_window() == Some(target.top_window) {
            let focus_restored = restore_target_focus(api, target, requirement)?;
            if target_is_valid(api, target)
                && api.foreground_window() == Some(target.top_window)
                && target_focus_is_confirmed(api, target, requirement)
            {
                return Ok(focus_restored);
            }
            return Err(SurfaceError::FocusDenied);
        }
        std::thread::sleep(Duration::from_millis(20));
        if !target_is_valid(api, target) {
            return Err(SurfaceError::TargetUnavailable);
        }
    }
    Err(SurfaceError::FocusDenied)
}

fn restore_target_focus<A: SurfaceApi>(
    api: &mut A,
    target: SurfaceTarget,
    requirement: SurfaceInputTargetRequirement,
) -> Result<bool, SurfaceError> {
    let (Some(focus_window), Some(focus_thread_id)) = (target.focus_window, target.focus_thread_id)
    else {
        return Ok(false);
    };
    if target_focus_is_confirmed(api, target, requirement) {
        return Ok(false);
    }

    let current_thread_id = api.current_thread_id();
    let mut attached = Vec::new();
    for target_thread_id in [target.top_thread_id, focus_thread_id] {
        if target_thread_id == current_thread_id || attached.contains(&target_thread_id) {
            continue;
        }
        if !api.attach_thread_input(current_thread_id, target_thread_id, true) {
            for attached_thread in attached.into_iter().rev() {
                let _ = api.attach_thread_input(current_thread_id, attached_thread, false);
            }
            return Err(SurfaceError::InputAttachmentDenied);
        }
        attached.push(target_thread_id);
    }
    api.set_active_window(target.top_window);
    api.set_focus(focus_window);
    let mut detached = true;
    for attached_thread in attached.into_iter().rev() {
        detached &= api.attach_thread_input(current_thread_id, attached_thread, false);
    }
    if !detached {
        return Err(SurfaceError::InputAttachmentDenied);
    }

    for _ in 0..10 {
        if !target_is_valid(api, target) {
            return Err(SurfaceError::TargetUnavailable);
        }
        if api.foreground_window() == Some(target.top_window)
            && target_focus_is_confirmed(api, target, requirement)
        {
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(SurfaceError::FocusDenied)
}

fn target_focus_is_confirmed<A: SurfaceApi>(
    api: &mut A,
    target: SurfaceTarget,
    requirement: SurfaceInputTargetRequirement,
) -> bool {
    if api.active_window(target.top_thread_id) != Some(target.top_window) {
        return false;
    }
    if !captured_focus_satisfies(target, requirement) {
        return false;
    }
    match (target.focus_window, target.focus_thread_id) {
        (Some(focus_window), Some(focus_thread_id)) => {
            let focus_thread_active_root = api
                .active_window(focus_thread_id)
                .and_then(|window| api.root_window(window));
            focus_thread_active_root == Some(target.top_window)
                && api.focused_window(focus_thread_id) == Some(focus_window)
        }
        (None, None) => true,
        _ => false,
    }
}

fn restore_and_run_locked<T, F, A: SurfaceApi>(
    state_guard: MutexGuard<'_, SurfaceState>,
    api: &mut A,
    target: SurfaceTarget,
    generation: u64,
    requirement: SurfaceInputTargetRequirement,
    operation: F,
) -> Result<(u64, T), SurfaceError>
where
    F: FnOnce() -> T,
{
    debug_qa::trace(format!(
        "surface input restore begin generation={generation} requirement={} target_top={:#x} captured_focus={:?} captured_focus_thread={:?}",
        requirement.as_str(),
        target.top_window,
        target.focus_window.map(|window| format!("{window:#x}")),
        target.focus_thread_id
    ));
    let focus_restored = restore_target(api, target, requirement)?;
    if !target_is_valid(api, target) {
        return Err(SurfaceError::TargetUnavailable);
    }
    if api.foreground_window() != Some(target.top_window) {
        return Err(SurfaceError::FocusDenied);
    }
    if !target_focus_is_confirmed(api, target, requirement) {
        return Err(SurfaceError::FocusDenied);
    }
    trace_focus(api, target, requirement, "restored");
    if requirement == SurfaceInputTargetRequirement::FocusedDescendant && focus_restored {
        api.wait_for_focus_settle(Duration::from_millis(35));
        if !target_is_valid(api, target)
            || api.foreground_window() != Some(target.top_window)
            || !target_focus_is_confirmed(api, target, requirement)
        {
            return Err(SurfaceError::FocusDenied);
        }
    }
    trace_focus(api, target, requirement, "pre_send");
    let result = operation();
    trace_focus(api, target, requirement, "post_send");
    if !target_is_valid(api, target)
        || api.foreground_window() != Some(target.top_window)
        || !target_focus_is_confirmed(api, target, requirement)
    {
        return Err(SurfaceError::FocusDenied);
    }
    drop(state_guard);
    Ok((generation, result))
}

fn trace_focus<A: SurfaceApi>(
    api: &mut A,
    target: SurfaceTarget,
    requirement: SurfaceInputTargetRequirement,
    stage: &'static str,
) {
    let observed_thread = target.focus_thread_id.unwrap_or(target.top_thread_id);
    let observed_focus = api.focused_window(observed_thread);
    debug_qa::trace(format!(
        "surface input focus stage={stage} requirement={} target_top={:#x} captured_focus={:?} observed_focus={:?} observed_thread={}",
        requirement.as_str(),
        target.top_window,
        target.focus_window.map(|window| format!("{window:#x}")),
        observed_focus.map(|window| format!("{window:#x}")),
        observed_thread
    ));
}

#[cfg(windows)]
struct SystemSurfaceApi;

#[cfg(windows)]
struct DescendantThreadCollector {
    top_window: windows_sys::Win32::Foundation::HWND,
    process_id: u32,
    thread_ids: Vec<u32>,
}

#[cfg(windows)]
unsafe extern "system" fn collect_descendant_thread(
    window: windows_sys::Win32::Foundation::HWND,
    context: isize,
) -> i32 {
    let collector = unsafe { &mut *(context as *mut DescendantThreadCollector) };
    let mut process_id = 0;
    let thread_id = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
            window,
            &mut process_id,
        )
    };
    let root = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetAncestor(
            window,
            windows_sys::Win32::UI::WindowsAndMessaging::GA_ROOT,
        )
    };
    if process_id == collector.process_id
        && root == collector.top_window
        && thread_id != 0
        && !collector.thread_ids.contains(&thread_id)
    {
        collector.thread_ids.push(thread_id);
    }
    1
}

#[cfg(windows)]
impl SurfaceApi for SystemSurfaceApi {
    fn foreground_window(&mut self) -> Option<usize> {
        let window = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
        (!window.is_null()).then_some(window as usize)
    }
    fn is_window(&mut self, window: usize) -> bool {
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::IsWindow(window as _) != 0 }
    }
    fn window_identity(&mut self, window: usize) -> Option<(u32, u32)> {
        let mut process_id = 0;
        let thread = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
                window as _,
                &mut process_id,
            )
        };
        (thread != 0 && process_id != 0).then_some((thread, process_id))
    }
    fn active_window(&mut self, thread_id: u32) -> Option<usize> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetGUIThreadInfo, GUITHREADINFO};
        let mut info: GUITHREADINFO = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        let succeeded = unsafe { GetGUIThreadInfo(thread_id, &mut info) } != 0;
        (succeeded && !info.hwndActive.is_null()).then_some(info.hwndActive as usize)
    }
    fn focused_window(&mut self, thread_id: u32) -> Option<usize> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetGUIThreadInfo, GUITHREADINFO};
        let mut info: GUITHREADINFO = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        let succeeded = unsafe { GetGUIThreadInfo(thread_id, &mut info) } != 0;
        (succeeded && !info.hwndFocus.is_null()).then_some(info.hwndFocus as usize)
    }
    fn descendant_thread_ids(&mut self, top_window: usize, process_id: u32) -> Vec<u32> {
        let mut collector = DescendantThreadCollector {
            top_window: top_window as _,
            process_id,
            thread_ids: Vec::new(),
        };
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::EnumChildWindows(
                top_window as _,
                Some(collect_descendant_thread),
                (&mut collector as *mut DescendantThreadCollector) as isize,
            );
        }
        collector.thread_ids
    }
    fn root_window(&mut self, window: usize) -> Option<usize> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetAncestor, GA_ROOT};
        let root = unsafe { GetAncestor(window as _, GA_ROOT) };
        (!root.is_null()).then_some(root as usize)
    }
    fn current_thread_id(&mut self) -> u32 {
        use std::ptr::null_mut;
        use windows_sys::Win32::UI::WindowsAndMessaging::{PeekMessageW, MSG, PM_NOREMOVE};

        // AttachThreadInput requires both threads to own message queues. Tauri
        // commands may execute on a worker thread, so create its queue first.
        let mut message: MSG = unsafe { std::mem::zeroed() };
        unsafe {
            PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
        }
        unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
    }
    fn attach_thread_input(&mut self, from: u32, to: u32, attach: bool) -> bool {
        unsafe {
            windows_sys::Win32::System::Threading::AttachThreadInput(from, to, i32::from(attach))
                != 0
        }
    }
    fn restore_window(&mut self, window: usize) {
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(window as _, 9);
        }
    }
    fn set_foreground(&mut self, window: usize) -> bool {
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow(window as _) != 0
        }
    }
    fn set_active_window(&mut self, window: usize) {
        unsafe {
            windows_sys::Win32::UI::Input::KeyboardAndMouse::SetActiveWindow(window as _);
        }
    }
    fn set_focus(&mut self, window: usize) {
        unsafe {
            windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(window as _);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{mpsc, Arc};
    use std::time::Duration;
    struct FakeSurface {
        foreground: Option<usize>,
        valid: bool,
        own_pid: u32,
        target_pid: Option<u32>,
        focus: bool,
        restored: usize,
    }
    impl SurfaceApi for FakeSurface {
        fn foreground_window(&mut self) -> Option<usize> {
            self.foreground
        }
        fn is_window(&mut self, _window: usize) -> bool {
            self.valid
        }
        fn window_identity(&mut self, window: usize) -> Option<(u32, u32)> {
            if window == 10 {
                Some((1, self.own_pid))
            } else {
                self.target_pid.map(|process_id| (2, process_id))
            }
        }
        fn active_window(&mut self, _thread_id: u32) -> Option<usize> {
            self.foreground
        }
        fn focused_window(&mut self, _thread_id: u32) -> Option<usize> {
            None
        }
        fn root_window(&mut self, window: usize) -> Option<usize> {
            Some(window)
        }
        fn current_thread_id(&mut self) -> u32 {
            9
        }
        fn attach_thread_input(&mut self, _from: u32, _to: u32, _attach: bool) -> bool {
            true
        }
        fn restore_window(&mut self, _window: usize) {
            self.restored += 1;
        }
        fn set_foreground(&mut self, window: usize) -> bool {
            if self.focus {
                self.foreground = Some(window);
            }
            self.focus
        }
        fn set_active_window(&mut self, _window: usize) {}
        fn set_focus(&mut self, _window: usize) {}
    }
    #[test]
    fn capture_excludes_own_window_and_restore_revalidates_pid_and_foreground() {
        let mut own = FakeSurface {
            foreground: Some(10),
            valid: true,
            own_pid: 1,
            target_pid: Some(2),
            focus: true,
            restored: 0,
        };
        assert_eq!(
            capture_target(&mut own, 10),
            Err(SurfaceError::TargetUnavailable)
        );
        let mut api = FakeSurface {
            foreground: Some(20),
            valid: true,
            own_pid: 1,
            target_pid: Some(2),
            focus: true,
            restored: 0,
        };
        let target = capture_target(&mut api, 10).unwrap();
        api.foreground = Some(99);
        restore_target(
            &mut api,
            target,
            SurfaceInputTargetRequirement::ActiveTarget,
        )
        .unwrap();
        assert_eq!(api.foreground, Some(20));
        assert_eq!(api.restored, 1);
        api.target_pid = Some(3);
        assert_eq!(
            restore_target(
                &mut api,
                target,
                SurfaceInputTargetRequirement::ActiveTarget
            ),
            Err(SurfaceError::TargetUnavailable)
        );
    }

    #[test]
    fn hook_candidate_is_revalidated_before_surface_activation() {
        let mut api = FakeSurface {
            foreground: Some(20),
            valid: true,
            own_pid: 1,
            target_pid: Some(2),
            focus: true,
            restored: 0,
        };
        assert_eq!(
            validate_candidate(&mut api, 20, 2, 10).unwrap(),
            SurfaceTarget {
                top_window: 20,
                process_id: 2,
                top_thread_id: 2,
                focus_window: None,
                focus_thread_id: None,
            }
        );
        assert_eq!(
            validate_candidate(&mut api, 20, 3, 10),
            Err(SurfaceError::TargetUnavailable)
        );
        assert_eq!(
            validate_candidate(&mut api, 10, 1, 10),
            Err(SurfaceError::TargetUnavailable)
        );
    }

    #[test]
    fn capture_searches_descendant_ui_threads_for_the_real_focused_child() {
        struct MultiThreadFocusSurface;

        impl SurfaceApi for MultiThreadFocusSurface {
            fn foreground_window(&mut self) -> Option<usize> {
                Some(20)
            }
            fn is_window(&mut self, window: usize) -> bool {
                matches!(window, 10 | 20 | 21 | 22 | 30)
            }
            fn window_identity(&mut self, window: usize) -> Option<(u32, u32)> {
                match window {
                    10 => Some((1, 1)),
                    20 => Some((2, 2)),
                    21 => Some((3, 2)),
                    22 => Some((4, 2)),
                    30 => Some((4, 3)),
                    _ => None,
                }
            }
            fn active_window(&mut self, thread_id: u32) -> Option<usize> {
                Some(if thread_id == 4 { 30 } else { 20 })
            }
            fn focused_window(&mut self, thread_id: u32) -> Option<usize> {
                match thread_id {
                    2 => Some(20),
                    3 => Some(21),
                    4 => Some(22),
                    _ => None,
                }
            }
            fn descendant_thread_ids(&mut self, _top_window: usize, _process_id: u32) -> Vec<u32> {
                // Thread 4 reports a same-process descendant focus, but its
                // active window belongs to another root and is stale.
                vec![2, 4, 3]
            }
            fn root_window(&mut self, window: usize) -> Option<usize> {
                match window {
                    20..=22 => Some(20),
                    10 => Some(10),
                    30 => Some(30),
                    _ => None,
                }
            }
            fn current_thread_id(&mut self) -> u32 {
                9
            }
            fn attach_thread_input(&mut self, _from: u32, _to: u32, _attach: bool) -> bool {
                true
            }
            fn restore_window(&mut self, _window: usize) {}
            fn set_foreground(&mut self, _window: usize) -> bool {
                true
            }
            fn set_active_window(&mut self, _window: usize) {}
            fn set_focus(&mut self, _window: usize) {}
        }

        let target = capture_target(&mut MultiThreadFocusSurface, 10).unwrap();

        assert_eq!(target.focus_window, Some(21));
        assert_eq!(target.focus_thread_id, Some(3));
    }

    #[test]
    fn file_input_handoff_requires_a_captured_descendant_focus() {
        for focus_window in [None, Some(20)] {
            let manager = SurfaceManager::default();
            let mut state = manager.state.lock().unwrap();
            state.target = Some(SurfaceTarget {
                top_window: 20,
                process_id: 2,
                top_thread_id: 2,
                focus_window,
                focus_thread_id: focus_window.map(|_| 2),
            });
            state.active = true;
            state.generation = 4;
            drop(state);

            assert!(matches!(
                manager.begin_input_handoff_for(SurfaceInputTargetRequirement::FocusedDescendant),
                Err(SurfaceError::FocusDenied)
            ));
        }

        let manager = SurfaceManager::default();
        let mut state = manager.state.lock().unwrap();
        state.target = Some(SurfaceTarget {
            top_window: 20,
            process_id: 2,
            top_thread_id: 2,
            focus_window: Some(21),
            focus_thread_id: Some(3),
        });
        state.active = true;
        state.generation = 5;
        drop(state);
        assert!(manager
            .begin_input_handoff_for(SurfaceInputTargetRequirement::FocusedDescendant)
            .is_ok());
    }

    #[test]
    fn no_activate_target_fast_path_does_not_restore_or_set_focus_again() {
        #[derive(Default)]
        struct NoActivateSurface {
            restore_calls: usize,
            foreground_calls: usize,
            active_calls: usize,
            focus_calls: usize,
            attachment_calls: usize,
        }
        impl SurfaceApi for NoActivateSurface {
            fn foreground_window(&mut self) -> Option<usize> {
                Some(20)
            }
            fn is_window(&mut self, window: usize) -> bool {
                matches!(window, 20 | 21)
            }
            fn window_identity(&mut self, window: usize) -> Option<(u32, u32)> {
                match window {
                    20 => Some((2, 2)),
                    21 => Some((3, 2)),
                    _ => None,
                }
            }
            fn active_window(&mut self, _thread_id: u32) -> Option<usize> {
                Some(20)
            }
            fn focused_window(&mut self, _thread_id: u32) -> Option<usize> {
                Some(21)
            }
            fn root_window(&mut self, window: usize) -> Option<usize> {
                matches!(window, 20 | 21).then_some(20)
            }
            fn current_thread_id(&mut self) -> u32 {
                9
            }
            fn attach_thread_input(&mut self, _from: u32, _to: u32, _attach: bool) -> bool {
                self.attachment_calls += 1;
                true
            }
            fn restore_window(&mut self, _window: usize) {
                self.restore_calls += 1;
            }
            fn set_foreground(&mut self, _window: usize) -> bool {
                self.foreground_calls += 1;
                true
            }
            fn set_active_window(&mut self, _window: usize) {
                self.active_calls += 1;
            }
            fn set_focus(&mut self, _window: usize) {
                self.focus_calls += 1;
            }
        }

        let target = SurfaceTarget {
            top_window: 20,
            process_id: 2,
            top_thread_id: 2,
            focus_window: Some(21),
            focus_thread_id: Some(3),
        };
        let mut api = NoActivateSurface::default();
        restore_target(
            &mut api,
            target,
            SurfaceInputTargetRequirement::ActiveTarget,
        )
        .unwrap();

        assert_eq!(api.restore_calls, 0);
        assert_eq!(api.foreground_calls, 0);
        assert_eq!(api.active_calls, 0);
        assert_eq!(api.focus_calls, 0);
        assert_eq!(api.attachment_calls, 0);
    }

    #[test]
    fn explorer_handoff_restores_target_when_internal_surface_root_is_foreground() {
        let manager = SurfaceManager::default();
        let mut state = manager.state.lock().unwrap();
        state.target = Some(SurfaceTarget {
            top_window: 20,
            process_id: 2,
            top_thread_id: 2,
            focus_window: None,
            focus_thread_id: None,
        });
        state.active = true;
        state.generation = 6;
        let target = state.target.unwrap();
        let mut api = FakeSurface {
            // 10 represents the transiently foreground internal popup root.
            foreground: Some(10),
            valid: true,
            own_pid: 1,
            target_pid: Some(2),
            focus: true,
            restored: 0,
        };

        let result = restore_and_run_locked(
            state,
            &mut api,
            target,
            6,
            SurfaceInputTargetRequirement::ActiveTarget,
            || "input_sent",
        )
        .unwrap();

        assert_eq!(result, (6, "input_sent"));
        assert_eq!(api.foreground, Some(20));
        assert_eq!(api.restored, 1);
    }

    #[test]
    fn captured_focus_child_is_restored_with_balanced_thread_attachment_before_input() {
        struct FocusSurface {
            foreground: Option<usize>,
            focus: Option<usize>,
            attachments: Vec<(u32, u32, bool)>,
            attach_results: VecDeque<bool>,
        }
        impl SurfaceApi for FocusSurface {
            fn foreground_window(&mut self) -> Option<usize> {
                self.foreground
            }
            fn is_window(&mut self, window: usize) -> bool {
                matches!(window, 10 | 20 | 21)
            }
            fn window_identity(&mut self, window: usize) -> Option<(u32, u32)> {
                match window {
                    10 => Some((1, 1)),
                    20 => Some((2, 2)),
                    21 => Some((3, 2)),
                    _ => None,
                }
            }
            fn active_window(&mut self, _thread_id: u32) -> Option<usize> {
                self.foreground
            }
            fn focused_window(&mut self, _thread_id: u32) -> Option<usize> {
                self.focus
            }
            fn root_window(&mut self, window: usize) -> Option<usize> {
                match window {
                    20 | 21 => Some(20),
                    10 => Some(10),
                    _ => None,
                }
            }
            fn current_thread_id(&mut self) -> u32 {
                9
            }
            fn attach_thread_input(&mut self, from: u32, to: u32, attach: bool) -> bool {
                self.attachments.push((from, to, attach));
                self.attach_results.pop_front().unwrap_or(true)
            }
            fn restore_window(&mut self, _window: usize) {}
            fn set_foreground(&mut self, window: usize) -> bool {
                self.foreground = Some(window);
                true
            }
            fn set_active_window(&mut self, _window: usize) {}
            fn set_focus(&mut self, window: usize) {
                self.focus = Some(window);
            }
        }

        let mut api = FocusSurface {
            foreground: Some(20),
            focus: Some(21),
            attachments: Vec::new(),
            attach_results: VecDeque::from([true, true, true, true]),
        };
        let target = capture_target(&mut api, 10).unwrap();
        assert_eq!(target.focus_window, Some(21));
        assert_eq!(target.focus_thread_id, Some(3));
        api.foreground = Some(10);
        api.focus = None;

        restore_target(
            &mut api,
            target,
            SurfaceInputTargetRequirement::FocusedDescendant,
        )
        .unwrap();

        assert_eq!(api.foreground, Some(20));
        assert_eq!(api.focus, Some(21));
        assert_eq!(
            api.attachments,
            vec![(9, 2, true), (9, 3, true), (9, 3, false), (9, 2, false)]
        );

        api.foreground = Some(10);
        api.focus = None;
        api.attachments.clear();
        api.attach_results = VecDeque::from([false]);
        assert_eq!(
            restore_target(
                &mut api,
                target,
                SurfaceInputTargetRequirement::FocusedDescendant
            ),
            Err(SurfaceError::InputAttachmentDenied)
        );
        assert_eq!(api.attachments, vec![(9, 2, true)]);

        api.foreground = Some(10);
        api.focus = None;
        api.attachments.clear();
        api.attach_results = VecDeque::from([true, true, false, true]);
        assert_eq!(
            restore_target(
                &mut api,
                target,
                SurfaceInputTargetRequirement::FocusedDescendant
            ),
            Err(SurfaceError::InputAttachmentDenied)
        );
        assert_eq!(
            api.attachments,
            vec![(9, 2, true), (9, 3, true), (9, 3, false), (9, 2, false)]
        );
    }

    #[test]
    fn restored_descendant_focus_is_settled_and_reconfirmed_before_file_input() {
        struct RestoringFocusSurface {
            foreground: usize,
            focus: usize,
            focus_calls: Vec<usize>,
            settle_calls: usize,
        }

        impl SurfaceApi for RestoringFocusSurface {
            fn foreground_window(&mut self) -> Option<usize> {
                Some(self.foreground)
            }
            fn is_window(&mut self, window: usize) -> bool {
                matches!(window, 10 | 20 | 21)
            }
            fn window_identity(&mut self, window: usize) -> Option<(u32, u32)> {
                match window {
                    10 => Some((1, 1)),
                    20 => Some((2, 2)),
                    21 => Some((3, 2)),
                    _ => None,
                }
            }
            fn active_window(&mut self, _thread_id: u32) -> Option<usize> {
                Some(self.foreground)
            }
            fn focused_window(&mut self, _thread_id: u32) -> Option<usize> {
                Some(self.focus)
            }
            fn root_window(&mut self, window: usize) -> Option<usize> {
                match window {
                    20 | 21 => Some(20),
                    10 => Some(10),
                    _ => None,
                }
            }
            fn current_thread_id(&mut self) -> u32 {
                9
            }
            fn attach_thread_input(&mut self, _from: u32, _to: u32, _attach: bool) -> bool {
                true
            }
            fn restore_window(&mut self, _window: usize) {}
            fn set_foreground(&mut self, window: usize) -> bool {
                self.foreground = window;
                true
            }
            fn set_active_window(&mut self, _window: usize) {}
            fn set_focus(&mut self, window: usize) {
                self.focus = window;
                self.focus_calls.push(window);
            }
            fn wait_for_focus_settle(&mut self, _duration: Duration) {
                self.settle_calls += 1;
            }
        }

        let manager = SurfaceManager::default();
        let mut state = manager.state.lock().unwrap();
        state.target = Some(SurfaceTarget {
            top_window: 20,
            process_id: 2,
            top_thread_id: 2,
            focus_window: Some(21),
            focus_thread_id: Some(3),
        });
        state.active = true;
        state.generation = 4;
        let target = state.target.unwrap();
        let mut api = RestoringFocusSurface {
            foreground: 10,
            focus: 20,
            focus_calls: Vec::new(),
            settle_calls: 0,
        };
        let mut operation_ran = false;

        let result = restore_and_run_locked(
            state,
            &mut api,
            target,
            4,
            SurfaceInputTargetRequirement::FocusedDescendant,
            || operation_ran = true,
        );

        assert_eq!(result, Ok((4, ())));
        assert!(operation_ran);
        assert_eq!(api.focus_calls, vec![21]);
        assert_eq!(api.settle_calls, 1);
    }

    #[test]
    fn handoff_guard_ignores_every_focus_loss_until_explicit_drop() {
        let manager = SurfaceManager::default();
        {
            let mut state = manager.state.lock().unwrap();
            state.generation = 5;
            state.active = true;
            state.target = Some(SurfaceTarget {
                top_window: 20,
                process_id: 2,
                top_thread_id: 2,
                focus_window: None,
                focus_thread_id: None,
            });
        }
        let handoff = manager
            .begin_input_handoff_for(SurfaceInputTargetRequirement::ActiveTarget)
            .unwrap();
        for _ in 0..4 {
            assert!(!manager.should_close_on_focus_loss());
        }
        drop(handoff);
        assert!(!manager.should_close_on_focus_loss());
        manager
            .focus_loss_grace_until_ms
            .store(0, Ordering::Release);
        assert!(manager.should_close_on_focus_loss());
        assert!(manager.surface_active());
    }

    #[test]
    fn operation_holds_identity_lock_and_old_generation_cannot_clear_new_surface() {
        let manager = Arc::new(SurfaceManager::default());
        {
            let mut state = manager.state.lock().unwrap();
            state.target = Some(SurfaceTarget {
                top_window: 20,
                process_id: 2,
                top_thread_id: 2,
                focus_window: None,
                focus_thread_id: None,
            });
            state.active = true;
            state.generation = 7;
        }
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker_manager = Arc::clone(&manager);
        let worker = std::thread::spawn(move || {
            let state_guard = worker_manager.state.lock().unwrap();
            let target = state_guard.target.unwrap();
            let generation = state_guard.generation;
            let mut api = FakeSurface {
                foreground: Some(20),
                valid: true,
                own_pid: 1,
                target_pid: Some(2),
                focus: true,
                restored: 0,
            };
            restore_and_run_locked(
                state_guard,
                &mut api,
                target,
                generation,
                SurfaceInputTargetRequirement::ActiveTarget,
                || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                },
            )
            .unwrap()
            .0
        });
        entered_rx.recv().unwrap();
        let (clear_tx, clear_rx) = mpsc::channel();
        let clear_manager = Arc::clone(&manager);
        let clearer = std::thread::spawn(move || {
            clear_manager.clear().unwrap();
            clear_tx.send(()).unwrap();
        });
        assert!(clear_rx.recv_timeout(Duration::from_millis(30)).is_err());
        release_tx.send(()).unwrap();
        assert_eq!(worker.join().unwrap(), 7);
        clear_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        clearer.join().unwrap();

        {
            let mut state = manager.state.lock().unwrap();
            state.target = Some(SurfaceTarget {
                top_window: 30,
                process_id: 3,
                top_thread_id: 2,
                focus_window: None,
                focus_thread_id: None,
            });
            state.active = true;
            state.generation = 9;
        }
        assert!(!manager.clear_if_generation(7));
        assert!(manager.surface_active());
        assert!(manager.clear_if_generation(9));
        assert!(!manager.surface_active());
    }

    #[test]
    fn browse_only_session_is_active_without_claiming_input_target() {
        let manager = SurfaceManager::default();

        let generation = manager.activate_without_target().unwrap();

        assert!(manager.surface_active());
        assert!(!manager.input_available());
        assert!(manager.clear_if_generation(generation));
        assert!(!manager.surface_active());
    }

    #[test]
    fn expected_input_handoff_focus_loss_does_not_close_retryable_surface() {
        let manager = SurfaceManager::default();
        manager.input_handoff_generation.store(7, Ordering::Release);

        assert!(!manager.should_close_on_focus_loss());
        assert!(!manager.should_close_on_focus_loss());

        manager.clear().unwrap();
        assert!(manager.should_close_on_focus_loss());
    }

    #[test]
    fn target_focus_moving_during_operation_is_reported_as_failure_not_success() {
        struct ScriptedSurface {
            foreground: VecDeque<Option<usize>>,
        }
        impl SurfaceApi for ScriptedSurface {
            fn foreground_window(&mut self) -> Option<usize> {
                self.foreground.pop_front().unwrap_or(Some(99))
            }
            fn is_window(&mut self, _window: usize) -> bool {
                true
            }
            fn window_identity(&mut self, window: usize) -> Option<(u32, u32)> {
                (window == 20).then_some((2, 2))
            }
            fn active_window(&mut self, _thread_id: u32) -> Option<usize> {
                Some(20)
            }
            fn focused_window(&mut self, _thread_id: u32) -> Option<usize> {
                None
            }
            fn root_window(&mut self, window: usize) -> Option<usize> {
                Some(window)
            }
            fn current_thread_id(&mut self) -> u32 {
                9
            }
            fn attach_thread_input(&mut self, _from: u32, _to: u32, _attach: bool) -> bool {
                true
            }
            fn restore_window(&mut self, _window: usize) {}
            fn set_foreground(&mut self, _window: usize) -> bool {
                true
            }
            fn set_active_window(&mut self, _window: usize) {}
            fn set_focus(&mut self, _window: usize) {}
        }

        let manager = SurfaceManager::default();
        let mut state = manager.state.lock().unwrap();
        state.target = Some(SurfaceTarget {
            top_window: 20,
            process_id: 2,
            top_thread_id: 2,
            focus_window: None,
            focus_thread_id: None,
        });
        state.active = true;
        state.generation = 4;
        let target = state.target.unwrap();
        let mut api = ScriptedSurface {
            foreground: VecDeque::from([Some(20), Some(20), Some(99)]),
        };
        assert_eq!(
            restore_and_run_locked(
                state,
                &mut api,
                target,
                4,
                SurfaceInputTargetRequirement::ActiveTarget,
                || 1
            ),
            Err(SurfaceError::FocusDenied)
        );
    }

    #[test]
    fn descendant_focus_moving_after_keys_are_sent_is_reported_as_failure() {
        struct ScriptedChildFocusSurface {
            focus: VecDeque<Option<usize>>,
        }

        impl SurfaceApi for ScriptedChildFocusSurface {
            fn foreground_window(&mut self) -> Option<usize> {
                Some(20)
            }
            fn is_window(&mut self, window: usize) -> bool {
                matches!(window, 20 | 21)
            }
            fn window_identity(&mut self, window: usize) -> Option<(u32, u32)> {
                match window {
                    20 => Some((2, 2)),
                    21 => Some((3, 2)),
                    _ => None,
                }
            }
            fn active_window(&mut self, _thread_id: u32) -> Option<usize> {
                Some(20)
            }
            fn focused_window(&mut self, _thread_id: u32) -> Option<usize> {
                self.focus.pop_front().unwrap_or(Some(20))
            }
            fn root_window(&mut self, _window: usize) -> Option<usize> {
                Some(20)
            }
            fn current_thread_id(&mut self) -> u32 {
                9
            }
            fn attach_thread_input(&mut self, _from: u32, _to: u32, _attach: bool) -> bool {
                true
            }
            fn restore_window(&mut self, _window: usize) {}
            fn set_foreground(&mut self, _window: usize) -> bool {
                true
            }
            fn set_active_window(&mut self, _window: usize) {}
            fn set_focus(&mut self, _window: usize) {}
        }

        let manager = SurfaceManager::default();
        let mut state = manager.state.lock().unwrap();
        state.target = Some(SurfaceTarget {
            top_window: 20,
            process_id: 2,
            top_thread_id: 2,
            focus_window: Some(21),
            focus_thread_id: Some(3),
        });
        state.active = true;
        state.generation = 4;
        let target = state.target.unwrap();
        let mut api = ScriptedChildFocusSurface {
            focus: VecDeque::from([Some(21), Some(21), Some(21), Some(21), Some(20), Some(20)]),
        };
        let mut operation_ran = false;

        let result = restore_and_run_locked(
            state,
            &mut api,
            target,
            4,
            SurfaceInputTargetRequirement::FocusedDescendant,
            || operation_ran = true,
        );

        assert_eq!(result, Err(SurfaceError::FocusDenied));
        assert!(operation_ran);
    }
}
