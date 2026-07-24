use super::model::{PhysicalPoint, PhysicalRect};

#[cfg(windows)]
pub use windows_impl::CaptureCandidateDetector;

#[cfg(not(windows))]
pub struct CaptureCandidateDetector;

#[cfg(not(windows))]
impl CaptureCandidateDetector {
    pub fn snapshot() -> Self {
        Self
    }

    pub fn candidate_at(&mut self, _point: PhysicalPoint) -> Option<PhysicalRect> {
        None
    }
}

#[cfg(windows)]
mod windows_impl {
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::sync::mpsc::{self, Receiver, SyncSender};
    use std::thread;

    use windows::Win32::Foundation::HWND as AutomationHwnd;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTreeWalker,
    };
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows_sys::Win32::Graphics::Dwm::{
        DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
    };

    use super::{PhysicalPoint, PhysicalRect};

    // Chromium/WebView accessibility trees can contain hundreds of siblings
    // before the element under the pointer. Keep the work bounded, but do not
    // give up so early that every rich application degrades to its top-level
    // window.
    const MAX_AUTOMATION_DEPTH: usize = 24;
    const MAX_AUTOMATION_NODES: usize = 4_096;
    const MIN_CANDIDATE_EDGE: u32 = 3;

    #[derive(Clone, Copy)]
    struct WindowCandidate {
        handle: isize,
        bounds: PhysicalRect,
    }

    #[derive(Clone, Copy)]
    struct CandidateRequest {
        point: PhysicalPoint,
        window: WindowCandidate,
    }

    #[derive(Clone, Copy)]
    struct CandidateResponse {
        point: PhysicalPoint,
        window_handle: isize,
        bounds: Option<PhysicalRect>,
    }

    pub struct CaptureCandidateDetector {
        windows: Vec<WindowCandidate>,
        request_sender: Option<SyncSender<CandidateRequest>>,
        response_receiver: Receiver<CandidateResponse>,
        last_response: Option<CandidateResponse>,
    }

    impl CaptureCandidateDetector {
        pub fn snapshot() -> Self {
            let windows = enumerate_windows();
            let (request_sender, request_receiver) = mpsc::sync_channel(1);
            let (response_sender, response_receiver) = mpsc::channel();
            let _ = thread::Builder::new()
                .name("screenshot-uia-candidate".to_owned())
                .spawn(move || automation_worker(request_receiver, response_sender));
            Self {
                windows,
                request_sender: Some(request_sender),
                response_receiver,
                last_response: None,
            }
        }

        pub fn candidate_at(&mut self, point: PhysicalPoint) -> Option<PhysicalRect> {
            while let Ok(response) = self.response_receiver.try_recv() {
                self.last_response = Some(response);
            }
            let window = *self
                .windows
                .iter()
                .find(|window| contains(window.bounds, point))?;
            if let Some(sender) = self.request_sender.as_ref() {
                let _ = sender.try_send(CandidateRequest { point, window });
            }
            // Do not paint the top-level window while the first UIA lookup is
            // still pending. A maximized application otherwise looks like an
            // already selected full-screen capture and hides the later,
            // precise element transition. Once UIA has answered, a provider
            // that exposes no child still falls back honestly to the window.
            self.last_response
                .filter(|response| {
                    response.window_handle == window.handle && distance(response.point, point) <= 80
                })
                .and_then(|response| match response.bounds {
                    Some(bounds) if contains(bounds, point) => Some(bounds),
                    Some(_) => None,
                    None => Some(window.bounds),
                })
        }
    }

    impl Drop for CaptureCandidateDetector {
        fn drop(&mut self) {
            // Do not join here: a third-party UIA provider can be slow. The
            // detached worker exits when healthy and cannot block Esc/close.
            self.request_sender.take();
        }
    }

    fn automation_worker(
        request_receiver: Receiver<CandidateRequest>,
        response_sender: mpsc::Sender<CandidateResponse>,
    ) {
        // This worker has no Windows message loop. Microsoft requires a
        // background UI Automation client like this to use an MTA; an STA can
        // stall while COM waits for messages that this thread never pumps.
        let com_result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let should_uninitialize = com_result.is_ok();
        let automation = unsafe {
            CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
        }
        .ok();
        let control_walker = automation
            .as_ref()
            .and_then(|automation| unsafe { automation.ControlViewWalker().ok() });
        let content_walker = automation
            .as_ref()
            .and_then(|automation| unsafe { automation.ContentViewWalker().ok() });

        while let Ok(mut request) = request_receiver.recv() {
            while let Ok(newer) = request_receiver.try_recv() {
                request = newer;
            }
            let bounds = automation.as_ref().and_then(|automation| {
                most_specific_rect([
                    control_walker
                        .as_ref()
                        .and_then(|walker| descendant_automation_rect(automation, walker, request)),
                    content_walker
                        .as_ref()
                        .and_then(|walker| descendant_automation_rect(automation, walker, request)),
                ])
            });
            if response_sender
                .send(CandidateResponse {
                    point: request.point,
                    window_handle: request.window.handle,
                    bounds,
                })
                .is_err()
            {
                break;
            }
        }
        drop(content_walker);
        drop(control_walker);
        drop(automation);
        if should_uninitialize {
            unsafe {
                CoUninitialize();
            }
        }
    }

    fn descendant_automation_rect(
        automation: &IUIAutomation,
        walker: &IUIAutomationTreeWalker,
        request: CandidateRequest,
    ) -> Option<PhysicalRect> {
        let root = unsafe {
            automation
                .ElementFromHandle(AutomationHwnd(request.window.handle as *mut c_void))
                .ok()?
        };
        let mut best = None;
        let mut visited = 0usize;
        let mut stack = vec![(root, 0usize)];
        while let Some((parent, depth)) = stack.pop() {
            if depth >= MAX_AUTOMATION_DEPTH || visited >= MAX_AUTOMATION_NODES {
                continue;
            }
            let mut child = unsafe { walker.GetFirstChildElement(&parent).ok() };
            while let Some(element) = child {
                if visited >= MAX_AUTOMATION_NODES {
                    break;
                }
                visited += 1;
                let next = unsafe { walker.GetNextSiblingElement(&element).ok() };
                if let Some(bounds) = automation_rect(&element)
                    .filter(|rect| contains(*rect, request.point))
                    .and_then(|rect| rect.intersection(request.window.bounds))
                {
                    if bounds != request.window.bounds
                        && best
                            .as_ref()
                            .is_none_or(|current| rect_area(&bounds) < rect_area(current))
                    {
                        best = Some(bounds);
                    }
                    // Several UIA siblings can overlap the same point (common
                    // in Chromium/WebView). Explore every matching branch
                    // instead of committing to the first container, while
                    // pruning every subtree that cannot contain the pointer.
                    stack.push((element, depth + 1));
                }
                child = next;
            }
        }
        best
    }

    fn automation_rect(element: &IUIAutomationElement) -> Option<PhysicalRect> {
        if unsafe { element.CurrentIsOffscreen().ok()?.as_bool() } {
            return None;
        }
        let rect = unsafe { element.CurrentBoundingRectangle().ok()? };
        valid_rect(rect.left, rect.top, rect.right, rect.bottom)
    }

    fn most_specific_rect<const N: usize>(
        candidates: [Option<PhysicalRect>; N],
    ) -> Option<PhysicalRect> {
        candidates.into_iter().flatten().min_by_key(rect_area)
    }

    fn rect_area(rect: &PhysicalRect) -> u64 {
        u64::from(rect.width().unwrap_or(u32::MAX))
            .saturating_mul(u64::from(rect.height().unwrap_or(u32::MAX)))
    }

    fn enumerate_windows() -> Vec<WindowCandidate> {
        let mut windows = Vec::new();
        unsafe {
            let _ = EnumWindows(
                Some(enumerate_window),
                (&mut windows as *mut Vec<WindowCandidate>) as LPARAM,
            );
        }
        windows
    }

    unsafe extern "system" fn enumerate_window(window: HWND, state: LPARAM) -> BOOL {
        let windows = &mut *(state as *mut Vec<WindowCandidate>);
        if IsWindowVisible(window) == 0 || IsIconic(window) != 0 {
            return 1;
        }
        let mut process_id = 0u32;
        let _ = GetWindowThreadProcessId(window, &mut process_id);
        // The overlay windows do not exist when this snapshot is taken, so
        // the application's own visible window is a valid target. Excluding
        // the entire process exposed the desktop underneath as a full-screen
        // candidate whenever F1 was pressed over OpenDeskTools itself.
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
        let mut rect: RECT = zeroed();
        let extended_result = DwmGetWindowAttribute(
            window,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            (&mut rect as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        );
        if extended_result < 0 && GetWindowRect(window, &mut rect) == 0 {
            return 1;
        }
        if let Some(bounds) = valid_rect(rect.left, rect.top, rect.right, rect.bottom) {
            windows.push(WindowCandidate {
                handle: window as isize,
                bounds,
            });
        }
        1
    }

    fn valid_rect(left: i32, top: i32, right: i32, bottom: i32) -> Option<PhysicalRect> {
        let rect = PhysicalRect::new(left, top, right, bottom).ok()?;
        (rect.width()? >= MIN_CANDIDATE_EDGE && rect.height()? >= MIN_CANDIDATE_EDGE)
            .then_some(rect)
    }

    fn contains(rect: PhysicalRect, point: PhysicalPoint) -> bool {
        point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
    }

    fn distance(first: PhysicalPoint, second: PhysicalPoint) -> i32 {
        (first.x - second.x)
            .abs()
            .saturating_add((first.y - second.y).abs())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn candidate_rect_rejects_empty_and_tiny_accessibility_artifacts() {
            assert!(valid_rect(0, 0, 2, 200).is_none());
            assert!(valid_rect(0, 0, 200, 2).is_none());
            assert_eq!(
                valid_rect(-20, 10, 80, 40),
                Some(PhysicalRect::new(-20, 10, 80, 40).unwrap())
            );
        }

        #[test]
        fn stale_element_response_is_not_reused_far_from_its_probe_point() {
            assert!(distance(PhysicalPoint::new(10, 20), PhysicalPoint::new(40, 50)) <= 80);
            assert!(distance(PhysicalPoint::new(10, 20), PhysicalPoint::new(100, 200)) > 80);
        }

        #[test]
        fn control_and_content_candidates_choose_the_most_specific_region() {
            let window = PhysicalRect::new(0, 0, 1200, 800).unwrap();
            let panel = PhysicalRect::new(100, 100, 900, 700).unwrap();
            let text_line = PhysicalRect::new(140, 180, 640, 214).unwrap();

            assert_eq!(
                most_specific_rect([Some(panel), Some(text_line)]),
                Some(text_line)
            );
            assert_eq!(most_specific_rect([None, Some(window)]), Some(window));
        }
    }
}
