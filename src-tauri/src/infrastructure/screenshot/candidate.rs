use super::model::{PhysicalPoint, PhysicalRect, VirtualDesktopSnapshot};

#[cfg(windows)]
pub use windows_impl::CaptureCandidateDetector;

#[cfg(not(windows))]
pub struct CaptureCandidateDetector;

#[cfg(not(windows))]
impl CaptureCandidateDetector {
    pub fn snapshot(_desktop: &VirtualDesktopSnapshot) -> Self {
        Self
    }

    pub fn candidate_at(&mut self, _point: PhysicalPoint) -> Option<PhysicalRect> {
        None
    }
}

#[cfg(windows)]
mod windows_impl {
    use std::cmp::Reverse;
    use std::collections::{HashMap, HashSet};
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Condvar, Mutex, OnceLock,
    };
    use std::thread;
    use std::time::{Duration, Instant};

    use windows::core::{Interface, BSTR};
    use windows::Win32::Foundation::HWND as AutomationHwnd;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Accessibility::{
        AutomationElementMode_None, CUIAutomation, IUIAutomation, IUIAutomation2,
        IUIAutomationCacheRequest, IUIAutomationElement, TreeScope_Subtree,
        UIA_AutomationIdPropertyId, UIA_BoundingRectanglePropertyId, UIA_ButtonControlTypeId,
        UIA_CheckBoxControlTypeId, UIA_ClassNamePropertyId, UIA_ComboBoxControlTypeId,
        UIA_ControlTypePropertyId, UIA_CustomControlTypeId, UIA_DataGridControlTypeId,
        UIA_DataItemControlTypeId, UIA_DocumentControlTypeId, UIA_EditControlTypeId,
        UIA_FrameworkIdPropertyId, UIA_GroupControlTypeId, UIA_HyperlinkControlTypeId,
        UIA_IsContentElementPropertyId, UIA_IsControlElementPropertyId, UIA_IsOffscreenPropertyId,
        UIA_ListControlTypeId, UIA_ListItemControlTypeId, UIA_MenuItemControlTypeId,
        UIA_NamePropertyId, UIA_PaneControlTypeId, UIA_RadioButtonControlTypeId,
        UIA_SliderControlTypeId, UIA_SpinnerControlTypeId, UIA_SplitButtonControlTypeId,
        UIA_TabItemControlTypeId, UIA_TableControlTypeId, UIA_TextControlTypeId,
        UIA_TreeControlTypeId, UIA_TreeItemControlTypeId, UIA_WindowControlTypeId,
    };
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, POINT, RECT};
    use windows_sys::Win32::Graphics::Dwm::{
        DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
    };
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, EnumWindows, GetClassNameW, GetClientRect, GetWindowRect,
        GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    };

    use super::{PhysicalPoint, PhysicalRect, VirtualDesktopSnapshot};
    use crate::infrastructure::screenshot::model::MonitorFrame;

    const MIN_CANDIDATE_EDGE: u32 = 3;
    const MAX_CACHED_AUTOMATION_NODES: usize = 20_000;
    const UIA_CONNECTION_TIMEOUT_MS: u32 = 120;
    const UIA_TRANSACTION_TIMEOUT_MS: u32 = 160;
    const SLOW_PROVIDER_THRESHOLD: Duration = Duration::from_millis(180);
    const MIN_MEANINGFUL_AUTOMATION_SCORE: i32 = 400;
    const MAX_SEMANTIC_TEXT_CHARS: usize = 256;
    const MAX_WINDOW_CLASS_CHARS: usize = 256;
    const CHAT_SURFACE_SCORE_BONUS: i32 = 300;
    const SPANNING_EDGE_MIN_DIFFERENCE: u32 = 5;
    const SPANNING_EDGE_MIN_COVERAGE_PERCENT: u64 = 80;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct WindowDescriptor {
        handle: isize,
        process_id: u32,
        bounds: PhysicalRect,
    }

    struct WindowCandidate {
        descriptor: WindowDescriptor,
        regions: Vec<PhysicalRect>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct AutomationCandidate {
        rect: PhysicalRect,
        parent: Option<usize>,
        depth: u16,
        control_type: i32,
        name: Option<String>,
        automation_id: Option<String>,
        class_name: Option<String>,
        framework_id: Option<String>,
        is_control: bool,
        is_content: bool,
        peer_count: u16,
    }

    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    struct AutomationSnapshot {
        nodes: Vec<AutomationCandidate>,
    }

    struct CandidateRequest {
        session_id: u64,
        generation: u64,
        window: WindowDescriptor,
        response_sender: Sender<CandidateResponse>,
    }

    struct CandidateResponse {
        generation: u64,
        window: WindowDescriptor,
        snapshot: AutomationSnapshot,
        elapsed: Duration,
    }

    #[derive(Default)]
    struct LatestRequestState {
        request: Option<CandidateRequest>,
    }

    #[derive(Default)]
    struct LatestRequestSlot {
        state: Mutex<LatestRequestState>,
        available: Condvar,
    }

    struct SharedAutomationWorker {
        request_slot: Arc<LatestRequestSlot>,
        _worker: thread::JoinHandle<()>,
    }

    static SHARED_AUTOMATION_WORKER: OnceLock<Option<SharedAutomationWorker>> = OnceLock::new();
    static NEXT_CANDIDATE_SESSION: AtomicU64 = AtomicU64::new(0);

    impl LatestRequestSlot {
        fn submit(&self, request: CandidateRequest) -> Option<CandidateRequest> {
            let Ok(mut state) = self.state.lock() else {
                return None;
            };
            let replaced = state.request.replace(request);
            self.available.notify_one();
            replaced
        }

        fn receive(&self) -> Option<CandidateRequest> {
            let mut state = self.state.lock().ok()?;
            loop {
                if let Some(request) = state.request.take() {
                    return Some(request);
                }
                state = self.available.wait(state).ok()?;
            }
        }
    }

    pub struct CaptureCandidateDetector {
        windows: Vec<WindowCandidate>,
        session_id: u64,
        request_slot: Option<Arc<LatestRequestSlot>>,
        response_sender: Sender<CandidateResponse>,
        response_receiver: Receiver<CandidateResponse>,
        uia_snapshots: HashMap<isize, AutomationSnapshot>,
        pending_windows: HashSet<isize>,
        slow_windows: HashSet<isize>,
        next_generation: u64,
    }

    impl CaptureCandidateDetector {
        pub fn snapshot(desktop: &VirtualDesktopSnapshot) -> Self {
            let windows = enumerate_windows(desktop);
            let (response_sender, response_receiver) = mpsc::channel();
            let request_slot = shared_automation_request_slot();

            Self {
                windows,
                session_id: NEXT_CANDIDATE_SESSION.fetch_add(1, Ordering::Relaxed) + 1,
                request_slot,
                response_sender,
                response_receiver,
                uia_snapshots: HashMap::new(),
                pending_windows: HashSet::new(),
                slow_windows: HashSet::new(),
                next_generation: 0,
            }
        }

        pub fn candidate_at(&mut self, point: PhysicalPoint) -> Option<PhysicalRect> {
            self.collect_responses();

            let window_index = self
                .windows
                .iter()
                .position(|window| contains(window.descriptor.bounds, point))?;
            let descriptor = self.windows[window_index].descriptor;

            if !self.uia_snapshots.contains_key(&descriptor.handle)
                && !self.pending_windows.contains(&descriptor.handle)
                && !self.slow_windows.contains(&descriptor.handle)
            {
                self.request_window_cache(descriptor);
            }

            best_cached_candidate(
                &self.windows[window_index],
                self.uia_snapshots.get(&descriptor.handle),
                point,
            )
        }

        fn request_window_cache(&mut self, window: WindowDescriptor) {
            let Some(slot) = self.request_slot.as_ref() else {
                return;
            };
            self.next_generation = self.next_generation.wrapping_add(1);
            let request = CandidateRequest {
                session_id: self.session_id,
                generation: self.next_generation,
                window,
                response_sender: self.response_sender.clone(),
            };
            if let Some(replaced) = slot.submit(request) {
                if replaced.session_id == self.session_id {
                    self.pending_windows.remove(&replaced.window.handle);
                }
            }
            self.pending_windows.insert(window.handle);
        }

        fn collect_responses(&mut self) {
            while let Ok(response) = self.response_receiver.try_recv() {
                self.pending_windows.remove(&response.window.handle);
                if response.elapsed >= SLOW_PROVIDER_THRESHOLD {
                    self.slow_windows.insert(response.window.handle);
                }
                if response.generation <= self.next_generation {
                    self.uia_snapshots
                        .insert(response.window.handle, response.snapshot);
                }
            }
        }
    }

    fn shared_automation_request_slot() -> Option<Arc<LatestRequestSlot>> {
        SHARED_AUTOMATION_WORKER
            .get_or_init(|| {
                let request_slot = Arc::new(LatestRequestSlot::default());
                let worker_slot = Arc::clone(&request_slot);
                let worker = thread::Builder::new()
                    .name("screenshot-uia-cache".to_owned())
                    .spawn(move || automation_worker(worker_slot))
                    .ok()?;
                Some(SharedAutomationWorker {
                    request_slot,
                    _worker: worker,
                })
            })
            .as_ref()
            .map(|worker| Arc::clone(&worker.request_slot))
    }

    fn automation_worker(request_slot: Arc<LatestRequestSlot>) {
        let com_result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let should_uninitialize = com_result.is_ok();
        let automation = unsafe {
            CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
        }
        .ok();
        if let Some(automation) = automation.as_ref() {
            configure_automation_timeouts(automation);
        }
        let cache_request = automation
            .as_ref()
            .and_then(create_automation_cache_request);

        while let Some(request) = request_slot.receive() {
            let started = Instant::now();
            let snapshot = automation
                .as_ref()
                .zip(cache_request.as_ref())
                .map(|(automation, cache_request)| {
                    cached_automation_snapshot(automation, cache_request, request.window)
                })
                .unwrap_or_default();
            let response = CandidateResponse {
                generation: request.generation,
                window: request.window,
                snapshot,
                elapsed: started.elapsed(),
            };
            let _ = request.response_sender.send(response);
        }

        drop(cache_request);
        drop(automation);
        if should_uninitialize {
            unsafe {
                CoUninitialize();
            }
        }
    }

    fn configure_automation_timeouts(automation: &IUIAutomation) {
        let Ok(automation2) = automation.cast::<IUIAutomation2>() else {
            return;
        };
        unsafe {
            let _ = automation2.SetConnectionTimeout(UIA_CONNECTION_TIMEOUT_MS);
            let _ = automation2.SetTransactionTimeout(UIA_TRANSACTION_TIMEOUT_MS);
        }
    }

    fn create_automation_cache_request(
        automation: &IUIAutomation,
    ) -> Option<IUIAutomationCacheRequest> {
        let request = unsafe { automation.CreateCacheRequest().ok()? };
        let control = unsafe { automation.ControlViewCondition().ok()? };
        let content = unsafe { automation.ContentViewCondition().ok()? };
        let visible_content = unsafe { automation.CreateOrCondition(&control, &content).ok()? };
        unsafe {
            request.AddProperty(UIA_BoundingRectanglePropertyId).ok()?;
            request.AddProperty(UIA_IsOffscreenPropertyId).ok()?;
            request.AddProperty(UIA_ControlTypePropertyId).ok()?;
            request.AddProperty(UIA_NamePropertyId).ok()?;
            request.AddProperty(UIA_AutomationIdPropertyId).ok()?;
            request.AddProperty(UIA_ClassNamePropertyId).ok()?;
            request.AddProperty(UIA_FrameworkIdPropertyId).ok()?;
            request.AddProperty(UIA_IsControlElementPropertyId).ok()?;
            request.AddProperty(UIA_IsContentElementPropertyId).ok()?;
            request.SetTreeScope(TreeScope_Subtree).ok()?;
            request.SetTreeFilter(&visible_content).ok()?;
            request
                .SetAutomationElementMode(AutomationElementMode_None)
                .ok()?;
        }
        Some(request)
    }

    fn cached_automation_snapshot(
        automation: &IUIAutomation,
        cache_request: &IUIAutomationCacheRequest,
        window: WindowDescriptor,
    ) -> AutomationSnapshot {
        let root = unsafe {
            automation
                .ElementFromHandleBuildCache(
                    AutomationHwnd(window.handle as *mut c_void),
                    cache_request,
                )
                .ok()
        };
        let Some(root) = root else {
            return AutomationSnapshot::default();
        };

        let mut snapshot = AutomationSnapshot::default();
        let mut stack = vec![(root, None, 0u16)];
        let mut visited = 0usize;
        while let Some((element, parent, depth)) = stack.pop() {
            if visited >= MAX_CACHED_AUTOMATION_NODES {
                break;
            }
            visited += 1;
            let candidate_parent = add_cached_automation_candidate(
                &element,
                window.bounds,
                parent,
                depth,
                &mut snapshot,
            )
            .or(parent);

            let Ok(children) = (unsafe { element.GetCachedChildren() }) else {
                continue;
            };
            let length = unsafe { children.Length().unwrap_or(0) }.max(0);
            for index in (0..length).rev() {
                if let Ok(child) = unsafe { children.GetElement(index) } {
                    stack.push((child, candidate_parent, depth.saturating_add(1)));
                }
            }
        }
        populate_peer_counts(&mut snapshot);
        snapshot
    }

    fn add_cached_automation_candidate(
        element: &IUIAutomationElement,
        window_bounds: PhysicalRect,
        parent: Option<usize>,
        depth: u16,
        snapshot: &mut AutomationSnapshot,
    ) -> Option<usize> {
        if unsafe { element.CachedIsOffscreen().ok() }.is_some_and(|value| value.as_bool()) {
            return None;
        }
        let rect = (unsafe { element.CachedBoundingRectangle().ok() })
            .and_then(|rect| valid_rect(rect.left, rect.top, rect.right, rect.bottom))
            .and_then(|rect| rect.intersection(window_bounds))?;
        let class_name = cached_string(unsafe { element.CachedClassName().ok() });
        if class_name.as_deref().is_some_and(is_auxiliary_shadow_class) {
            return None;
        }
        let candidate = AutomationCandidate {
            rect,
            parent,
            depth,
            control_type: unsafe { element.CachedControlType().ok() }.map_or(0, |value| value.0),
            name: cached_string(unsafe { element.CachedName().ok() }),
            automation_id: cached_string(unsafe { element.CachedAutomationId().ok() }),
            class_name,
            framework_id: cached_string(unsafe { element.CachedFrameworkId().ok() }),
            is_control: unsafe { element.CachedIsControlElement().ok() }
                .is_some_and(|value| value.as_bool()),
            is_content: unsafe { element.CachedIsContentElement().ok() }
                .is_some_and(|value| value.as_bool()),
            peer_count: 1,
        };
        let index = snapshot.nodes.len();
        snapshot.nodes.push(candidate);
        Some(index)
    }

    fn cached_string(value: Option<BSTR>) -> Option<String> {
        let value = String::try_from(value?).ok()?;
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        Some(value.chars().take(MAX_SEMANTIC_TEXT_CHARS).collect())
    }

    fn populate_peer_counts(snapshot: &mut AutomationSnapshot) {
        let mut counts = HashMap::<(Option<usize>, i32), u16>::new();
        for node in &snapshot.nodes {
            let count = counts.entry((node.parent, node.control_type)).or_default();
            *count = count.saturating_add(1);
        }
        for node in &mut snapshot.nodes {
            node.peer_count = counts
                .get(&(node.parent, node.control_type))
                .copied()
                .unwrap_or(1);
        }
    }

    fn best_cached_candidate(
        window: &WindowCandidate,
        uia_snapshot: Option<&AutomationSnapshot>,
        point: PhysicalPoint,
    ) -> Option<PhysicalRect> {
        let native_fallback = window
            .regions
            .iter()
            .copied()
            .filter(|rect| contains(*rect, point))
            .min_by_key(rect_area);
        let automation = uia_snapshot.and_then(|snapshot| {
            best_automation_candidate(snapshot, window.descriptor.bounds, point)
        });
        automation.or(native_fallback)
    }

    fn best_automation_candidate(
        snapshot: &AutomationSnapshot,
        window_bounds: PhysicalRect,
        point: PhysicalPoint,
    ) -> Option<PhysicalRect> {
        snapshot
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                let rect = automation_candidate_rect(snapshot, index)?;
                if !contains(rect, point) {
                    return None;
                }
                let score = automation_candidate_score(snapshot, index, window_bounds);
                (score >= MIN_MEANINGFUL_AUTOMATION_SCORE).then_some((
                    score,
                    node.depth,
                    std::cmp::Reverse(rect_area(&rect)),
                    rect,
                ))
            })
            .max_by_key(|(score, depth, area, _)| (*score, *depth, *area))
            .map(|(_, _, _, rect)| rect)
    }

    fn automation_candidate_score(
        snapshot: &AutomationSnapshot,
        index: usize,
        window_bounds: PhysicalRect,
    ) -> i32 {
        let node = &snapshot.nodes[index];
        let has_name = node.name.is_some();
        let has_stable_id = node.automation_id.is_some();
        let mut score = control_type_base_score(node.control_type, has_name, has_stable_id);

        score += i32::from(node.depth.min(24)) * 4;
        if has_name {
            score += 120;
        }
        if has_stable_id {
            score += 55;
        }
        if node.class_name.is_some() {
            score += 15;
        }
        if node.is_control {
            score += 20;
        }
        if node.is_content {
            score += 25;
        }
        if is_known_chat_surface(node) {
            score += CHAT_SURFACE_SCORE_BONUS;
        }
        if !is_actionable_control_type(node.control_type)
            && has_known_chat_surface_ancestor(snapshot, index)
        {
            score -= CHAT_SURFACE_SCORE_BONUS;
        }

        if node.control_type == UIA_TextControlTypeId.0 {
            score -= 300;
            if node.peer_count >= 3 {
                score -= 120;
            }
            if node.rect.height().is_some_and(|height| height <= 40) {
                score -= 70;
            }
        }

        if node.control_type == UIA_GroupControlTypeId.0 && !has_name && !has_stable_id {
            score -= 80;
        }

        if let Some(parent) = node.parent.and_then(|parent| snapshot.nodes.get(parent)) {
            if parent.rect == node.rect {
                score -= 90;
            }
        }

        let candidate_area = rect_area(&node.rect);
        let window_area = rect_area(&window_bounds).max(1);
        if candidate_area.saturating_mul(100) >= window_area.saturating_mul(90)
            && node.control_type != UIA_WindowControlTypeId.0
        {
            score -= 180;
        }
        if candidate_area.saturating_mul(10_000) < window_area {
            score -= 100;
        }

        score + provider_profile_adjustment(snapshot, index)
    }

    fn control_type_base_score(control_type: i32, has_name: bool, has_stable_id: bool) -> i32 {
        if is_actionable_control_type(control_type) {
            return 850;
        }
        if matches!(
            control_type,
            value
                if value == UIA_ListItemControlTypeId.0
                    || value == UIA_DataItemControlTypeId.0
                    || value == UIA_TreeItemControlTypeId.0
        ) {
            return 720;
        }
        if control_type == UIA_GroupControlTypeId.0 {
            return 500;
        }
        if control_type == UIA_CustomControlTypeId.0 {
            return if has_name || has_stable_id { 500 } else { 280 };
        }
        if control_type == UIA_DocumentControlTypeId.0 {
            return 460;
        }
        if matches!(
            control_type,
            value
                if value == UIA_ListControlTypeId.0
                    || value == UIA_TreeControlTypeId.0
                    || value == UIA_TableControlTypeId.0
                    || value == UIA_DataGridControlTypeId.0
        ) {
            return 430;
        }
        if control_type == UIA_PaneControlTypeId.0 {
            return 360;
        }
        if control_type == UIA_TextControlTypeId.0 {
            return 350;
        }
        if control_type == UIA_WindowControlTypeId.0 {
            return 100;
        }
        220
    }

    fn is_actionable_control_type(control_type: i32) -> bool {
        matches!(
            control_type,
            value
                if value == UIA_ButtonControlTypeId.0
                    || value == UIA_CheckBoxControlTypeId.0
                    || value == UIA_ComboBoxControlTypeId.0
                    || value == UIA_EditControlTypeId.0
                    || value == UIA_HyperlinkControlTypeId.0
                    || value == UIA_MenuItemControlTypeId.0
                    || value == UIA_RadioButtonControlTypeId.0
                    || value == UIA_SliderControlTypeId.0
                    || value == UIA_SpinnerControlTypeId.0
                    || value == UIA_SplitButtonControlTypeId.0
                    || value == UIA_TabItemControlTypeId.0
        )
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum ProviderProfile {
        Chromium,
        Qt,
        Win32,
        Other,
    }

    fn provider_profile_adjustment(snapshot: &AutomationSnapshot, index: usize) -> i32 {
        let node = &snapshot.nodes[index];
        match provider_profile(snapshot, index) {
            ProviderProfile::Chromium => {
                if node.control_type == UIA_TextControlTypeId.0 {
                    -100
                } else if node.control_type == UIA_GroupControlTypeId.0
                    && (node.name.is_some() || node.automation_id.is_some())
                {
                    90
                } else if node.control_type == UIA_DocumentControlTypeId.0 {
                    60
                } else {
                    0
                }
            }
            ProviderProfile::Qt => {
                if node.control_type == UIA_TextControlTypeId.0 {
                    -60
                } else if node.control_type == UIA_CustomControlTypeId.0
                    && (node.name.is_some() || node.automation_id.is_some())
                {
                    50
                } else {
                    0
                }
            }
            ProviderProfile::Win32 | ProviderProfile::Other => 0,
        }
    }

    fn provider_profile(snapshot: &AutomationSnapshot, mut index: usize) -> ProviderProfile {
        loop {
            let node = &snapshot.nodes[index];
            if let Some(framework) = node.framework_id.as_deref() {
                if contains_ascii_case_insensitive(framework, "chrome")
                    || contains_ascii_case_insensitive(framework, "chromium")
                    || contains_ascii_case_insensitive(framework, "cef")
                {
                    return ProviderProfile::Chromium;
                }
                if contains_ascii_case_insensitive(framework, "qt") {
                    return ProviderProfile::Qt;
                }
                if framework.eq_ignore_ascii_case("win32") {
                    return ProviderProfile::Win32;
                }
            }
            if let Some(class_name) = node.class_name.as_deref() {
                if contains_ascii_case_insensitive(class_name, "chrome")
                    || contains_ascii_case_insensitive(class_name, "chromium")
                    || contains_ascii_case_insensitive(class_name, "cef")
                {
                    return ProviderProfile::Chromium;
                }
                if contains_ascii_case_insensitive(class_name, "qt") {
                    return ProviderProfile::Qt;
                }
            }
            let Some(parent) = node.parent else {
                return ProviderProfile::Other;
            };
            index = parent;
        }
    }

    fn contains_ascii_case_insensitive(value: &str, pattern: &str) -> bool {
        value
            .as_bytes()
            .windows(pattern.len())
            .any(|window| window.eq_ignore_ascii_case(pattern.as_bytes()))
    }

    fn is_known_chat_surface(candidate: &AutomationCandidate) -> bool {
        candidate
            .automation_id
            .as_deref()
            .is_some_and(|id| id.eq_ignore_ascii_case("chat_message_page"))
            || candidate
                .class_name
                .as_deref()
                .is_some_and(|class_name| class_name.eq_ignore_ascii_case("mmui::ChatMessagePage"))
    }

    fn automation_candidate_rect(
        snapshot: &AutomationSnapshot,
        index: usize,
    ) -> Option<PhysicalRect> {
        let candidate = snapshot.nodes.get(index)?;
        if !is_known_chat_surface(candidate) {
            return Some(candidate.rect);
        }
        let input_top = snapshot
            .nodes
            .iter()
            .enumerate()
            .filter(|(descendant, node)| {
                is_descendant_of(snapshot, *descendant, index)
                    && node.class_name.as_deref().is_some_and(|class_name| {
                        class_name.eq_ignore_ascii_case("mmui::ChatInputView")
                    })
            })
            .map(|(_, node)| node.rect.top)
            .filter(|top| *top > candidate.rect.top && *top < candidate.rect.bottom)
            .min();
        let Some(bottom) = input_top else {
            return Some(candidate.rect);
        };
        valid_rect(
            candidate.rect.left,
            candidate.rect.top,
            candidate.rect.right,
            bottom,
        )
        .or(Some(candidate.rect))
    }

    fn is_descendant_of(
        snapshot: &AutomationSnapshot,
        mut descendant: usize,
        ancestor: usize,
    ) -> bool {
        for _ in 0..snapshot.nodes.len() {
            let Some(parent) = snapshot.nodes.get(descendant).and_then(|node| node.parent) else {
                return false;
            };
            if parent == ancestor {
                return true;
            }
            descendant = parent;
        }
        false
    }

    fn has_known_chat_surface_ancestor(snapshot: &AutomationSnapshot, mut index: usize) -> bool {
        for _ in 0..snapshot.nodes.len() {
            let Some(parent) = snapshot.nodes.get(index).and_then(|node| node.parent) else {
                return false;
            };
            let Some(parent_node) = snapshot.nodes.get(parent) else {
                return false;
            };
            if is_known_chat_surface(parent_node) {
                return true;
            }
            index = parent;
        }
        false
    }

    fn is_auxiliary_shadow_class(class_name: &str) -> bool {
        class_name.eq_ignore_ascii_case("popupshadow")
            || class_name.eq_ignore_ascii_case("PerryShadowWnd")
    }

    fn native_chat_reading_surface(
        class_name: &str,
        window_bounds: PhysicalRect,
        desktop: &VirtualDesktopSnapshot,
    ) -> Option<PhysicalRect> {
        if !class_name.eq_ignore_ascii_case("WeWorkWindow") {
            return None;
        }
        let frame = desktop.frames.iter().find(|frame| {
            let frame_bounds = frame.monitor.physical_bounds;
            window_bounds.left >= frame_bounds.left
                && window_bounds.top >= frame_bounds.top
                && window_bounds.right <= frame_bounds.right
                && window_bounds.bottom <= frame_bounds.bottom
        })?;
        let chat_left = spanning_vertical_edge(frame, window_bounds)?;
        let chat_bottom = spanning_horizontal_edge(frame, window_bounds, chat_left)?;
        valid_rect(
            chat_left,
            window_bounds.top,
            window_bounds.right,
            chat_bottom,
        )
    }

    fn spanning_vertical_edge(frame: &MonitorFrame, bounds: PhysicalRect) -> Option<i32> {
        let window_width = bounds.width()?;
        let search_start = bounds
            .left
            .checked_add(i32::try_from(window_width / 5).ok()?)?;
        let search_end = bounds
            .left
            .checked_add(i32::try_from(window_width / 2).ok()?)?;
        (search_start..search_end)
            .filter_map(|x| {
                let mut total = 0u64;
                let mut spanning = 0u64;
                let mut samples = 0u64;
                for y in (bounds.top..bounds.bottom).step_by(2) {
                    let difference = pixel_difference(frame, x - 1, y, x, y)?;
                    total = total.saturating_add(u64::from(difference));
                    spanning += u64::from(difference >= SPANNING_EDGE_MIN_DIFFERENCE);
                    samples += 1;
                }
                (samples > 0
                    && spanning.saturating_mul(100)
                        >= samples.saturating_mul(SPANNING_EDGE_MIN_COVERAGE_PERCENT))
                .then_some((total, Reverse(x)))
            })
            .max_by_key(|candidate| *candidate)
            .map(|(_, Reverse(x))| x)
    }

    fn spanning_horizontal_edge(
        frame: &MonitorFrame,
        bounds: PhysicalRect,
        chat_left: i32,
    ) -> Option<i32> {
        let window_height = bounds.height()?;
        let search_start = bounds
            .top
            .checked_add(i32::try_from(window_height / 2).ok()?)?;
        let search_end = bounds
            .top
            .checked_add(i32::try_from(u64::from(window_height).saturating_mul(92) / 100).ok()?)?;
        (search_start..search_end)
            .filter_map(|y| {
                let mut total = 0u64;
                let mut spanning = 0u64;
                let mut samples = 0u64;
                for x in (chat_left..bounds.right).step_by(2) {
                    let difference = pixel_difference(frame, x, y - 1, x, y)?;
                    total = total.saturating_add(u64::from(difference));
                    spanning += u64::from(difference >= SPANNING_EDGE_MIN_DIFFERENCE);
                    samples += 1;
                }
                (samples > 0
                    && spanning.saturating_mul(100)
                        >= samples.saturating_mul(SPANNING_EDGE_MIN_COVERAGE_PERCENT))
                .then_some((total, Reverse(y)))
            })
            .max_by_key(|candidate| *candidate)
            .map(|(_, Reverse(y))| y)
    }

    fn pixel_difference(
        frame: &MonitorFrame,
        first_x: i32,
        first_y: i32,
        second_x: i32,
        second_y: i32,
    ) -> Option<u32> {
        let first = frame_pixel(frame, first_x, first_y)?;
        let second = frame_pixel(frame, second_x, second_y)?;
        Some(
            u32::from(first[0].abs_diff(second[0]))
                .saturating_add(u32::from(first[1].abs_diff(second[1])))
                .saturating_add(u32::from(first[2].abs_diff(second[2])))
                / 3,
        )
    }

    fn frame_pixel(frame: &MonitorFrame, x: i32, y: i32) -> Option<[u8; 3]> {
        let bounds = frame.monitor.physical_bounds;
        if !contains(bounds, PhysicalPoint::new(x, y)) {
            return None;
        }
        let local_x = usize::try_from(x.checked_sub(bounds.left)?).ok()?;
        let local_y = usize::try_from(y.checked_sub(bounds.top)?).ok()?;
        let offset = local_y
            .checked_mul(frame.stride)?
            .checked_add(local_x.checked_mul(4)?)?;
        Some([
            *frame.bgra.get(offset)?,
            *frame.bgra.get(offset.checked_add(1)?)?,
            *frame.bgra.get(offset.checked_add(2)?)?,
        ])
    }

    fn window_class_name(window: HWND) -> Option<String> {
        let mut buffer = [0u16; MAX_WINDOW_CLASS_CHARS];
        let length = unsafe { GetClassNameW(window, buffer.as_mut_ptr(), buffer.len() as i32) };
        if length <= 0 {
            return None;
        }
        String::from_utf16(&buffer[..length as usize]).ok()
    }

    struct WindowEnumerationState<'a> {
        windows: &'a mut Vec<WindowCandidate>,
        desktop: &'a VirtualDesktopSnapshot,
    }

    fn enumerate_windows(desktop: &VirtualDesktopSnapshot) -> Vec<WindowCandidate> {
        let mut windows = Vec::new();
        let mut state = WindowEnumerationState {
            windows: &mut windows,
            desktop,
        };
        unsafe {
            let _ = EnumWindows(
                Some(enumerate_window),
                (&mut state as *mut WindowEnumerationState<'_>) as LPARAM,
            );
        }
        windows
    }

    unsafe extern "system" fn enumerate_window(window: HWND, state: LPARAM) -> BOOL {
        let state = &mut *(state as *mut WindowEnumerationState<'_>);
        if IsWindowVisible(window) == 0 || IsIconic(window) != 0 {
            return 1;
        }
        let class_name = window_class_name(window);
        if class_name.as_deref().is_some_and(is_auxiliary_shadow_class) {
            return 1;
        }
        let mut process_id = 0u32;
        let _ = GetWindowThreadProcessId(window, &mut process_id);
        if process_id == 0 {
            return 1;
        }
        let mut cloaked = 0u32;
        let cloaked_result = DwmGetWindowAttribute(
            window,
            DWMWA_CLOAKED as u32,
            (&mut cloaked as *mut u32).cast(),
            size_of::<u32>() as u32,
        );
        if cloaked_result >= 0 && cloaked != 0 {
            return 1;
        }
        let Some(bounds) = top_level_window_bounds(window) else {
            return 1;
        };

        let descriptor = WindowDescriptor {
            handle: window as isize,
            process_id,
            bounds,
        };
        let mut regions = vec![bounds];
        if let Some(client) =
            client_rect_on_screen(window).and_then(|rect| rect.intersection(bounds))
        {
            regions.push(client);
        }
        if let Some(chat_surface) = class_name
            .as_deref()
            .and_then(|class_name| native_chat_reading_surface(class_name, bounds, state.desktop))
        {
            regions.push(chat_surface);
        }
        let mut child_state = ChildEnumerationState {
            window_bounds: bounds,
            regions: &mut regions,
        };
        let _ = EnumChildWindows(
            window,
            Some(enumerate_child_window),
            (&mut child_state as *mut ChildEnumerationState) as LPARAM,
        );
        normalize_regions(&mut regions);
        state.windows.push(WindowCandidate {
            descriptor,
            regions,
        });
        1
    }

    struct ChildEnumerationState<'a> {
        window_bounds: PhysicalRect,
        regions: &'a mut Vec<PhysicalRect>,
    }

    unsafe extern "system" fn enumerate_child_window(window: HWND, state: LPARAM) -> BOOL {
        if IsWindowVisible(window) == 0 {
            return 1;
        }
        let state = &mut *(state as *mut ChildEnumerationState<'_>);
        if let Some(bounds) =
            basic_window_rect(window).and_then(|rect| rect.intersection(state.window_bounds))
        {
            state.regions.push(bounds);
        }
        if let Some(client) =
            client_rect_on_screen(window).and_then(|rect| rect.intersection(state.window_bounds))
        {
            state.regions.push(client);
        }
        1
    }

    fn top_level_window_bounds(window: HWND) -> Option<PhysicalRect> {
        let mut rect: RECT = unsafe { zeroed() };
        let extended_result = unsafe {
            DwmGetWindowAttribute(
                window,
                DWMWA_EXTENDED_FRAME_BOUNDS as u32,
                (&mut rect as *mut RECT).cast(),
                size_of::<RECT>() as u32,
            )
        };
        if extended_result < 0 && unsafe { GetWindowRect(window, &mut rect) } == 0 {
            return None;
        }
        valid_rect(rect.left, rect.top, rect.right, rect.bottom)
    }

    fn basic_window_rect(window: HWND) -> Option<PhysicalRect> {
        let mut rect: RECT = unsafe { zeroed() };
        if unsafe { GetWindowRect(window, &mut rect) } == 0 {
            return None;
        }
        valid_rect(rect.left, rect.top, rect.right, rect.bottom)
    }

    fn client_rect_on_screen(window: HWND) -> Option<PhysicalRect> {
        let mut rect: RECT = unsafe { zeroed() };
        if unsafe { GetClientRect(window, &mut rect) } == 0 {
            return None;
        }
        let mut top_left = POINT {
            x: rect.left,
            y: rect.top,
        };
        let mut bottom_right = POINT {
            x: rect.right,
            y: rect.bottom,
        };
        if unsafe { ClientToScreen(window, &mut top_left) } == 0
            || unsafe { ClientToScreen(window, &mut bottom_right) } == 0
        {
            return None;
        }
        valid_rect(top_left.x, top_left.y, bottom_right.x, bottom_right.y)
    }

    fn normalize_regions(regions: &mut Vec<PhysicalRect>) {
        regions.sort_unstable_by_key(|rect| {
            (
                rect_area(rect),
                rect.left,
                rect.top,
                rect.right,
                rect.bottom,
            )
        });
        regions.dedup();
    }

    fn valid_rect(left: i32, top: i32, right: i32, bottom: i32) -> Option<PhysicalRect> {
        let rect = PhysicalRect::new(left, top, right, bottom).ok()?;
        (rect.width()? >= MIN_CANDIDATE_EDGE && rect.height()? >= MIN_CANDIDATE_EDGE)
            .then_some(rect)
    }

    fn contains(rect: PhysicalRect, point: PhysicalPoint) -> bool {
        point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
    }

    fn rect_area(rect: &PhysicalRect) -> u64 {
        u64::from(rect.width().unwrap_or(u32::MAX))
            .saturating_mul(u64::from(rect.height().unwrap_or(u32::MAX)))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn descriptor(handle: isize, bounds: PhysicalRect) -> WindowDescriptor {
            WindowDescriptor {
                handle,
                process_id: handle as u32,
                bounds,
            }
        }

        fn automation_candidate(
            rect: PhysicalRect,
            parent: Option<usize>,
            depth: u16,
            control_type: i32,
            name: Option<&str>,
            framework_id: Option<&str>,
        ) -> AutomationCandidate {
            AutomationCandidate {
                rect,
                parent,
                depth,
                control_type,
                name: name.map(str::to_owned),
                automation_id: None,
                class_name: None,
                framework_id: framework_id.map(str::to_owned),
                is_control: true,
                is_content: true,
                peer_count: 1,
            }
        }

        fn chat_layout_snapshot() -> VirtualDesktopSnapshot {
            use crate::infrastructure::screenshot::model::{DisplayRotation, MonitorDescriptor};

            let bounds = PhysicalRect::new(0, 0, 100, 80).unwrap();
            let width = 100u32;
            let height = 80u32;
            let stride = usize::try_from(width).unwrap() * 4;
            let mut bgra = vec![0u8; stride * usize::try_from(height).unwrap()];
            for y in 0..height {
                for x in 0..width {
                    let value = if x < 30 {
                        50
                    } else if y < 60 {
                        200
                    } else {
                        220
                    };
                    let offset =
                        usize::try_from(y).unwrap() * stride + usize::try_from(x).unwrap() * 4;
                    bgra[offset] = value;
                    bgra[offset + 1] = value;
                    bgra[offset + 2] = value;
                    bgra[offset + 3] = 255;
                }
            }
            VirtualDesktopSnapshot::new(
                1,
                0,
                vec![MonitorFrame {
                    monitor: MonitorDescriptor {
                        id: "primary".to_owned(),
                        physical_bounds: bounds,
                        work_bounds: bounds,
                        dpi_x: 96,
                        dpi_y: 96,
                        rotation: DisplayRotation::Identity,
                        is_primary: true,
                    },
                    width,
                    height,
                    stride,
                    bgra,
                }],
                Vec::new(),
            )
            .unwrap()
        }

        #[test]
        fn candidate_rect_rejects_empty_and_tiny_artifacts() {
            assert!(valid_rect(0, 0, 2, 200).is_none());
            assert!(valid_rect(0, 0, 200, 2).is_none());
            assert_eq!(
                valid_rect(-20, 10, 80, 40),
                Some(PhysicalRect::new(-20, 10, 80, 40).unwrap())
            );
        }

        #[test]
        fn latest_request_slot_replaces_queued_work() {
            let slot = LatestRequestSlot::default();
            let (response_sender, _response_receiver) = mpsc::channel();
            let first = CandidateRequest {
                session_id: 7,
                generation: 1,
                window: descriptor(10, PhysicalRect::new(0, 0, 100, 100).unwrap()),
                response_sender: response_sender.clone(),
            };
            let latest = CandidateRequest {
                session_id: 7,
                generation: 2,
                window: descriptor(20, PhysicalRect::new(100, 0, 200, 100).unwrap()),
                response_sender,
            };

            assert!(slot.submit(first).is_none());
            let replaced = slot.submit(latest).unwrap();
            assert_eq!(replaced.generation, 1);
            assert_eq!(replaced.window.handle, 10);
            let received = slot.receive().unwrap();
            assert_eq!(received.generation, 2);
            assert_eq!(received.window.handle, 20);
        }

        #[test]
        fn semantic_hit_test_prefers_named_message_group_over_text_line() {
            let window_bounds = PhysicalRect::new(0, 0, 1200, 800).unwrap();
            let client = PhysicalRect::new(8, 32, 1192, 792).unwrap();
            let document = PhysicalRect::new(100, 100, 900, 700).unwrap();
            let message = PhysicalRect::new(120, 140, 700, 280).unwrap();
            let text_line = PhysicalRect::new(140, 180, 640, 214).unwrap();
            let window = WindowCandidate {
                descriptor: descriptor(10, window_bounds),
                regions: vec![window_bounds, client],
            };
            let mut snapshot = AutomationSnapshot {
                nodes: vec![
                    automation_candidate(
                        document,
                        None,
                        1,
                        UIA_DocumentControlTypeId.0,
                        Some("Codex"),
                        Some("Chrome"),
                    ),
                    automation_candidate(
                        message,
                        Some(0),
                        2,
                        UIA_GroupControlTypeId.0,
                        Some("完整消息"),
                        None,
                    ),
                    automation_candidate(
                        text_line,
                        Some(1),
                        3,
                        UIA_TextControlTypeId.0,
                        Some("一行文字"),
                        None,
                    ),
                ],
            };
            populate_peer_counts(&mut snapshot);

            assert_eq!(
                best_cached_candidate(&window, Some(&snapshot), PhysicalPoint::new(200, 190)),
                Some(message)
            );
            assert_eq!(
                best_cached_candidate(&window, None, PhysicalPoint::new(80, 80)),
                Some(client)
            );
        }

        #[test]
        fn semantic_hit_test_keeps_actionable_controls_precise() {
            let window_bounds = PhysicalRect::new(0, 0, 900, 700).unwrap();
            let panel = PhysicalRect::new(100, 100, 800, 600).unwrap();
            let button = PhysicalRect::new(640, 520, 760, 566).unwrap();
            let window = WindowCandidate {
                descriptor: descriptor(10, window_bounds),
                regions: vec![window_bounds],
            };
            let snapshot = AutomationSnapshot {
                nodes: vec![
                    automation_candidate(
                        panel,
                        None,
                        1,
                        UIA_GroupControlTypeId.0,
                        Some("操作区"),
                        Some("Chrome"),
                    ),
                    automation_candidate(
                        button,
                        Some(0),
                        2,
                        UIA_ButtonControlTypeId.0,
                        Some("发送"),
                        None,
                    ),
                ],
            };

            assert_eq!(
                best_cached_candidate(&window, Some(&snapshot), PhysicalPoint::new(700, 540)),
                Some(button)
            );
        }

        #[test]
        fn weixin_chat_reading_surface_excludes_input_and_beats_message_rows() {
            let window_bounds = PhysicalRect::new(500, 90, 1610, 930).unwrap();
            let chat_page = PhysicalRect::new(858, 126, 1610, 929).unwrap();
            let chat_reading_surface = PhysicalRect::new(858, 126, 1610, 686).unwrap();
            let message_list = PhysicalRect::new(858, 174, 1610, 686).unwrap();
            let message_row = PhysicalRect::new(872, 230, 1530, 370).unwrap();
            let button = PhysicalRect::new(1510, 130, 1580, 166).unwrap();
            let input_rect = PhysicalRect::new(858, 686, 1610, 929).unwrap();
            let mut page = automation_candidate(
                chat_page,
                None,
                8,
                UIA_GroupControlTypeId.0,
                None,
                Some("Qt"),
            );
            page.automation_id = Some("chat_message_page".to_owned());
            page.class_name = Some("mmui::ChatMessagePage".to_owned());
            let mut input = automation_candidate(
                input_rect,
                Some(0),
                15,
                UIA_GroupControlTypeId.0,
                None,
                None,
            );
            input.class_name = Some("mmui::ChatInputView".to_owned());
            let snapshot = AutomationSnapshot {
                nodes: vec![
                    page,
                    automation_candidate(
                        message_list,
                        Some(0),
                        15,
                        UIA_ListControlTypeId.0,
                        Some("消息"),
                        None,
                    ),
                    automation_candidate(
                        message_row,
                        Some(1),
                        16,
                        UIA_ListItemControlTypeId.0,
                        Some("国志 图片消息"),
                        None,
                    ),
                    automation_candidate(
                        button,
                        Some(0),
                        15,
                        UIA_ButtonControlTypeId.0,
                        Some("更多"),
                        None,
                    ),
                    input,
                ],
            };

            assert_eq!(
                automation_candidate_rect(&snapshot, 0),
                Some(chat_reading_surface)
            );
            assert_eq!(
                best_automation_candidate(&snapshot, window_bounds, PhysicalPoint::new(1_000, 300)),
                Some(chat_reading_surface)
            );
            assert_eq!(
                best_automation_candidate(&snapshot, window_bounds, PhysicalPoint::new(1_540, 145)),
                Some(button)
            );
            assert_ne!(
                best_automation_candidate(&snapshot, window_bounds, PhysicalPoint::new(1_000, 800)),
                Some(chat_reading_surface)
            );
        }

        #[test]
        fn repeated_presentation_text_does_not_beat_its_container() {
            let window_bounds = PhysicalRect::new(0, 0, 900, 700).unwrap();
            let row = PhysicalRect::new(60, 120, 360, 210).unwrap();
            let first_line = PhysicalRect::new(90, 138, 300, 162).unwrap();
            let second_line = PhysicalRect::new(90, 170, 330, 194).unwrap();
            let mut snapshot = AutomationSnapshot {
                nodes: vec![
                    automation_candidate(row, None, 1, UIA_GroupControlTypeId.0, None, Some("Qt")),
                    automation_candidate(
                        first_line,
                        Some(0),
                        2,
                        UIA_TextControlTypeId.0,
                        Some("联系人"),
                        None,
                    ),
                    automation_candidate(
                        second_line,
                        Some(0),
                        2,
                        UIA_TextControlTypeId.0,
                        Some("最近一条消息"),
                        None,
                    ),
                    automation_candidate(
                        PhysicalRect::new(90, 196, 220, 220).unwrap(),
                        Some(0),
                        2,
                        UIA_TextControlTypeId.0,
                        Some("时间"),
                        None,
                    ),
                ],
            };
            populate_peer_counts(&mut snapshot);

            assert_eq!(snapshot.nodes[1].peer_count, 3);
            assert_eq!(
                best_automation_candidate(&snapshot, window_bounds, PhysicalPoint::new(120, 150)),
                Some(row)
            );
        }

        #[test]
        fn provider_profile_is_inherited_from_ancestors() {
            let snapshot = AutomationSnapshot {
                nodes: vec![
                    automation_candidate(
                        PhysicalRect::new(0, 0, 800, 600).unwrap(),
                        None,
                        0,
                        UIA_DocumentControlTypeId.0,
                        Some("Codex"),
                        Some("Chrome"),
                    ),
                    automation_candidate(
                        PhysicalRect::new(100, 100, 500, 300).unwrap(),
                        Some(0),
                        1,
                        UIA_GroupControlTypeId.0,
                        Some("消息"),
                        None,
                    ),
                ],
            };

            assert_eq!(provider_profile(&snapshot, 1), ProviderProfile::Chromium);
        }

        #[test]
        fn auxiliary_shadow_window_classes_are_filtered_case_insensitively() {
            assert!(is_auxiliary_shadow_class("popupshadow"));
            assert!(is_auxiliary_shadow_class("PopupShadow"));
            assert!(is_auxiliary_shadow_class("PerryShadowWnd"));
            assert!(!is_auxiliary_shadow_class("WeChatMainWndForPC"));
        }

        #[test]
        fn native_chat_layout_is_detected_for_enterprise_wechat_but_not_classic_wechat() {
            let desktop = chat_layout_snapshot();
            let window = PhysicalRect::new(0, 0, 100, 80).unwrap();
            let reading_surface = PhysicalRect::new(30, 0, 100, 60).unwrap();
            assert_eq!(
                best_cached_candidate(
                    &WindowCandidate {
                        descriptor: descriptor(10, window),
                        regions: vec![window],
                    },
                    None,
                    PhysicalPoint::new(50, 30),
                ),
                Some(window)
            );
            assert!(native_chat_reading_surface("WeChatMainWndForPC", window, &desktop).is_none());
            assert_eq!(
                native_chat_reading_surface("WeWorkWindow", window, &desktop),
                Some(reading_surface)
            );
            assert!(native_chat_reading_surface("Chrome_WidgetWin_1", window, &desktop).is_none());
        }

        #[test]
        fn normalized_regions_are_area_sorted_and_deduplicated() {
            let large = PhysicalRect::new(0, 0, 100, 100).unwrap();
            let small = PhysicalRect::new(10, 10, 30, 30).unwrap();
            let mut regions = vec![large, small, small];

            normalize_regions(&mut regions);

            assert_eq!(regions, vec![small, large]);
        }
    }
}
