use std::sync::Arc;

use super::annotation::Annotation;
use super::model::{PhysicalRect, VirtualDesktopSnapshot};
use super::ScreenshotError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureAction {
    Copy,
    Save,
    DecodeQr,
    Pin,
    Finish,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureSelection {
    pub rect: PhysicalRect,
    pub action: CaptureAction,
    pub annotations: Vec<Annotation>,
}

#[cfg(windows)]
pub fn probe() -> Result<(), ScreenshotError> {
    windows_impl::OverlayWindowClass::register().map(|_| ())
}

#[cfg(not(windows))]
pub fn probe() -> Result<(), ScreenshotError> {
    Err(ScreenshotError::UnsupportedPlatform)
}

#[cfg(windows)]
pub fn select(
    snapshot: Arc<VirtualDesktopSnapshot>,
) -> Result<Option<CaptureSelection>, ScreenshotError> {
    windows_impl::select(snapshot)
}

#[cfg(not(windows))]
pub fn select(
    _snapshot: Arc<VirtualDesktopSnapshot>,
) -> Result<Option<CaptureSelection>, ScreenshotError> {
    Err(ScreenshotError::UnsupportedPlatform)
}

#[cfg(windows)]
mod windows_impl {
    use std::cell::RefCell;
    use std::mem::{size_of, zeroed};
    use std::ptr::{null, null_mut};
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{
        COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
    };
    use windows_sys::Win32::Graphics::Gdi::{
        BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreatePen,
        CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, FrameRect,
        GetStockObject, IntersectClipRect, InvalidateRect, LineTo, MoveToEx, RestoreDC, SaveDC,
        SelectObject, SetBkMode, SetTextColor, StretchDIBits, UpdateWindow, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET,
        DEFAULT_PITCH, DIB_RGB_COLORS, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
        FF_DONTCARE, FW_NORMAL, HDC, HGDIOBJ, HOLLOW_BRUSH, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
        PS_SOLID, SRCCOPY, TRANSPARENT,
    };
    use windows_sys::Win32::Graphics::GdiPlus::{
        FillModeWinding, GdipAddPathArcI, GdipClosePathFigure, GdipCreateFromHDC, GdipCreatePath,
        GdipCreatePen1, GdipCreateSolidFill, GdipDeleteBrush, GdipDeleteGraphics, GdipDeletePath,
        GdipDeletePen, GdipDrawEllipseI, GdipDrawLineI, GdipDrawPath, GdipDrawRectangleI,
        GdipFillEllipseI, GdipFillPath, GdipSetSmoothingMode, GdiplusShutdown, GdiplusStartup,
        GdiplusStartupInput, GpBrush, GpGraphics, GpPath, GpPen, GpSolidFill,
        SmoothingModeAntiAlias8x8, UnitPixel,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Controls::EM_SETCUEBANNER;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_LEFT,
        VK_RETURN, VK_RIGHT, VK_SHIFT, VK_UP,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
        GetForegroundWindow, GetMessageW, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW,
        IsWindow, KillTimer, LoadCursorW, PostQuitMessage, RegisterClassW, SendMessageW,
        SetForegroundWindow, SetTimer, SetWindowLongPtrW, ShowWindow, TranslateMessage,
        UnregisterClassW, CREATESTRUCTW, CS_DBLCLKS, ES_AUTOHSCROLL, GWLP_USERDATA, IDC_CROSS, MSG,
        SW_SHOW, WM_ACTIVATEAPP, WM_CAPTURECHANGED, WM_CLOSE, WM_CTLCOLOREDIT, WM_DISPLAYCHANGE,
        WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
        WM_NCCREATE, WM_PAINT, WM_RBUTTONDOWN, WM_SETFONT, WM_TIMER, WNDCLASSW, WS_CHILD,
        WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP, WS_VISIBLE,
    };

    use super::*;
    use crate::infrastructure::screenshot::annotation::{
        Annotation, AnnotationStyle, AnnotationTool,
    };
    use crate::infrastructure::screenshot::candidate::CaptureCandidateDetector;
    use crate::infrastructure::screenshot::model::{MonitorFrame, PhysicalPoint, PhysicalRect};
    use crate::infrastructure::screenshot::selection::{
        selection_size, SelectionHandle, SelectionOutcome, SelectionState,
    };

    const CLASS_NAME: &[u16] = &[
        b'O' as u16,
        b'p' as u16,
        b'e' as u16,
        b'n' as u16,
        b'D' as u16,
        b'e' as u16,
        b's' as u16,
        b'k' as u16,
        b'T' as u16,
        b'o' as u16,
        b'o' as u16,
        b'l' as u16,
        b's' as u16,
        b'S' as u16,
        b'c' as u16,
        b'r' as u16,
        b'e' as u16,
        b'e' as u16,
        b'n' as u16,
        b's' as u16,
        b'h' as u16,
        b'o' as u16,
        b't' as u16,
        b'O' as u16,
        b'v' as u16,
        b'e' as u16,
        b'r' as u16,
        b'l' as u16,
        b'a' as u16,
        b'y' as u16,
        0,
    ];
    const OVERLAY_ALPHA: u8 = 116;
    const LABEL_HEIGHT: i32 = 24;
    const LABEL_WIDTH: i32 = 116;
    const fn colorref(red: u8, green: u8, blue: u8) -> COLORREF {
        red as COLORREF | ((green as COLORREF) << 8) | ((blue as COLORREF) << 16)
    }

    const ACCENT_COLOR: COLORREF = colorref(36, 112, 224);
    const LABEL_BACKGROUND: COLORREF = colorref(36, 42, 48);
    const TOOLBAR_BACKGROUND: COLORREF = colorref(250, 251, 252);
    const TOOLBAR_HOVER: COLORREF = colorref(235, 242, 252);
    const TOOLBAR_SELECTED: COLORREF = colorref(222, 234, 252);
    const TOOLBAR_BORDER: COLORREF = colorref(210, 216, 224);
    const TOOLBAR_DIVIDER: COLORREF = colorref(224, 228, 234);
    const TOOLBAR_ICON: COLORREF = colorref(42, 48, 56);
    const TOOLBAR_DISABLED: COLORREF = colorref(155, 163, 174);
    const TOOLBAR_SHADOW: COLORREF = colorref(205, 211, 220);
    const TOOLTIP_BACKGROUND: COLORREF = colorref(38, 43, 50);
    const WHITE: COLORREF = colorref(255, 255, 255);
    const TOOLBAR_HEIGHT: i32 = 40;
    const TOOLBAR_BUTTON_WIDTH: i32 = 34;
    const TOOLBAR_PADDING: i32 = 3;
    const TOOLBAR_GROUP_GAP: i32 = 8;
    const TOOLBAR_RADIUS: i32 = 10;
    const TOOLBAR_BUTTON_RADIUS: i32 = 7;
    const TOOLBAR_GAP: i32 = 6;
    const HANDLE_RADIUS: i32 = 4;
    const HANDLE_HIT_RADIUS: i32 = 7;
    const CANDIDATE_DRAG_THRESHOLD: i32 = 4;
    const CANDIDATE_PROBE_INTERVAL: Duration = Duration::from_millis(30);
    const CANDIDATE_TIMER_ID: usize = 1;
    const PARAMETER_HEIGHT: i32 = 36;
    const PARAMETER_GAP: i32 = 4;
    const PARAMETER_PADDING: i32 = 4;
    const COLOR_BUTTON_WIDTH: i32 = 26;
    const SIZE_BUTTON_WIDTH: i32 = 34;
    const PARAMETER_GROUP_GAP: i32 = 8;
    const ANNOTATION_COLORS: [[u8; 3]; 5] = [
        [238, 49, 49],
        [245, 155, 35],
        [33, 166, 91],
        [36, 112, 224],
        [35, 35, 35],
    ];
    const LINE_THICKNESSES: [u8; 3] = [2, 4, 8];
    const MOSAIC_BLOCKS: [u8; 3] = [6, 12, 20];
    const TEXT_FONT_SIZES: [u8; 5] = [14, 18, 24, 32, 48];
    const EDIT_CLASS: &[u16] = &[b'E' as u16, b'D' as u16, b'I' as u16, b'T' as u16, 0];

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ToolbarItem {
        Select,
        Tool(AnnotationTool),
        Undo,
        Redo,
        Action(CaptureAction),
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ToolbarIcon {
        Select,
        Rectangle,
        Arrow,
        Pen,
        Text,
        Mosaic,
        Undo,
        Redo,
        Copy,
        Save,
        Qr,
        Pin,
        Finish,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ParameterItem {
        Color(usize),
        Size(u8),
    }

    const TOOLBAR_GROUP_BREAKS: [usize; 2] = [6, 8];
    const TOOLBAR_ITEMS: [(ToolbarItem, ToolbarIcon, &str); 13] = [
        (ToolbarItem::Select, ToolbarIcon::Select, "选择和移动标注"),
        (
            ToolbarItem::Tool(AnnotationTool::Rectangle),
            ToolbarIcon::Rectangle,
            "矩形",
        ),
        (
            ToolbarItem::Tool(AnnotationTool::Arrow),
            ToolbarIcon::Arrow,
            "箭头",
        ),
        (
            ToolbarItem::Tool(AnnotationTool::Pen),
            ToolbarIcon::Pen,
            "画笔",
        ),
        (
            ToolbarItem::Tool(AnnotationTool::Text),
            ToolbarIcon::Text,
            "文字",
        ),
        (
            ToolbarItem::Tool(AnnotationTool::Mosaic),
            ToolbarIcon::Mosaic,
            "马赛克",
        ),
        (ToolbarItem::Undo, ToolbarIcon::Undo, "撤销 (Ctrl+Z)"),
        (ToolbarItem::Redo, ToolbarIcon::Redo, "重做 (Ctrl+Y)"),
        (
            ToolbarItem::Action(CaptureAction::Copy),
            ToolbarIcon::Copy,
            "复制 (Ctrl+C)",
        ),
        (
            ToolbarItem::Action(CaptureAction::Save),
            ToolbarIcon::Save,
            "保存到文件 (Ctrl+S)",
        ),
        (
            ToolbarItem::Action(CaptureAction::DecodeQr),
            ToolbarIcon::Qr,
            "识别二维码 (Q)",
        ),
        (
            ToolbarItem::Action(CaptureAction::Pin),
            ToolbarIcon::Pin,
            "贴到屏幕 (P)",
        ),
        (
            ToolbarItem::Action(CaptureAction::Finish),
            ToolbarIcon::Finish,
            "完成 (Enter)",
        ),
    ];
    const HGDI_ERROR: HGDIOBJ = -1isize as HGDIOBJ;

    struct OverlayShared {
        selection: SelectionState,
        window_handles: Vec<isize>,
        double_click_candidate: bool,
        action: Option<CaptureAction>,
        active_tool: Option<AnnotationTool>,
        annotation_style: AnnotationStyle,
        annotations: Vec<Annotation>,
        redo_annotations: Vec<Annotation>,
        draft_annotation: Option<Annotation>,
        hovered_item: Option<ToolbarItem>,
        hovered_parameter: Option<ParameterItem>,
        hover_candidate: Option<PhysicalRect>,
        pending_candidate: Option<(PhysicalPoint, PhysicalRect)>,
        last_candidate_probe: Instant,
        text_editor: Option<TextEditorState>,
        selected_annotation: Option<usize>,
        annotation_drag: Option<AnnotationDrag>,
    }

    struct TextEditorState {
        window: isize,
        parent: isize,
        anchor: PhysicalPoint,
        style: AnnotationStyle,
        font: isize,
    }

    struct AnnotationDrag {
        index: usize,
        pointer: PhysicalPoint,
        original_points: Vec<PhysicalPoint>,
    }

    struct OverlayWindowContext {
        snapshot: Arc<VirtualDesktopSnapshot>,
        monitor_index: usize,
        dimmed_bgra: Vec<u8>,
        back_buffer: RefCell<Option<OverlayBackBuffer>>,
        shared: Arc<Mutex<OverlayShared>>,
        candidate_detector: Rc<RefCell<CaptureCandidateDetector>>,
    }

    #[derive(Clone, Copy)]
    struct ToolbarRenderState {
        active_tool: Option<AnnotationTool>,
        hovered_item: Option<ToolbarItem>,
        hovered_parameter: Option<ParameterItem>,
        annotation_style: AnnotationStyle,
        can_undo: bool,
        can_redo: bool,
    }

    struct OverlayBackBuffer {
        device_context: HDC,
        bitmap: HGDIOBJ,
        previous: HGDIOBJ,
        width: u32,
        height: u32,
    }

    struct GdiPlusToken(usize);

    impl GdiPlusToken {
        fn start() -> Option<Self> {
            let input = GdiplusStartupInput {
                GdiplusVersion: 1,
                DebugEventCallback: 0,
                SuppressBackgroundThread: 0,
                SuppressExternalCodecs: 0,
            };
            let mut token = 0usize;
            (unsafe { GdiplusStartup(&mut token, &input, null_mut()) } == 0).then_some(Self(token))
        }
    }

    impl Drop for GdiPlusToken {
        fn drop(&mut self) {
            unsafe {
                GdiplusShutdown(self.0);
            }
        }
    }

    struct GdiPlusGraphics(*mut GpGraphics);

    impl GdiPlusGraphics {
        fn from_hdc(device_context: HDC) -> Option<Self> {
            let mut graphics = null_mut();
            if unsafe { GdipCreateFromHDC(device_context, &mut graphics) } != 0
                || graphics.is_null()
            {
                return None;
            }
            unsafe {
                let _ = GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias8x8);
            }
            Some(Self(graphics))
        }
    }

    impl Drop for GdiPlusGraphics {
        fn drop(&mut self) {
            unsafe {
                let _ = GdipDeleteGraphics(self.0);
            }
        }
    }

    impl OverlayBackBuffer {
        fn new(target: HDC, width: u32, height: u32) -> Option<Self> {
            let device_context = unsafe { CreateCompatibleDC(target) };
            if device_context.is_null() {
                return None;
            }
            let bitmap =
                unsafe { CreateCompatibleBitmap(target, width as i32, height as i32) } as HGDIOBJ;
            if bitmap.is_null() {
                unsafe {
                    let _ = DeleteDC(device_context);
                }
                return None;
            }
            let previous = unsafe { SelectObject(device_context, bitmap) };
            if previous.is_null() || previous == HGDI_ERROR {
                unsafe {
                    let _ = DeleteObject(bitmap);
                    let _ = DeleteDC(device_context);
                }
                return None;
            }
            Some(Self {
                device_context,
                bitmap,
                previous,
                width,
                height,
            })
        }
    }

    impl Drop for OverlayBackBuffer {
        fn drop(&mut self) {
            unsafe {
                let _ = SelectObject(self.device_context, self.previous);
                let _ = DeleteObject(self.bitmap);
                let _ = DeleteDC(self.device_context);
            }
        }
    }

    pub(super) fn select(
        snapshot: Arc<VirtualDesktopSnapshot>,
    ) -> Result<Option<CaptureSelection>, ScreenshotError> {
        let _class = OverlayWindowClass::register()?;
        let _gdi_plus = GdiPlusToken::start();
        let candidate_detector = Rc::new(RefCell::new(CaptureCandidateDetector::snapshot()));
        let previous_foreground = unsafe { GetForegroundWindow() };
        let shared = Arc::new(Mutex::new(OverlayShared {
            selection: SelectionState::new(snapshot.virtual_bounds),
            window_handles: Vec::with_capacity(snapshot.frames.len()),
            double_click_candidate: false,
            action: None,
            active_tool: None,
            annotation_style: AnnotationStyle::default(),
            annotations: Vec::new(),
            redo_annotations: Vec::new(),
            draft_annotation: None,
            hovered_item: None,
            hovered_parameter: None,
            hover_candidate: None,
            pending_candidate: None,
            last_candidate_probe: Instant::now()
                .checked_sub(CANDIDATE_PROBE_INTERVAL)
                .unwrap_or_else(Instant::now),
            text_editor: None,
            selected_annotation: None,
            annotation_drag: None,
        }));
        let module = module_handle()?;
        let mut contexts = Vec::with_capacity(snapshot.frames.len());
        let mut windows = Vec::with_capacity(snapshot.frames.len());

        for monitor_index in 0..snapshot.frames.len() {
            let frame = &snapshot.frames[monitor_index];
            let mut context = Box::new(OverlayWindowContext {
                snapshot: Arc::clone(&snapshot),
                monitor_index,
                dimmed_bgra: dimmed_bgra(&frame.bgra, OVERLAY_ALPHA),
                back_buffer: RefCell::new(None),
                shared: Arc::clone(&shared),
                candidate_detector: Rc::clone(&candidate_detector),
            });
            let bounds = frame.monitor.physical_bounds;
            let window = unsafe {
                CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                    CLASS_NAME.as_ptr(),
                    CLASS_NAME.as_ptr(),
                    WS_POPUP,
                    bounds.left,
                    bounds.top,
                    i32::try_from(bounds.width().ok_or(ScreenshotError::InvalidTopology)?)
                        .map_err(|_| ScreenshotError::InvalidTopology)?,
                    i32::try_from(bounds.height().ok_or(ScreenshotError::InvalidTopology)?)
                        .map_err(|_| ScreenshotError::InvalidTopology)?,
                    null_mut(),
                    null_mut(),
                    module,
                    (&mut *context as *mut OverlayWindowContext).cast(),
                )
            };
            if window.is_null() {
                cleanup_windows(&windows);
                return Err(ScreenshotError::WindowsApi("CreateWindowExW"));
            }
            contexts.push(context);
            windows.push(window);
        }
        {
            let mut state = shared
                .lock()
                .map_err(|_| ScreenshotError::OverlayStateUnavailable)?;
            state.window_handles = windows.iter().map(|window| *window as isize).collect();
        }

        for window in &windows {
            unsafe {
                ShowWindow(*window, SW_SHOW);
                let _ = UpdateWindow(*window);
            }
        }
        let focus_index = snapshot
            .frames
            .iter()
            .position(|frame| frame.monitor.is_primary)
            .unwrap_or(0);
        unsafe {
            let focus_window = windows[focus_index];
            let _ = SetForegroundWindow(focus_window);
            let _ = SetFocus(focus_window);
            // UI Automation runs on a detached COM worker. Polling from a timer
            // lets a refined element result replace the top-level window even
            // after the pointer has stopped moving.
            let _ = SetTimer(
                focus_window,
                CANDIDATE_TIMER_ID,
                CANDIDATE_PROBE_INTERVAL.as_millis() as u32,
                None,
            );
        }

        let loop_result = message_loop(&shared);
        finish_text_editor(&shared, false);
        unsafe {
            let _ = KillTimer(windows[focus_index], CANDIDATE_TIMER_ID);
        }
        for window in &windows {
            unsafe {
                SetWindowLongPtrW(*window, GWLP_USERDATA, 0);
                DestroyWindow(*window);
            }
        }
        drop(contexts);
        if !previous_foreground.is_null() && unsafe { IsWindow(previous_foreground) } != 0 {
            unsafe {
                let _ = SetForegroundWindow(previous_foreground);
            }
        }
        loop_result?;

        let state = shared
            .lock()
            .map_err(|_| ScreenshotError::OverlayStateUnavailable)?;
        match state.selection.outcome() {
            Some(SelectionOutcome::Confirmed(selection)) => Ok(Some(CaptureSelection {
                rect: selection,
                action: state.action.unwrap_or(CaptureAction::Finish),
                annotations: state.annotations.clone(),
            })),
            Some(SelectionOutcome::Cancelled) | None => Ok(None),
        }
    }

    pub(super) struct OverlayWindowClass {
        module: HINSTANCE,
    }

    impl OverlayWindowClass {
        pub(super) fn register() -> Result<Self, ScreenshotError> {
            let module = module_handle()?;
            let class = WNDCLASSW {
                style: CS_DBLCLKS,
                lpfnWndProc: Some(window_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: module,
                hIcon: null_mut(),
                hCursor: unsafe { LoadCursorW(null_mut(), IDC_CROSS) },
                hbrBackground: null_mut(),
                lpszMenuName: null(),
                lpszClassName: CLASS_NAME.as_ptr(),
            };
            if unsafe { RegisterClassW(&class) } == 0 {
                return Err(ScreenshotError::WindowsApi("RegisterClassW"));
            }
            Ok(Self { module })
        }
    }

    impl Drop for OverlayWindowClass {
        fn drop(&mut self) {
            unsafe {
                let _ = UnregisterClassW(CLASS_NAME.as_ptr(), self.module);
            }
        }
    }

    fn module_handle() -> Result<HINSTANCE, ScreenshotError> {
        let module = unsafe { GetModuleHandleW(null()) };
        if module.is_null() {
            return Err(ScreenshotError::WindowsApi("GetModuleHandleW"));
        }
        Ok(module)
    }

    fn cleanup_windows(windows: &[HWND]) {
        for window in windows {
            unsafe {
                SetWindowLongPtrW(*window, GWLP_USERDATA, 0);
                DestroyWindow(*window);
            }
        }
    }

    fn message_loop(shared: &Arc<Mutex<OverlayShared>>) -> Result<(), ScreenshotError> {
        let mut message: MSG = unsafe { zeroed() };
        loop {
            let result = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
            if result > 0 {
                if handle_text_editor_key(shared, &message) {
                    continue;
                }
                unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            } else if result == 0 {
                return Ok(());
            } else {
                return Err(ScreenshotError::WindowsApi("GetMessageW"));
            }
        }
    }

    fn handle_text_editor_key(shared: &Arc<Mutex<OverlayShared>>, message: &MSG) -> bool {
        if message.message != WM_KEYDOWN {
            return false;
        }
        let editor = shared
            .lock()
            .ok()
            .and_then(|state| state.text_editor.as_ref().map(|editor| editor.window));
        if editor != Some(message.hwnd as isize) {
            return false;
        }
        match message.wParam {
            key if key == VK_RETURN as usize => {
                finish_text_editor(shared, true);
                true
            }
            key if key == VK_ESCAPE as usize => {
                finish_text_editor(shared, false);
                true
            }
            _ => false,
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
                SetWindowLongPtrW(window, GWLP_USERDATA, (*create).lpCreateParams as isize);
            }
        }
        let context = GetWindowLongPtrW(window, GWLP_USERDATA) as *const OverlayWindowContext;
        if context.is_null() {
            return DefWindowProcW(window, message, wparam, lparam);
        }
        let context = &*context;
        match message {
            WM_ERASEBKGND => 1,
            WM_CTLCOLOREDIT => {
                let editor_style = context.shared.lock().ok().and_then(|shared| {
                    shared
                        .text_editor
                        .as_ref()
                        .and_then(|editor| (editor.window == lparam).then_some(editor.style))
                });
                if let Some(style) = editor_style {
                    let edit_dc = wparam as HDC;
                    let _ = SetBkMode(edit_dc, TRANSPARENT as i32);
                    let _ = SetTextColor(edit_dc, rgb_to_colorref(style.color));
                    return GetStockObject(HOLLOW_BRUSH) as LRESULT;
                }
                DefWindowProcW(window, message, wparam, lparam)
            }
            WM_PAINT => {
                paint(window, context);
                0
            }
            WM_LBUTTONDOWN => {
                let _ = SetFocus(window);
                finish_text_editor(&context.shared, true);
                if activate_toolbar_item_at_cursor(context) {
                    return 0;
                }
                if begin_pointer(window, context) {
                    let _ = SetCapture(window);
                }
                0
            }
            WM_MOUSEMOVE => {
                if is_dragging(context) {
                    clear_double_click_candidate(context);
                    update_pointer(context, PointerAction::Update);
                } else {
                    update_capture_candidate(context);
                    update_toolbar_hover(context);
                }
                0
            }
            WM_TIMER if wparam == CANDIDATE_TIMER_ID => {
                if !is_dragging(context) {
                    update_capture_candidate(context);
                }
                0
            }
            WM_LBUTTONUP => {
                update_pointer(context, PointerAction::Finish);
                let _ = ReleaseCapture();
                0
            }
            WM_LBUTTONDBLCLK => {
                confirm_double_click_candidate(context);
                0
            }
            WM_RBUTTONDOWN | WM_CLOSE | WM_DISPLAYCHANGE => {
                cancel(context);
                0
            }
            WM_KEYDOWN if wparam == VK_ESCAPE as usize => {
                cancel(context);
                0
            }
            WM_KEYDOWN if wparam == VK_RETURN as usize => {
                confirm_with_action(context, CaptureAction::Finish);
                0
            }
            WM_KEYDOWN if undo_redo_key(wparam).is_some() => {
                if let Some(redo) = undo_redo_key(wparam) {
                    undo_or_redo(context, redo);
                }
                0
            }
            WM_KEYDOWN if keyboard_action(wparam).is_some() => {
                if let Some(action) = keyboard_action(wparam) {
                    confirm_with_action(context, action);
                }
                0
            }
            WM_KEYDOWN if arrow_delta(wparam).is_some() => {
                adjust_selection(context, wparam);
                0
            }
            WM_ACTIVATEAPP if wparam == 0 => {
                cancel(context);
                0
            }
            WM_CAPTURECHANGED if is_dragging(context) => {
                cancel(context);
                0
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }

    enum PointerAction {
        Update,
        Finish,
    }

    fn begin_pointer(window: HWND, context: &OverlayWindowContext) -> bool {
        let Some(point) = cursor_position() else {
            return false;
        };
        let text_request = context.shared.lock().ok().and_then(|shared| {
            let selection = shared.selection.selection()?;
            (shared.active_tool == Some(AnnotationTool::Text) && contains(selection, point))
                .then_some((selection, shared.annotation_style))
        });
        if let Some((selection, style)) = text_request {
            start_text_editor(window, context, selection, point, style);
            return false;
        }
        let (handles, started) = {
            let Ok(mut shared) = context.shared.lock() else {
                unsafe { PostQuitMessage(1) };
                return false;
            };
            let selection = shared.selection.selection();
            let started = if let Some(selection) = selection {
                if let Some(handle) = selection_handle_at(selection, point) {
                    shared.selection.begin_resize(point, handle)
                } else if contains(selection, point) {
                    if let Some(tool) = shared.active_tool {
                        shared.selected_annotation = None;
                        shared.draft_annotation =
                            Some(Annotation::with_style(tool, point, shared.annotation_style));
                        true
                    } else if let Some(index) = annotation_at(&shared.annotations, point) {
                        shared.selected_annotation = Some(index);
                        shared.annotation_drag = Some(AnnotationDrag {
                            index,
                            pointer: point,
                            original_points: shared.annotations[index].points.clone(),
                        });
                        true
                    } else {
                        shared.selected_annotation = None;
                        shared.selection.begin_move(point)
                    }
                } else {
                    shared.selected_annotation = None;
                    shared.annotations.clear();
                    shared.redo_annotations.clear();
                    shared.selection.begin(point);
                    true
                }
            } else if let Some(candidate) = shared.hover_candidate.take() {
                shared.pending_candidate = Some((point, candidate));
                true
            } else {
                shared.selection.begin(point);
                true
            };
            shared.double_click_candidate = selection
                .is_some_and(|selection| contains(selection, point))
                && shared.active_tool.is_none();
            shared.hovered_item = None;
            shared.hovered_parameter = None;
            (shared.window_handles.clone(), started)
        };
        invalidate_windows(&handles);
        started
    }

    fn update_pointer(context: &OverlayWindowContext, action: PointerAction) {
        let Some(point) = cursor_position() else {
            return;
        };
        let handles = {
            let Ok(mut shared) = context.shared.lock() else {
                unsafe { PostQuitMessage(1) };
                return;
            };
            let annotation_bounds = shared
                .selection
                .selection()
                .unwrap_or(context.snapshot.virtual_bounds);
            if let Some((anchor, candidate)) = shared.pending_candidate {
                let moved = (point.x - anchor.x).abs() >= CANDIDATE_DRAG_THRESHOLD
                    || (point.y - anchor.y).abs() >= CANDIDATE_DRAG_THRESHOLD;
                match action {
                    PointerAction::Update if moved => {
                        shared.pending_candidate = None;
                        shared.selection.begin(anchor);
                        shared.selection.update(point);
                    }
                    PointerAction::Finish => {
                        shared.pending_candidate = None;
                        if moved {
                            shared.selection.begin(anchor);
                            shared.selection.finish(point);
                        } else {
                            let _ = shared.selection.set_selection(candidate);
                        }
                    }
                    PointerAction::Update => {}
                }
            } else if let Some(drag) = shared.annotation_drag.take() {
                let (delta_x, delta_y) = clamped_annotation_delta(
                    &drag.original_points,
                    point.x.saturating_sub(drag.pointer.x),
                    point.y.saturating_sub(drag.pointer.y),
                    annotation_bounds,
                );
                if let Some(annotation) = shared.annotations.get_mut(drag.index) {
                    annotation.points = drag
                        .original_points
                        .iter()
                        .map(|point| {
                            PhysicalPoint::new(
                                point.x.saturating_add(delta_x),
                                point.y.saturating_add(delta_y),
                            )
                        })
                        .collect();
                }
                if matches!(action, PointerAction::Update) {
                    shared.annotation_drag = Some(drag);
                }
            } else if let Some(draft) = shared.draft_annotation.as_mut() {
                draft.update(clamp_to_selection(point, annotation_bounds));
                if matches!(action, PointerAction::Finish) {
                    let draft = shared.draft_annotation.take().unwrap();
                    if draft.is_visible() {
                        shared.annotations.push(draft);
                        shared.redo_annotations.clear();
                    }
                }
            } else {
                let before = shared.selection.selection();
                match action {
                    PointerAction::Update => shared.selection.update(point),
                    PointerAction::Finish => shared.selection.finish(point),
                }
                let after = shared.selection.selection();
                if let (Some(before), Some(after)) = (before, after) {
                    if before.width() == after.width()
                        && before.height() == after.height()
                        && before != after
                    {
                        translate_annotations(
                            &mut shared.annotations,
                            after.left - before.left,
                            after.top - before.top,
                        );
                    }
                }
            }
            shared.window_handles.clone()
        };
        invalidate_windows(&handles);
    }

    fn clear_double_click_candidate(context: &OverlayWindowContext) {
        if let Ok(mut shared) = context.shared.lock() {
            shared.double_click_candidate = false;
        }
    }

    fn cursor_position() -> Option<PhysicalPoint> {
        let mut point = POINT { x: 0, y: 0 };
        (unsafe { GetCursorPos(&mut point) } != 0).then_some(PhysicalPoint::new(point.x, point.y))
    }

    fn start_text_editor(
        parent: HWND,
        context: &OverlayWindowContext,
        selection: PhysicalRect,
        point: PhysicalPoint,
        style: AnnotationStyle,
    ) {
        finish_text_editor(&context.shared, true);
        let monitor = context.snapshot.frames[context.monitor_index]
            .monitor
            .physical_bounds;
        let width = (selection.right - point.x).clamp(120, 360);
        let height = i32::from(style.font_size.clamp(10, 72)) + 18;
        let local_x = point.x.saturating_sub(monitor.left);
        let local_y = point.y.saturating_sub(monitor.top);
        let editor = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TRANSPARENT,
                EDIT_CLASS.as_ptr(),
                null(),
                WS_CHILD | WS_VISIBLE | ES_AUTOHSCROLL as u32,
                local_x,
                local_y,
                width,
                height,
                parent,
                null_mut(),
                module_handle().unwrap_or(null_mut()),
                null_mut(),
            )
        };
        if editor.is_null() {
            return;
        }
        let cue: Vec<u16> = "输入文字，Enter 完成\0".encode_utf16().collect();
        unsafe {
            let _ = SendMessageW(editor, EM_SETCUEBANNER, 1, cue.as_ptr() as isize);
        }
        let font = create_text_font(style.font_size);
        if !font.is_null() {
            unsafe {
                let _ = SendMessageW(editor, WM_SETFONT, font as usize, 1);
            }
        }
        let handles = context.shared.lock().ok().map(|mut shared| {
            shared.text_editor = Some(TextEditorState {
                window: editor as isize,
                parent: parent as isize,
                anchor: point,
                style,
                font: font as isize,
            });
            shared.window_handles.clone()
        });
        unsafe {
            let _ = ShowWindow(editor, SW_SHOW);
            let _ = SetFocus(editor);
        }
        if let Some(handles) = handles {
            invalidate_windows(&handles);
        }
    }

    fn finish_text_editor(shared: &Arc<Mutex<OverlayShared>>, commit: bool) {
        let Some((editor, handles)) = shared.lock().ok().and_then(|mut shared| {
            let editor = shared.text_editor.take()?;
            Some((editor, shared.window_handles.clone()))
        }) else {
            return;
        };
        let window = editor.window as HWND;
        let text = if commit && !window.is_null() && unsafe { IsWindow(window) } != 0 {
            let length = unsafe { GetWindowTextLengthW(window) }.max(0) as usize;
            let mut wide = vec![0u16; length.saturating_add(1)];
            let copied = unsafe {
                GetWindowTextW(
                    window,
                    wide.as_mut_ptr(),
                    i32::try_from(wide.len()).unwrap_or(i32::MAX),
                )
            }
            .max(0) as usize;
            String::from_utf16(&wide[..copied]).ok()
        } else {
            None
        };
        if !window.is_null() && unsafe { IsWindow(window) } != 0 {
            unsafe {
                let _ = DestroyWindow(window);
            }
        }
        let font = editor.font as HGDIOBJ;
        if !font.is_null() {
            unsafe {
                let _ = DeleteObject(font);
            }
        }
        if commit {
            if let Some(text) = text.filter(|text| !text.trim().is_empty()) {
                if let Ok(mut state) = shared.lock() {
                    state
                        .annotations
                        .push(Annotation::text(editor.anchor, text, editor.style));
                    state.redo_annotations.clear();
                }
            }
        }
        let parent = editor.parent as HWND;
        if !parent.is_null() && unsafe { IsWindow(parent) } != 0 {
            unsafe {
                let _ = SetFocus(parent);
            }
        }
        invalidate_windows(&handles);
    }

    fn is_dragging(context: &OverlayWindowContext) -> bool {
        context
            .shared
            .lock()
            .map(|shared| {
                shared.selection.dragging()
                    || shared.draft_annotation.is_some()
                    || shared.annotation_drag.is_some()
                    || shared.pending_candidate.is_some()
            })
            .unwrap_or(false)
    }

    fn confirm_with_action(context: &OverlayWindowContext, action: CaptureAction) {
        if action == CaptureAction::Cancel {
            cancel(context);
            return;
        }
        finish_text_editor(&context.shared, true);
        let should_quit = context
            .shared
            .lock()
            .map(|mut shared| {
                shared.action = Some(action);
                shared.selection.confirm()
            })
            .unwrap_or(true);
        if should_quit {
            unsafe { PostQuitMessage(0) };
        }
    }

    fn confirm_double_click_candidate(context: &OverlayWindowContext) {
        finish_text_editor(&context.shared, true);
        let should_quit = context
            .shared
            .lock()
            .map(|mut shared| {
                let candidate = shared.double_click_candidate;
                shared.double_click_candidate = false;
                if candidate {
                    shared.action = Some(CaptureAction::Finish);
                }
                candidate && shared.selection.confirm()
            })
            .unwrap_or(true);
        if should_quit {
            unsafe { PostQuitMessage(0) };
        }
    }

    fn arrow_delta(key: WPARAM) -> Option<(i32, i32)> {
        match key as u16 {
            VK_LEFT => Some((-1, 0)),
            VK_RIGHT => Some((1, 0)),
            VK_UP => Some((0, -1)),
            VK_DOWN => Some((0, 1)),
            _ => None,
        }
    }

    fn keyboard_action(key: WPARAM) -> Option<CaptureAction> {
        let control = unsafe { GetKeyState(VK_CONTROL as i32) } < 0;
        match (control, key as u16) {
            (true, 0x43) => Some(CaptureAction::Copy),
            (true, 0x53) => Some(CaptureAction::Save),
            (false, 0x50) => Some(CaptureAction::Pin),
            (false, 0x51) => Some(CaptureAction::DecodeQr),
            _ => None,
        }
    }

    fn undo_redo_key(key: WPARAM) -> Option<bool> {
        let control = unsafe { GetKeyState(VK_CONTROL as i32) } < 0;
        match (control, key as u16) {
            (true, 0x5A) => Some(false),
            (true, 0x59) => Some(true),
            _ => None,
        }
    }

    fn activate_toolbar_item_at_cursor(context: &OverlayWindowContext) -> bool {
        let Some(point) = cursor_position() else {
            return false;
        };
        let (parameter, item) = context
            .shared
            .lock()
            .ok()
            .and_then(|shared| {
                (!shared.selection.dragging())
                    .then_some(shared.selection.selection())
                    .flatten()
                    .map(|selection| {
                        (
                            shared.active_tool.and_then(|tool| {
                                toolbar_parameter_at(context, selection, tool, point)
                            }),
                            toolbar_item_at(context, selection, point),
                        )
                    })
            })
            .unwrap_or((None, None));
        if let Some(parameter) = parameter {
            activate_toolbar_parameter(context, parameter);
            true
        } else if let Some(item) = item {
            activate_toolbar_item(context, item);
            true
        } else {
            false
        }
    }

    fn update_capture_candidate(context: &OverlayWindowContext) {
        let Some(point) = cursor_position() else {
            return;
        };
        let should_probe = {
            let Ok(mut shared) = context.shared.lock() else {
                return;
            };
            if shared.selection.selection().is_some()
                || shared.pending_candidate.is_some()
                || shared.last_candidate_probe.elapsed() < CANDIDATE_PROBE_INTERVAL
            {
                return;
            }
            shared.last_candidate_probe = Instant::now();
            true
        };
        if !should_probe {
            return;
        }
        let candidate = context
            .candidate_detector
            .try_borrow_mut()
            .ok()
            .and_then(|mut detector| detector.candidate_at(point))
            .and_then(|candidate| candidate.intersection(context.snapshot.virtual_bounds));
        let handles = {
            let Ok(mut shared) = context.shared.lock() else {
                return;
            };
            if candidate == shared.hover_candidate {
                return;
            }
            shared.hover_candidate = candidate;
            shared.window_handles.clone()
        };
        invalidate_windows(&handles);
    }

    fn update_toolbar_hover(context: &OverlayWindowContext) {
        let Some(point) = cursor_position() else {
            return;
        };
        let handles = {
            let Ok(mut shared) = context.shared.lock() else {
                return;
            };
            let selection = shared.selection.selection();
            let hovered =
                selection.and_then(|selection| toolbar_item_at(context, selection, point));
            let hovered_parameter = selection.and_then(|selection| {
                shared
                    .active_tool
                    .and_then(|tool| toolbar_parameter_at(context, selection, tool, point))
            });
            if hovered == shared.hovered_item && hovered_parameter == shared.hovered_parameter {
                return;
            }
            shared.hovered_item = hovered;
            shared.hovered_parameter = hovered_parameter;
            shared.window_handles.clone()
        };
        invalidate_windows(&handles);
    }

    fn activate_toolbar_item(context: &OverlayWindowContext, item: ToolbarItem) {
        match item {
            ToolbarItem::Select => {
                let handles = context.shared.lock().ok().map(|mut shared| {
                    shared.active_tool = None;
                    shared.hovered_item = Some(item);
                    shared.hovered_parameter = None;
                    shared.window_handles.clone()
                });
                if let Some(handles) = handles {
                    invalidate_windows(&handles);
                }
            }
            ToolbarItem::Tool(tool) => {
                let handles = context.shared.lock().ok().map(|mut shared| {
                    shared.active_tool = (shared.active_tool != Some(tool)).then_some(tool);
                    shared.selected_annotation = None;
                    shared.hovered_item = Some(item);
                    shared.hovered_parameter = None;
                    shared.window_handles.clone()
                });
                if let Some(handles) = handles {
                    invalidate_windows(&handles);
                }
            }
            ToolbarItem::Undo => undo_or_redo(context, false),
            ToolbarItem::Redo => undo_or_redo(context, true),
            ToolbarItem::Action(action) => confirm_with_action(context, action),
        }
    }

    fn activate_toolbar_parameter(context: &OverlayWindowContext, parameter: ParameterItem) {
        let handles = context.shared.lock().ok().map(|mut shared| {
            match parameter {
                ParameterItem::Color(index) => {
                    if let Some(color) = ANNOTATION_COLORS.get(index) {
                        shared.annotation_style.color = *color;
                    }
                }
                ParameterItem::Size(value) => match shared.active_tool {
                    Some(AnnotationTool::Mosaic) => {
                        shared.annotation_style.mosaic_block = value;
                    }
                    Some(AnnotationTool::Text) => {
                        shared.annotation_style.font_size = value;
                    }
                    _ => {
                        shared.annotation_style.thickness = value;
                    }
                },
            }
            shared.hovered_parameter = Some(parameter);
            shared.window_handles.clone()
        });
        if let Some(handles) = handles {
            invalidate_windows(&handles);
        }
    }

    fn undo_or_redo(context: &OverlayWindowContext, redo: bool) {
        let handles = context.shared.lock().ok().and_then(|mut shared| {
            let changed = if redo {
                shared.redo_annotations.pop().map(|annotation| {
                    shared.annotations.push(annotation);
                    shared.selected_annotation = Some(shared.annotations.len() - 1);
                })
            } else {
                shared.annotations.pop().map(|annotation| {
                    shared.redo_annotations.push(annotation);
                    shared.selected_annotation = None;
                })
            };
            changed.map(|_| shared.window_handles.clone())
        });
        if let Some(handles) = handles {
            invalidate_windows(&handles);
        }
    }

    fn selection_handle_at(
        selection: PhysicalRect,
        point: PhysicalPoint,
    ) -> Option<SelectionHandle> {
        selection_handles(selection)
            .into_iter()
            .find_map(|(handle, center)| {
                ((point.x - center.x).abs() <= HANDLE_HIT_RADIUS
                    && (point.y - center.y).abs() <= HANDLE_HIT_RADIUS)
                    .then_some(handle)
            })
    }

    fn selection_handles(selection: PhysicalRect) -> [(SelectionHandle, PhysicalPoint); 8] {
        let center_x = selection.left + (selection.right - selection.left) / 2;
        let center_y = selection.top + (selection.bottom - selection.top) / 2;
        [
            (
                SelectionHandle::TopLeft,
                PhysicalPoint::new(selection.left, selection.top),
            ),
            (
                SelectionHandle::Top,
                PhysicalPoint::new(center_x, selection.top),
            ),
            (
                SelectionHandle::TopRight,
                PhysicalPoint::new(selection.right - 1, selection.top),
            ),
            (
                SelectionHandle::Right,
                PhysicalPoint::new(selection.right - 1, center_y),
            ),
            (
                SelectionHandle::BottomRight,
                PhysicalPoint::new(selection.right - 1, selection.bottom - 1),
            ),
            (
                SelectionHandle::Bottom,
                PhysicalPoint::new(center_x, selection.bottom - 1),
            ),
            (
                SelectionHandle::BottomLeft,
                PhysicalPoint::new(selection.left, selection.bottom - 1),
            ),
            (
                SelectionHandle::Left,
                PhysicalPoint::new(selection.left, center_y),
            ),
        ]
    }

    fn clamp_to_selection(point: PhysicalPoint, selection: PhysicalRect) -> PhysicalPoint {
        PhysicalPoint::new(
            point.x.clamp(selection.left, selection.right - 1),
            point.y.clamp(selection.top, selection.bottom - 1),
        )
    }

    fn annotation_at(annotations: &[Annotation], point: PhysicalPoint) -> Option<usize> {
        annotations
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, annotation)| annotation_hit(annotation, point).then_some(index))
    }

    fn annotation_hit(annotation: &Annotation, point: PhysicalPoint) -> bool {
        let Some(bounds) = annotation_bounds(annotation) else {
            return false;
        };
        match annotation.tool {
            AnnotationTool::Rectangle | AnnotationTool::Mosaic | AnnotationTool::Text => {
                contains(inflate_rect(bounds, 6), point)
            }
            AnnotationTool::Arrow | AnnotationTool::Pen => annotation
                .points
                .windows(2)
                .any(|pair| distance_to_segment(point, pair[0], pair[1]) <= 8.0),
        }
    }

    fn annotation_bounds(annotation: &Annotation) -> Option<PhysicalRect> {
        let first = *annotation.points.first()?;
        if annotation.tool == AnnotationTool::Text {
            let character_count = annotation
                .text
                .as_deref()
                .map(|text| text.chars().count())
                .unwrap_or(1)
                .max(1) as i32;
            let width = (character_count * i32::from(annotation.style.font_size) * 3 / 5).max(24);
            let height = i32::from(annotation.style.font_size).saturating_add(8);
            return PhysicalRect::new(
                first.x,
                first.y,
                first.x.saturating_add(width),
                first.y.saturating_add(height),
            )
            .ok();
        }
        let (min_x, max_x, min_y, max_y) = annotation.points.iter().fold(
            (first.x, first.x, first.y, first.y),
            |(min_x, max_x, min_y, max_y), point| {
                (
                    min_x.min(point.x),
                    max_x.max(point.x),
                    min_y.min(point.y),
                    max_y.max(point.y),
                )
            },
        );
        PhysicalRect::new(
            min_x,
            min_y,
            max_x.saturating_add(1).max(min_x + 1),
            max_y.saturating_add(1).max(min_y + 1),
        )
        .ok()
    }

    fn inflate_rect(rect: PhysicalRect, amount: i32) -> PhysicalRect {
        PhysicalRect {
            left: rect.left.saturating_sub(amount),
            top: rect.top.saturating_sub(amount),
            right: rect.right.saturating_add(amount),
            bottom: rect.bottom.saturating_add(amount),
        }
    }

    fn distance_to_segment(point: PhysicalPoint, start: PhysicalPoint, end: PhysicalPoint) -> f64 {
        let dx = f64::from(end.x - start.x);
        let dy = f64::from(end.y - start.y);
        if dx == 0.0 && dy == 0.0 {
            return (f64::from(point.x - start.x).powi(2) + f64::from(point.y - start.y).powi(2))
                .sqrt();
        }
        let projection = ((f64::from(point.x - start.x) * dx + f64::from(point.y - start.y) * dy)
            / (dx * dx + dy * dy))
            .clamp(0.0, 1.0);
        let closest_x = f64::from(start.x) + projection * dx;
        let closest_y = f64::from(start.y) + projection * dy;
        ((f64::from(point.x) - closest_x).powi(2) + (f64::from(point.y) - closest_y).powi(2)).sqrt()
    }

    fn clamped_annotation_delta(
        points: &[PhysicalPoint],
        delta_x: i32,
        delta_y: i32,
        selection: PhysicalRect,
    ) -> (i32, i32) {
        let Some(first) = points.first() else {
            return (0, 0);
        };
        let (min_x, max_x, min_y, max_y) = points.iter().fold(
            (first.x, first.x, first.y, first.y),
            |(min_x, max_x, min_y, max_y), point| {
                (
                    min_x.min(point.x),
                    max_x.max(point.x),
                    min_y.min(point.y),
                    max_y.max(point.y),
                )
            },
        );
        (
            delta_x.clamp(selection.left - min_x, selection.right - 1 - max_x),
            delta_y.clamp(selection.top - min_y, selection.bottom - 1 - max_y),
        )
    }

    fn translate_annotations(annotations: &mut [Annotation], delta_x: i32, delta_y: i32) {
        for annotation in annotations {
            for point in &mut annotation.points {
                point.x = point.x.saturating_add(delta_x);
                point.y = point.y.saturating_add(delta_y);
            }
        }
    }

    fn adjust_selection(context: &OverlayWindowContext, key: WPARAM) {
        let Some((delta_x, delta_y)) = arrow_delta(key) else {
            return;
        };
        let handles = {
            let Ok(mut shared) = context.shared.lock() else {
                unsafe { PostQuitMessage(1) };
                return;
            };
            let resizing = unsafe { GetKeyState(VK_SHIFT as i32) } < 0;
            let changed = if resizing {
                shared.selection.resize(delta_x, delta_y)
            } else {
                shared.selection.nudge(delta_x, delta_y)
            };
            changed.then(|| shared.window_handles.clone())
        };
        if let Some(handles) = handles {
            invalidate_windows(&handles);
        }
    }

    fn cancel(context: &OverlayWindowContext) {
        finish_text_editor(&context.shared, false);
        if let Ok(mut shared) = context.shared.lock() {
            shared.action = Some(CaptureAction::Cancel);
            shared.selection.cancel();
        }
        unsafe { PostQuitMessage(0) };
    }

    fn invalidate_windows(handles: &[isize]) {
        for handle in handles {
            unsafe {
                let _ = InvalidateRect(*handle as HWND, null(), 0);
            }
        }
    }

    fn paint(window: HWND, context: &OverlayWindowContext) {
        let mut paint: PAINTSTRUCT = unsafe { zeroed() };
        let device_context = unsafe { BeginPaint(window, &mut paint) };
        if device_context.is_null() {
            return;
        }
        let frame = &context.snapshot.frames[context.monitor_index];
        if !paint_buffered(device_context, context, frame) {
            draw_overlay(device_context, context, frame);
        }
        unsafe {
            EndPaint(window, &paint);
        }
    }

    fn paint_buffered(target: HDC, context: &OverlayWindowContext, frame: &MonitorFrame) -> bool {
        let Ok(mut back_buffer) = context.back_buffer.try_borrow_mut() else {
            return false;
        };
        let needs_recreate = match back_buffer.as_ref() {
            Some(buffer) => buffer.width != frame.width || buffer.height != frame.height,
            None => true,
        };
        if needs_recreate {
            *back_buffer = OverlayBackBuffer::new(target, frame.width, frame.height);
        }
        let Some(buffer) = back_buffer.as_ref() else {
            return false;
        };
        draw_overlay(buffer.device_context, context, frame);
        let copied = unsafe {
            BitBlt(
                target,
                0,
                0,
                frame.width as i32,
                frame.height as i32,
                buffer.device_context,
                0,
                0,
                SRCCOPY,
            )
        };
        copied != 0
    }

    fn draw_overlay(device_context: HDC, context: &OverlayWindowContext, frame: &MonitorFrame) {
        draw_pixels(
            device_context,
            frame.width,
            frame.height,
            &context.dimmed_bgra,
        );
        let (
            selection,
            dragging,
            active_tool,
            annotations,
            draft_annotation,
            hovered_item,
            hovered_parameter,
            annotation_style,
            can_redo,
            hover_candidate,
            selected_annotation,
        ) = context
            .shared
            .lock()
            .map(|shared| {
                (
                    shared.selection.selection(),
                    shared.selection.dragging()
                        || shared.annotation_drag.is_some()
                        || shared.draft_annotation.is_some(),
                    shared.active_tool,
                    shared.annotations.clone(),
                    shared.draft_annotation.clone(),
                    shared.hovered_item,
                    shared.hovered_parameter,
                    shared.annotation_style,
                    !shared.redo_annotations.is_empty(),
                    shared
                        .hover_candidate
                        .or_else(|| shared.pending_candidate.map(|(_, candidate)| candidate)),
                    shared.selected_annotation,
                )
            })
            .unwrap_or((
                None,
                false,
                None,
                Vec::new(),
                None,
                None,
                None,
                AnnotationStyle::default(),
                false,
                None,
                None,
            ));
        if let Some(selection) = selection {
            draw_selection(device_context, frame, selection);
            draw_annotations(
                device_context,
                frame,
                selection,
                annotations.iter().chain(draft_annotation.iter()),
            );
            if let Some(index) = selected_annotation {
                if let Some(annotation) = annotations.get(index) {
                    draw_annotation_selection(device_context, frame, annotation);
                }
            }
            draw_selection_handles(device_context, frame, selection);
            if !dragging {
                draw_toolbar(
                    device_context,
                    frame,
                    selection,
                    ToolbarRenderState {
                        active_tool,
                        hovered_item,
                        hovered_parameter,
                        annotation_style,
                        can_undo: !annotations.is_empty(),
                        can_redo,
                    },
                );
            }
        } else if let Some(candidate) = hover_candidate {
            draw_capture_candidate(device_context, frame, candidate);
        }
    }

    fn draw_frame(device_context: HDC, frame: &MonitorFrame) {
        draw_pixels(device_context, frame.width, frame.height, &frame.bgra);
    }

    fn draw_pixels(device_context: HDC, width: u32, height: u32, bgra: &[u8]) {
        let info = bitmap_info(width, height);
        unsafe {
            let _ = StretchDIBits(
                device_context,
                0,
                0,
                width as i32,
                height as i32,
                0,
                0,
                width as i32,
                height as i32,
                bgra.as_ptr().cast(),
                &info,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
        }
    }

    fn dimmed_bgra(source: &[u8], overlay_alpha: u8) -> Vec<u8> {
        let retained = u16::from(255 - overlay_alpha);
        let mut output = source.to_vec();
        for pixel in output.chunks_exact_mut(4) {
            pixel[0] = ((u16::from(pixel[0]) * retained + 127) / 255) as u8;
            pixel[1] = ((u16::from(pixel[1]) * retained + 127) / 255) as u8;
            pixel[2] = ((u16::from(pixel[2]) * retained + 127) / 255) as u8;
        }
        output
    }

    fn draw_selection(device_context: HDC, frame: &MonitorFrame, selection: PhysicalRect) {
        let monitor_bounds = frame.monitor.physical_bounds;
        let Some(intersection) = monitor_bounds.intersection(selection) else {
            return;
        };
        let local = RECT {
            left: intersection.left - monitor_bounds.left,
            top: intersection.top - monitor_bounds.top,
            right: intersection.right - monitor_bounds.left,
            bottom: intersection.bottom - monitor_bounds.top,
        };
        let saved = unsafe { SaveDC(device_context) };
        if saved != 0 {
            unsafe {
                let _ = IntersectClipRect(
                    device_context,
                    local.left,
                    local.top,
                    local.right,
                    local.bottom,
                );
            }
            draw_frame(device_context, frame);
            unsafe {
                let _ = RestoreDC(device_context, saved);
            }
        }
        draw_border(device_context, &local);
        if selection.left >= monitor_bounds.left
            && selection.left < monitor_bounds.right
            && selection.top >= monitor_bounds.top
            && selection.top < monitor_bounds.bottom
        {
            draw_size_label(device_context, monitor_bounds, selection);
        }
    }

    fn draw_capture_candidate(device_context: HDC, frame: &MonitorFrame, candidate: PhysicalRect) {
        let monitor = frame.monitor.physical_bounds;
        let Some(intersection) = monitor.intersection(candidate) else {
            return;
        };
        let local = to_local_rect(intersection, monitor);
        draw_border(device_context, &local);
        if contains(monitor, PhysicalPoint::new(candidate.left, candidate.top)) {
            draw_size_label(device_context, monitor, candidate);
        }
    }

    fn draw_selection_handles(device_context: HDC, frame: &MonitorFrame, selection: PhysicalRect) {
        let monitor = frame.monitor.physical_bounds;
        for (_, center) in selection_handles(selection) {
            if !contains(monitor, center) {
                continue;
            }
            let local_x = center.x - monitor.left;
            let local_y = center.y - monitor.top;
            let rect = RECT {
                left: local_x - HANDLE_RADIUS,
                top: local_y - HANDLE_RADIUS,
                right: local_x + HANDLE_RADIUS + 1,
                bottom: local_y + HANDLE_RADIUS + 1,
            };
            draw_ellipse(device_context, &rect, WHITE, ACCENT_COLOR);
        }
    }

    fn draw_annotations<'a>(
        device_context: HDC,
        frame: &MonitorFrame,
        selection: PhysicalRect,
        annotations: impl Iterator<Item = &'a Annotation>,
    ) {
        let monitor = frame.monitor.physical_bounds;
        let Some(intersection) = monitor.intersection(selection) else {
            return;
        };
        let saved = unsafe { SaveDC(device_context) };
        if saved == 0 {
            return;
        }
        unsafe {
            let _ = IntersectClipRect(
                device_context,
                intersection.left - monitor.left,
                intersection.top - monitor.top,
                intersection.right - monitor.left,
                intersection.bottom - monitor.top,
            );
        }
        for annotation in annotations {
            draw_annotation(device_context, frame, annotation);
        }
        unsafe {
            let _ = RestoreDC(device_context, saved);
        }
    }

    fn draw_annotation(device_context: HDC, frame: &MonitorFrame, annotation: &Annotation) {
        let monitor = frame.monitor.physical_bounds;
        if annotation.tool == AnnotationTool::Text {
            draw_text_annotation(device_context, monitor, annotation);
            return;
        }
        let pen = unsafe {
            CreatePen(
                PS_SOLID,
                i32::from(annotation.style.thickness.clamp(1, 12)),
                rgb_to_colorref(annotation.style.color),
            )
        } as HGDIOBJ;
        if pen.is_null() {
            return;
        }
        let previous = unsafe { SelectObject(device_context, pen) };
        match annotation.tool {
            AnnotationTool::Rectangle | AnnotationTool::Mosaic => {
                if let Some((start, end)) = annotation_endpoints(annotation) {
                    let left = start.x.min(end.x) - monitor.left;
                    let top = start.y.min(end.y) - monitor.top;
                    let right = start.x.max(end.x) - monitor.left;
                    let bottom = start.y.max(end.y) - monitor.top;
                    if annotation.tool == AnnotationTool::Mosaic {
                        draw_mosaic_preview(
                            device_context,
                            frame,
                            PhysicalRect::new(
                                start.x.min(end.x),
                                start.y.min(end.y),
                                start.x.max(end.x).saturating_add(1),
                                start.y.max(end.y).saturating_add(1),
                            )
                            .ok(),
                            annotation.style.mosaic_block,
                        );
                    } else {
                        draw_gdi_line(
                            device_context,
                            PhysicalPoint::new(left, top),
                            PhysicalPoint::new(right, top),
                        );
                        draw_gdi_line(
                            device_context,
                            PhysicalPoint::new(right, top),
                            PhysicalPoint::new(right, bottom),
                        );
                        draw_gdi_line(
                            device_context,
                            PhysicalPoint::new(right, bottom),
                            PhysicalPoint::new(left, bottom),
                        );
                        draw_gdi_line(
                            device_context,
                            PhysicalPoint::new(left, bottom),
                            PhysicalPoint::new(left, top),
                        );
                    }
                }
            }
            AnnotationTool::Arrow => {
                if let Some((start, end)) = annotation_endpoints(annotation) {
                    draw_arrow_preview(
                        device_context,
                        PhysicalPoint::new(start.x - monitor.left, start.y - monitor.top),
                        PhysicalPoint::new(end.x - monitor.left, end.y - monitor.top),
                    );
                }
            }
            AnnotationTool::Pen => {
                for pair in annotation.points.windows(2) {
                    draw_gdi_line(
                        device_context,
                        PhysicalPoint::new(pair[0].x - monitor.left, pair[0].y - monitor.top),
                        PhysicalPoint::new(pair[1].x - monitor.left, pair[1].y - monitor.top),
                    );
                }
            }
            AnnotationTool::Text => {}
        }
        unsafe {
            if !previous.is_null() && previous != HGDI_ERROR {
                let _ = SelectObject(device_context, previous);
            }
            let _ = DeleteObject(pen);
        }
    }

    fn draw_annotation_selection(
        device_context: HDC,
        frame: &MonitorFrame,
        annotation: &Annotation,
    ) {
        let Some(bounds) = annotation_bounds(annotation) else {
            return;
        };
        let monitor = frame.monitor.physical_bounds;
        let Some(bounds) = bounds.intersection(monitor) else {
            return;
        };
        let local = to_local_rect(inflate_rect(bounds, 4), monitor);
        frame_rect(device_context, &local, ACCENT_COLOR);
        for center in [
            PhysicalPoint::new(local.left, local.top),
            PhysicalPoint::new(local.right, local.top),
            PhysicalPoint::new(local.right, local.bottom),
            PhysicalPoint::new(local.left, local.bottom),
        ] {
            draw_ellipse(
                device_context,
                &RECT {
                    left: center.x - 3,
                    top: center.y - 3,
                    right: center.x + 4,
                    bottom: center.y + 4,
                },
                WHITE,
                ACCENT_COLOR,
            );
        }
    }

    fn draw_text_annotation(device_context: HDC, monitor: PhysicalRect, annotation: &Annotation) {
        let Some(anchor) = annotation.points.first() else {
            return;
        };
        let Some(text) = annotation
            .text
            .as_deref()
            .filter(|text| !text.trim().is_empty())
        else {
            return;
        };
        let wide: Vec<u16> = text.encode_utf16().collect();
        let font = create_text_font(annotation.style.font_size);
        let previous = (!font.is_null()).then(|| unsafe { SelectObject(device_context, font) });
        let mut rect = RECT {
            left: anchor.x - monitor.left,
            top: anchor.y - monitor.top,
            right: monitor.right - monitor.left,
            bottom: monitor.bottom - monitor.top,
        };
        unsafe {
            let _ = SetBkMode(device_context, TRANSPARENT as i32);
            let _ = SetTextColor(device_context, rgb_to_colorref(annotation.style.color));
            let _ = DrawTextW(
                device_context,
                wide.as_ptr(),
                wide.len() as i32,
                &mut rect,
                windows_sys::Win32::Graphics::Gdi::DT_LEFT
                    | windows_sys::Win32::Graphics::Gdi::DT_TOP
                    | DT_NOPREFIX,
            );
            if let Some(previous) = previous {
                if !previous.is_null() && previous != HGDI_ERROR {
                    let _ = SelectObject(device_context, previous);
                }
            }
            if !font.is_null() {
                let _ = DeleteObject(font);
            }
        }
    }

    fn annotation_endpoints(annotation: &Annotation) -> Option<(PhysicalPoint, PhysicalPoint)> {
        (annotation.points.len() >= 2)
            .then(|| (annotation.points[0], *annotation.points.last().unwrap()))
    }

    fn draw_gdi_line(device_context: HDC, start: PhysicalPoint, end: PhysicalPoint) {
        unsafe {
            let _ = MoveToEx(device_context, start.x, start.y, null_mut());
            let _ = LineTo(device_context, end.x, end.y);
        }
    }

    fn draw_arrow_preview(device_context: HDC, start: PhysicalPoint, end: PhysicalPoint) {
        draw_gdi_line(device_context, start, end);
        let dx = f64::from(end.x - start.x);
        let dy = f64::from(end.y - start.y);
        let length = (dx * dx + dy * dy).sqrt();
        if length < 2.0 {
            return;
        }
        let head = (length * 0.24).clamp(10.0, 22.0);
        let angle = dy.atan2(dx);
        for wing in [angle + 2.55, angle - 2.55] {
            draw_gdi_line(
                device_context,
                end,
                PhysicalPoint::new(
                    end.x + (head * wing.cos()).round() as i32,
                    end.y + (head * wing.sin()).round() as i32,
                ),
            );
        }
    }

    fn draw_mosaic_preview(
        device_context: HDC,
        frame: &MonitorFrame,
        requested: Option<PhysicalRect>,
        block: u8,
    ) {
        let Some(requested) = requested else {
            return;
        };
        let monitor = frame.monitor.physical_bounds;
        let Some(bounds) = requested.intersection(monitor) else {
            return;
        };
        let step = i32::from(block.clamp(4, 24));
        let mut y = bounds.top;
        while y < bounds.bottom {
            let mut x = bounds.left;
            while x < bounds.right {
                let block_rect = PhysicalRect::new(
                    x,
                    y,
                    (x + step).min(bounds.right),
                    (y + step).min(bounds.bottom),
                )
                .ok();
                if let Some(block_rect) = block_rect {
                    let color = average_frame_color(frame, block_rect).unwrap_or(0x00808080);
                    fill_rect(device_context, &to_local_rect(block_rect, monitor), color);
                }
                x += step;
            }
            y += step;
        }
    }

    fn average_frame_color(frame: &MonitorFrame, rect: PhysicalRect) -> Option<COLORREF> {
        let monitor = frame.monitor.physical_bounds;
        let rect = rect.intersection(monitor)?;
        let start_x = usize::try_from(rect.left - monitor.left).ok()?;
        let start_y = usize::try_from(rect.top - monitor.top).ok()?;
        let width = usize::try_from(rect.width()?).ok()?;
        let height = usize::try_from(rect.height()?).ok()?;
        let mut totals = [0u64; 3];
        let mut count = 0u64;
        for row in 0..height {
            let row_offset = start_y
                .checked_add(row)?
                .checked_mul(frame.stride)?
                .checked_add(start_x.checked_mul(4)?)?;
            for column in 0..width {
                let offset = row_offset.checked_add(column.checked_mul(4)?)?;
                let pixel = frame.bgra.get(offset..offset + 4)?;
                totals[0] += u64::from(pixel[2]);
                totals[1] += u64::from(pixel[1]);
                totals[2] += u64::from(pixel[0]);
                count += 1;
            }
        }
        (count > 0).then(|| {
            rgb_to_colorref([
                (totals[0] / count) as u8,
                (totals[1] / count) as u8,
                (totals[2] / count) as u8,
            ])
        })
    }

    fn draw_border(device_context: HDC, rect: &RECT) {
        let brush = unsafe { CreateSolidBrush(ACCENT_COLOR) };
        if brush.is_null() {
            return;
        }
        let mut inner = *rect;
        unsafe {
            let _ = FrameRect(device_context, rect, brush);
        }
        inner.left += 1;
        inner.top += 1;
        inner.right -= 1;
        inner.bottom -= 1;
        if inner.right > inner.left && inner.bottom > inner.top {
            unsafe {
                let _ = FrameRect(device_context, &inner, brush);
            }
        }
        unsafe {
            let _ = DeleteObject(brush as HGDIOBJ);
        }
    }

    fn draw_size_label(device_context: HDC, monitor_bounds: PhysicalRect, selection: PhysicalRect) {
        let Ok((width, height)) = selection_size(selection) else {
            return;
        };
        let text = format!("{width} × {height}");
        let wide: Vec<u16> = text.encode_utf16().collect();
        let local_left = selection.left - monitor_bounds.left;
        let local_top = selection.top - monitor_bounds.top;
        let monitor_height = monitor_bounds.height().unwrap_or(0) as i32;
        let mut top = local_top - LABEL_HEIGHT - 6;
        if top < 4 {
            top = (local_top + 6).min(monitor_height.saturating_sub(LABEL_HEIGHT + 4));
        }
        let monitor_width = monitor_bounds.width().unwrap_or(0) as i32;
        let left = local_left
            .max(4)
            .min(monitor_width.saturating_sub(LABEL_WIDTH + 4));
        let mut rect = RECT {
            left,
            top,
            right: left + LABEL_WIDTH,
            bottom: top + LABEL_HEIGHT,
        };
        let brush = unsafe { CreateSolidBrush(LABEL_BACKGROUND) };
        if !brush.is_null() {
            unsafe {
                let _ = FillRect(device_context, &rect, brush);
                let _ = DeleteObject(brush as HGDIOBJ);
            }
        }
        unsafe {
            let _ = SetBkMode(device_context, TRANSPARENT as i32);
            let _ = SetTextColor(device_context, WHITE);
            let _ = DrawTextW(
                device_context,
                wide.as_ptr(),
                wide.len() as i32,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
        }
    }

    fn toolbar_bounds(
        monitor_bounds: PhysicalRect,
        selection: PhysicalRect,
    ) -> Option<PhysicalRect> {
        let anchor = PhysicalPoint::new(selection.right - 1, selection.bottom - 1);
        if !contains(monitor_bounds, anchor) {
            return None;
        }
        let width = TOOLBAR_PADDING * 2
            + TOOLBAR_BUTTON_WIDTH * i32::try_from(TOOLBAR_ITEMS.len()).ok()?
            + TOOLBAR_GROUP_GAP * i32::try_from(TOOLBAR_GROUP_BREAKS.len()).ok()?;
        if i32::try_from(monitor_bounds.width()?).ok()? < width {
            return None;
        }
        let preferred_left = selection.right - width;
        let left = preferred_left.clamp(monitor_bounds.left, monitor_bounds.right - width);
        let below = selection.bottom + TOOLBAR_GAP;
        let above = selection.top - TOOLBAR_GAP - TOOLBAR_HEIGHT;
        let top = if below + TOOLBAR_HEIGHT <= monitor_bounds.bottom {
            below
        } else if above >= monitor_bounds.top {
            above
        } else {
            (monitor_bounds.bottom - TOOLBAR_HEIGHT).max(monitor_bounds.top)
        };
        PhysicalRect::new(left, top, left + width, top + TOOLBAR_HEIGHT).ok()
    }

    fn toolbar_button_rect(bounds: PhysicalRect, index: usize) -> Option<PhysicalRect> {
        let group_offset = TOOLBAR_GROUP_GAP
            * i32::try_from(
                TOOLBAR_GROUP_BREAKS
                    .iter()
                    .filter(|group_start| index >= **group_start)
                    .count(),
            )
            .ok()?;
        let index = i32::try_from(index).ok()?;
        let left = bounds.left + TOOLBAR_PADDING + index * TOOLBAR_BUTTON_WIDTH + group_offset;
        PhysicalRect::new(
            left,
            bounds.top + TOOLBAR_PADDING,
            left + TOOLBAR_BUTTON_WIDTH,
            bounds.bottom - TOOLBAR_PADDING,
        )
        .ok()
    }

    fn parameter_bar_width(tool: AnnotationTool) -> i32 {
        let color_width = if tool == AnnotationTool::Mosaic {
            0
        } else {
            COLOR_BUTTON_WIDTH * ANNOTATION_COLORS.len() as i32 + PARAMETER_GROUP_GAP
        };
        PARAMETER_PADDING * 2
            + color_width
            + SIZE_BUTTON_WIDTH * parameter_values(tool).len() as i32
    }

    fn parameter_values(tool: AnnotationTool) -> &'static [u8] {
        match tool {
            AnnotationTool::Mosaic => &MOSAIC_BLOCKS,
            AnnotationTool::Text => &TEXT_FONT_SIZES,
            AnnotationTool::Rectangle | AnnotationTool::Arrow | AnnotationTool::Pen => {
                &LINE_THICKNESSES
            }
        }
    }

    fn parameter_bounds(
        monitor: PhysicalRect,
        toolbar: PhysicalRect,
        tool: AnnotationTool,
    ) -> Option<PhysicalRect> {
        let width = parameter_bar_width(tool).min(monitor.right - monitor.left);
        let left = toolbar
            .left
            .clamp(monitor.left, monitor.right.saturating_sub(width));
        let below = toolbar.bottom + PARAMETER_GAP;
        let above = toolbar.top - PARAMETER_GAP - PARAMETER_HEIGHT;
        let top = if below + PARAMETER_HEIGHT <= monitor.bottom {
            below
        } else {
            above.max(monitor.top)
        };
        PhysicalRect::new(left, top, left + width, top + PARAMETER_HEIGHT).ok()
    }

    fn parameter_color_rect(bounds: PhysicalRect, index: usize) -> Option<PhysicalRect> {
        let index = i32::try_from(index).ok()?;
        let left = bounds.left + PARAMETER_PADDING + index * COLOR_BUTTON_WIDTH;
        PhysicalRect::new(
            left,
            bounds.top + PARAMETER_PADDING,
            left + COLOR_BUTTON_WIDTH,
            bounds.bottom - PARAMETER_PADDING,
        )
        .ok()
    }

    fn parameter_size_rect(
        bounds: PhysicalRect,
        index: usize,
        tool: AnnotationTool,
    ) -> Option<PhysicalRect> {
        let index = i32::try_from(index).ok()?;
        let color_width = if tool == AnnotationTool::Mosaic {
            0
        } else {
            COLOR_BUTTON_WIDTH * ANNOTATION_COLORS.len() as i32 + PARAMETER_GROUP_GAP
        };
        let left = bounds.left + PARAMETER_PADDING + color_width + index * SIZE_BUTTON_WIDTH;
        PhysicalRect::new(
            left,
            bounds.top + PARAMETER_PADDING,
            left + SIZE_BUTTON_WIDTH,
            bounds.bottom - PARAMETER_PADDING,
        )
        .ok()
    }

    fn toolbar_parameter_at(
        context: &OverlayWindowContext,
        selection: PhysicalRect,
        tool: AnnotationTool,
        point: PhysicalPoint,
    ) -> Option<ParameterItem> {
        let monitor = context.snapshot.frames[context.monitor_index]
            .monitor
            .physical_bounds;
        let toolbar = toolbar_bounds(monitor, selection)?;
        let bounds = parameter_bounds(monitor, toolbar, tool)?;
        if tool != AnnotationTool::Mosaic {
            if let Some(color) = ANNOTATION_COLORS.iter().enumerate().find_map(|(index, _)| {
                parameter_color_rect(bounds, index)
                    .filter(|rect| contains(*rect, point))
                    .map(|_| ParameterItem::Color(index))
            }) {
                return Some(color);
            }
        }
        parameter_values(tool)
            .iter()
            .enumerate()
            .find_map(|(index, value)| {
                parameter_size_rect(bounds, index, tool)
                    .filter(|rect| contains(*rect, point))
                    .map(|_| ParameterItem::Size(*value))
            })
    }

    fn toolbar_item_at(
        context: &OverlayWindowContext,
        selection: PhysicalRect,
        point: PhysicalPoint,
    ) -> Option<ToolbarItem> {
        let monitor_bounds = context.snapshot.frames[context.monitor_index]
            .monitor
            .physical_bounds;
        let bounds = toolbar_bounds(monitor_bounds, selection)?;
        TOOLBAR_ITEMS
            .iter()
            .enumerate()
            .find_map(|(index, (item, _, _))| {
                toolbar_button_rect(bounds, index)
                    .filter(|rect| contains(*rect, point))
                    .map(|_| *item)
            })
    }

    fn draw_toolbar(
        device_context: HDC,
        frame: &MonitorFrame,
        selection: PhysicalRect,
        state: ToolbarRenderState,
    ) {
        let monitor_bounds = frame.monitor.physical_bounds;
        let Some(bounds) = toolbar_bounds(monitor_bounds, selection) else {
            return;
        };
        let local_bounds = to_local_rect(bounds, monitor_bounds);
        draw_floating_surface(device_context, &local_bounds, TOOLBAR_RADIUS);
        draw_toolbar_dividers(device_context, bounds, monitor_bounds);

        for (index, (item, icon, _tooltip)) in TOOLBAR_ITEMS.iter().enumerate() {
            let Some(button) = toolbar_button_rect(bounds, index) else {
                continue;
            };
            let local = to_local_rect(button, monitor_bounds);
            let selected = match item {
                ToolbarItem::Select => state.active_tool.is_none(),
                ToolbarItem::Tool(tool) => state.active_tool == Some(*tool),
                _ => false,
            };
            let primary = matches!(item, ToolbarItem::Action(CaptureAction::Finish));
            let enabled = match item {
                ToolbarItem::Undo => state.can_undo,
                ToolbarItem::Redo => state.can_redo,
                _ => true,
            };
            if selected {
                draw_rounded_rect(
                    device_context,
                    &local,
                    TOOLBAR_BUTTON_RADIUS,
                    TOOLBAR_SELECTED,
                    ACCENT_COLOR,
                );
            } else if primary {
                draw_rounded_rect(
                    device_context,
                    &local,
                    TOOLBAR_BUTTON_RADIUS,
                    ACCENT_COLOR,
                    ACCENT_COLOR,
                );
            } else if state.hovered_item == Some(*item) && enabled {
                draw_rounded_rect(
                    device_context,
                    &local,
                    TOOLBAR_BUTTON_RADIUS,
                    TOOLBAR_HOVER,
                    TOOLBAR_HOVER,
                );
            }
            let icon_color = if !enabled {
                TOOLBAR_DISABLED
            } else if primary {
                WHITE
            } else if selected {
                ACCENT_COLOR
            } else {
                TOOLBAR_ICON
            };
            draw_toolbar_icon(device_context, &local, *icon, icon_color);
        }
        if let Some(tool) = state.active_tool {
            draw_parameter_bar(
                device_context,
                monitor_bounds,
                bounds,
                tool,
                state.annotation_style,
                state.hovered_parameter,
            );
        }
        if let Some(item) = state.hovered_item {
            draw_toolbar_tooltip(device_context, monitor_bounds, bounds, item);
        }
    }

    fn draw_toolbar_dividers(device_context: HDC, toolbar: PhysicalRect, monitor: PhysicalRect) {
        for group_start in TOOLBAR_GROUP_BREAKS {
            let Some(previous) = toolbar_button_rect(toolbar, group_start - 1) else {
                continue;
            };
            let Some(next) = toolbar_button_rect(toolbar, group_start) else {
                continue;
            };
            let x = (previous.right + next.left) / 2 - monitor.left;
            draw_colored_line(
                device_context,
                PhysicalPoint::new(x, toolbar.top - monitor.top + 9),
                PhysicalPoint::new(x, toolbar.bottom - monitor.top - 9),
                TOOLBAR_DIVIDER,
                1,
            );
        }
    }

    fn draw_toolbar_icon(device_context: HDC, bounds: &RECT, icon: ToolbarIcon, color: COLORREF) {
        let center_x = (bounds.left + bounds.right) / 2;
        let center_y = (bounds.top + bounds.bottom) / 2;
        let left = center_x - 9;
        let right = center_x + 9;
        let top = center_y - 9;
        let bottom = center_y + 9;
        let draw_gdi_line = |_: HDC, start: PhysicalPoint, end: PhysicalPoint| {
            draw_colored_line(device_context, start, end, color, 2);
        };
        let pen = unsafe { CreatePen(PS_SOLID, 2, color) } as HGDIOBJ;
        if pen.is_null() {
            return;
        }
        let previous = unsafe { SelectObject(device_context, pen) };
        match icon {
            ToolbarIcon::Select => {
                for (start, end) in [
                    (
                        PhysicalPoint::new(left + 3, top + 2),
                        PhysicalPoint::new(left + 3, bottom - 3),
                    ),
                    (
                        PhysicalPoint::new(left + 3, top + 2),
                        PhysicalPoint::new(right - 3, center_y + 3),
                    ),
                    (
                        PhysicalPoint::new(right - 3, center_y + 3),
                        PhysicalPoint::new(center_x, center_y + 5),
                    ),
                    (
                        PhysicalPoint::new(center_x, center_y + 5),
                        PhysicalPoint::new(center_x + 5, bottom - 2),
                    ),
                ] {
                    draw_gdi_line(device_context, start, end);
                }
            }
            ToolbarIcon::Rectangle => {
                frame_rect(
                    device_context,
                    &RECT {
                        left: left + 1,
                        top: top + 3,
                        right: right - 1,
                        bottom: bottom - 3,
                    },
                    color,
                );
            }
            ToolbarIcon::Arrow => {
                draw_gdi_line(
                    device_context,
                    PhysicalPoint::new(left + 2, bottom - 3),
                    PhysicalPoint::new(right - 2, top + 3),
                );
                draw_gdi_line(
                    device_context,
                    PhysicalPoint::new(right - 8, top + 3),
                    PhysicalPoint::new(right - 2, top + 3),
                );
                draw_gdi_line(
                    device_context,
                    PhysicalPoint::new(right - 2, top + 3),
                    PhysicalPoint::new(right - 2, top + 9),
                );
            }
            ToolbarIcon::Pen => {
                draw_gdi_line(
                    device_context,
                    PhysicalPoint::new(left + 3, bottom - 3),
                    PhysicalPoint::new(right - 4, top + 4),
                );
                draw_gdi_line(
                    device_context,
                    PhysicalPoint::new(left + 2, bottom - 2),
                    PhysicalPoint::new(left + 7, bottom - 4),
                );
                draw_gdi_line(
                    device_context,
                    PhysicalPoint::new(right - 7, top + 4),
                    PhysicalPoint::new(right - 4, top + 7),
                );
            }
            ToolbarIcon::Text => {
                let wide = [b'T' as u16];
                let font = create_text_font(18);
                let previous_font =
                    (!font.is_null()).then(|| unsafe { SelectObject(device_context, font) });
                let mut text_rect = *bounds;
                unsafe {
                    let _ = SetBkMode(device_context, TRANSPARENT as i32);
                    let _ = SetTextColor(device_context, color);
                    let _ = DrawTextW(
                        device_context,
                        wide.as_ptr(),
                        wide.len() as i32,
                        &mut text_rect,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                    );
                    if let Some(previous_font) = previous_font {
                        if !previous_font.is_null() && previous_font != HGDI_ERROR {
                            let _ = SelectObject(device_context, previous_font);
                        }
                    }
                    if !font.is_null() {
                        let _ = DeleteObject(font);
                    }
                }
            }
            ToolbarIcon::Mosaic => {
                for row in 0..2 {
                    for column in 0..2 {
                        let cell_left = left + 2 + column * 8;
                        let cell_top = top + 2 + row * 8;
                        fill_rect(
                            device_context,
                            &RECT {
                                left: cell_left,
                                top: cell_top,
                                right: cell_left + 6,
                                bottom: cell_top + 6,
                            },
                            color,
                        );
                    }
                }
            }
            ToolbarIcon::Undo | ToolbarIcon::Redo => {
                let direction = if icon == ToolbarIcon::Undo { -1 } else { 1 };
                let tip_x = center_x + direction * 7;
                let wing_x = tip_x - direction * 5;
                let tail_x = center_x - direction * 7;
                draw_gdi_line(
                    device_context,
                    PhysicalPoint::new(tip_x, center_y),
                    PhysicalPoint::new(wing_x, center_y - 5),
                );
                draw_gdi_line(
                    device_context,
                    PhysicalPoint::new(tip_x, center_y),
                    PhysicalPoint::new(wing_x, center_y + 5),
                );
                draw_gdi_line(
                    device_context,
                    PhysicalPoint::new(tip_x, center_y),
                    PhysicalPoint::new(tail_x, center_y),
                );
                draw_gdi_line(
                    device_context,
                    PhysicalPoint::new(tail_x, center_y),
                    PhysicalPoint::new(tail_x, center_y + 5),
                );
            }
            ToolbarIcon::Copy => {
                frame_rect(
                    device_context,
                    &RECT {
                        left: left + 5,
                        top: top + 2,
                        right: right - 1,
                        bottom: bottom - 4,
                    },
                    color,
                );
                frame_rect(
                    device_context,
                    &RECT {
                        left: left + 1,
                        top: top + 6,
                        right: right - 5,
                        bottom,
                    },
                    color,
                );
            }
            ToolbarIcon::Save => {
                frame_rect(
                    device_context,
                    &RECT {
                        left: left + 2,
                        top: top + 1,
                        right: right - 2,
                        bottom: bottom - 1,
                    },
                    color,
                );
                frame_rect(
                    device_context,
                    &RECT {
                        left: left + 5,
                        top: top + 2,
                        right: right - 6,
                        bottom: center_y - 1,
                    },
                    color,
                );
                frame_rect(
                    device_context,
                    &RECT {
                        left: left + 5,
                        top: center_y + 3,
                        right: right - 5,
                        bottom: bottom - 2,
                    },
                    color,
                );
            }
            ToolbarIcon::Qr => {
                for rect in [
                    RECT {
                        left: left + 1,
                        top: top + 1,
                        right: center_x - 1,
                        bottom: center_y - 1,
                    },
                    RECT {
                        left: center_x + 1,
                        top: top + 1,
                        right: right - 1,
                        bottom: center_y - 1,
                    },
                    RECT {
                        left: left + 1,
                        top: center_y + 1,
                        right: center_x - 1,
                        bottom: bottom - 1,
                    },
                ] {
                    frame_rect(device_context, &rect, color);
                }
                fill_rect(
                    device_context,
                    &RECT {
                        left: center_x + 2,
                        top: center_y + 2,
                        right: center_x + 6,
                        bottom: center_y + 6,
                    },
                    color,
                );
                fill_rect(
                    device_context,
                    &RECT {
                        left: right - 5,
                        top: bottom - 5,
                        right,
                        bottom,
                    },
                    color,
                );
            }
            ToolbarIcon::Pin => {
                draw_gdi_line(
                    device_context,
                    PhysicalPoint::new(left + 5, top + 3),
                    PhysicalPoint::new(right - 4, bottom - 6),
                );
                draw_gdi_line(
                    device_context,
                    PhysicalPoint::new(left + 3, top + 7),
                    PhysicalPoint::new(left + 8, top + 2),
                );
                draw_gdi_line(
                    device_context,
                    PhysicalPoint::new(right - 8, bottom - 2),
                    PhysicalPoint::new(right - 2, bottom - 8),
                );
                draw_gdi_line(
                    device_context,
                    PhysicalPoint::new(center_x + 4, center_y + 4),
                    PhysicalPoint::new(left + 2, bottom - 1),
                );
            }
            ToolbarIcon::Finish => {
                draw_gdi_line(
                    device_context,
                    PhysicalPoint::new(left + 2, center_y),
                    PhysicalPoint::new(center_x - 2, bottom - 4),
                );
                draw_gdi_line(
                    device_context,
                    PhysicalPoint::new(center_x - 2, bottom - 4),
                    PhysicalPoint::new(right - 2, top + 3),
                );
            }
        }
        unsafe {
            if !previous.is_null() && previous != HGDI_ERROR {
                let _ = SelectObject(device_context, previous);
            }
            let _ = DeleteObject(pen);
        }
    }

    fn draw_parameter_bar(
        device_context: HDC,
        monitor: PhysicalRect,
        toolbar: PhysicalRect,
        tool: AnnotationTool,
        style: AnnotationStyle,
        hovered: Option<ParameterItem>,
    ) {
        let Some(bounds) = parameter_bounds(monitor, toolbar, tool) else {
            return;
        };
        let local_bounds = to_local_rect(bounds, monitor);
        draw_floating_surface(device_context, &local_bounds, TOOLBAR_RADIUS);

        if tool != AnnotationTool::Mosaic {
            for (index, color) in ANNOTATION_COLORS.iter().enumerate() {
                let Some(button) = parameter_color_rect(bounds, index) else {
                    continue;
                };
                let local = to_local_rect(button, monitor);
                if hovered == Some(ParameterItem::Color(index)) {
                    draw_rounded_rect(
                        device_context,
                        &local,
                        TOOLBAR_BUTTON_RADIUS,
                        TOOLBAR_HOVER,
                        TOOLBAR_HOVER,
                    );
                }
                let inset = RECT {
                    left: local.left + 5,
                    top: local.top + 5,
                    right: local.right - 5,
                    bottom: local.bottom - 5,
                };
                draw_ellipse(
                    device_context,
                    &inset,
                    rgb_to_colorref(*color),
                    if style.color == *color {
                        ACCENT_COLOR
                    } else {
                        TOOLBAR_BORDER
                    },
                );
            }
        }

        for (index, value) in parameter_values(tool).iter().enumerate() {
            let Some(button) = parameter_size_rect(bounds, index, tool) else {
                continue;
            };
            let local = to_local_rect(button, monitor);
            let selected = match tool {
                AnnotationTool::Mosaic => style.mosaic_block == *value,
                AnnotationTool::Text => style.font_size == *value,
                AnnotationTool::Rectangle | AnnotationTool::Arrow | AnnotationTool::Pen => {
                    style.thickness == *value
                }
            };
            if selected {
                draw_rounded_rect(
                    device_context,
                    &local,
                    TOOLBAR_BUTTON_RADIUS,
                    TOOLBAR_SELECTED,
                    ACCENT_COLOR,
                );
            } else if hovered == Some(ParameterItem::Size(*value)) {
                draw_rounded_rect(
                    device_context,
                    &local,
                    TOOLBAR_BUTTON_RADIUS,
                    TOOLBAR_HOVER,
                    TOOLBAR_HOVER,
                );
            }
            match tool {
                AnnotationTool::Mosaic => {
                    draw_mosaic_size_sample(device_context, &local, *value);
                }
                AnnotationTool::Text => {
                    draw_text_size_sample(device_context, &local, *value, style.color);
                }
                AnnotationTool::Rectangle | AnnotationTool::Arrow | AnnotationTool::Pen => {
                    draw_thickness_sample(device_context, &local, *value, style.color);
                }
            }
        }
    }

    fn draw_thickness_sample(device_context: HDC, bounds: &RECT, thickness: u8, color: [u8; 3]) {
        let height = i32::from(thickness.clamp(1, 10));
        let sample = RECT {
            left: bounds.left + 7,
            right: bounds.right - 7,
            top: (bounds.top + bounds.bottom - height) / 2,
            bottom: (bounds.top + bounds.bottom + height) / 2,
        };
        fill_rect(device_context, &sample, rgb_to_colorref(color));
    }

    fn draw_mosaic_size_sample(device_context: HDC, bounds: &RECT, block: u8) {
        let size = i32::from(block.clamp(6, 18));
        let left = (bounds.left + bounds.right - size) / 2;
        let top = (bounds.top + bounds.bottom - size) / 2;
        let sample = RECT {
            left,
            top,
            right: left + size,
            bottom: top + size,
        };
        fill_rect(device_context, &sample, 0x00989898);
        frame_rect(device_context, &sample, 0x00606060);
    }

    fn draw_text_size_sample(device_context: HDC, bounds: &RECT, font_size: u8, color: [u8; 3]) {
        let text = font_size.to_string();
        let wide: Vec<u16> = text.encode_utf16().collect();
        let font = create_text_font(13);
        let previous = (!font.is_null()).then(|| unsafe { SelectObject(device_context, font) });
        let mut rect = *bounds;
        unsafe {
            let _ = SetBkMode(device_context, TRANSPARENT as i32);
            let _ = SetTextColor(device_context, rgb_to_colorref(color));
            let _ = DrawTextW(
                device_context,
                wide.as_ptr(),
                wide.len() as i32,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
            if let Some(previous) = previous {
                if !previous.is_null() && previous != HGDI_ERROR {
                    let _ = SelectObject(device_context, previous);
                }
            }
            if !font.is_null() {
                let _ = DeleteObject(font);
            }
        }
    }

    fn rgb_to_colorref(color: [u8; 3]) -> COLORREF {
        u32::from(color[0]) | (u32::from(color[1]) << 8) | (u32::from(color[2]) << 16)
    }

    fn create_ui_font() -> HGDIOBJ {
        create_text_font(13)
    }

    fn create_text_font(size: u8) -> HGDIOBJ {
        let face: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
        unsafe {
            CreateFontW(
                -i32::from(size.clamp(10, 72)),
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                (DEFAULT_PITCH | FF_DONTCARE) as u32,
                face.as_ptr(),
            ) as HGDIOBJ
        }
    }

    fn draw_toolbar_tooltip(
        device_context: HDC,
        monitor: PhysicalRect,
        toolbar: PhysicalRect,
        item: ToolbarItem,
    ) {
        let Some((_, _, tooltip)) = TOOLBAR_ITEMS.iter().find(|(entry, _, _)| *entry == item)
        else {
            return;
        };
        let width = (tooltip.encode_utf16().count() as i32 * 14 + 20).clamp(90, 220);
        let height = 28;
        let button = TOOLBAR_ITEMS
            .iter()
            .position(|(entry, _, _)| *entry == item)
            .and_then(|index| toolbar_button_rect(toolbar, index));
        let preferred_left = button
            .map(|button| (button.left + button.right - width) / 2)
            .unwrap_or(toolbar.right.saturating_sub(width));
        let left = preferred_left.clamp(monitor.left, monitor.right - width);
        let below = toolbar.bottom + 5;
        let top = if below + height <= monitor.bottom {
            below
        } else {
            toolbar.top - height - 5
        };
        let bounds = PhysicalRect::new(left, top, left + width, top + height).ok();
        let Some(bounds) = bounds else {
            return;
        };
        let mut local = to_local_rect(bounds, monitor);
        draw_rounded_rect(
            device_context,
            &local,
            TOOLBAR_BUTTON_RADIUS,
            TOOLTIP_BACKGROUND,
            TOOLTIP_BACKGROUND,
        );
        let font = create_ui_font();
        let previous_font = (!font.is_null())
            .then(|| unsafe { SelectObject(device_context, font) })
            .filter(|selected| !selected.is_null() && *selected != HGDI_ERROR);
        let wide: Vec<u16> = tooltip.encode_utf16().collect();
        unsafe {
            let _ = SetBkMode(device_context, TRANSPARENT as i32);
            let _ = SetTextColor(device_context, WHITE);
            let _ = DrawTextW(
                device_context,
                wide.as_ptr(),
                wide.len() as i32,
                &mut local,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
        }
        if let Some(previous_font) = previous_font {
            unsafe {
                let _ = SelectObject(device_context, previous_font);
            }
        }
        if !font.is_null() {
            unsafe {
                let _ = DeleteObject(font);
            }
        }
    }

    fn contains(rect: PhysicalRect, point: PhysicalPoint) -> bool {
        point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
    }

    fn to_local_rect(rect: PhysicalRect, monitor: PhysicalRect) -> RECT {
        RECT {
            left: rect.left - monitor.left,
            top: rect.top - monitor.top,
            right: rect.right - monitor.left,
            bottom: rect.bottom - monitor.top,
        }
    }

    fn draw_floating_surface(device_context: HDC, rect: &RECT, radius: i32) {
        let shadow = RECT {
            left: rect.left + 1,
            top: rect.top + 1,
            right: rect.right + 1,
            bottom: rect.bottom + 1,
        };
        draw_rounded_rect(
            device_context,
            &shadow,
            radius,
            TOOLBAR_SHADOW,
            TOOLBAR_SHADOW,
        );
        draw_rounded_rect(
            device_context,
            rect,
            radius,
            TOOLBAR_BACKGROUND,
            TOOLBAR_BORDER,
        );
    }

    fn draw_rounded_rect(
        device_context: HDC,
        rect: &RECT,
        radius: i32,
        fill: COLORREF,
        border: COLORREF,
    ) {
        let Some(graphics) = GdiPlusGraphics::from_hdc(device_context) else {
            fill_rect(device_context, rect, fill);
            frame_rect(device_context, rect, border);
            return;
        };
        let width = rect.right.saturating_sub(rect.left).max(1);
        let height = rect.bottom.saturating_sub(rect.top).max(1);
        let radius = radius.clamp(1, width / 2).clamp(1, height / 2);
        let diameter = radius.saturating_mul(2);
        let mut path: *mut GpPath = null_mut();
        unsafe {
            if GdipCreatePath(FillModeWinding, &mut path) != 0 || path.is_null() {
                return;
            }
            let right = rect.right - diameter - 1;
            let bottom = rect.bottom - diameter - 1;
            let _ = GdipAddPathArcI(path, rect.left, rect.top, diameter, diameter, 180.0, 90.0);
            let _ = GdipAddPathArcI(path, right, rect.top, diameter, diameter, 270.0, 90.0);
            let _ = GdipAddPathArcI(path, right, bottom, diameter, diameter, 0.0, 90.0);
            let _ = GdipAddPathArcI(path, rect.left, bottom, diameter, diameter, 90.0, 90.0);
            let _ = GdipClosePathFigure(path);
            let mut brush: *mut GpSolidFill = null_mut();
            if GdipCreateSolidFill(colorref_to_argb(fill), &mut brush) == 0 && !brush.is_null() {
                let _ = GdipFillPath(graphics.0, brush.cast::<GpBrush>(), path);
                let _ = GdipDeleteBrush(brush.cast::<GpBrush>());
            }
            let mut pen: *mut GpPen = null_mut();
            if GdipCreatePen1(colorref_to_argb(border), 1.0, UnitPixel, &mut pen) == 0
                && !pen.is_null()
            {
                let _ = GdipDrawPath(graphics.0, pen, path);
                let _ = GdipDeletePen(pen);
            }
            let _ = GdipDeletePath(path);
        }
    }

    fn draw_ellipse(device_context: HDC, rect: &RECT, fill: COLORREF, border: COLORREF) {
        let Some(graphics) = GdiPlusGraphics::from_hdc(device_context) else {
            fill_rect(device_context, rect, fill);
            frame_rect(device_context, rect, border);
            return;
        };
        let width = rect.right.saturating_sub(rect.left).max(1);
        let height = rect.bottom.saturating_sub(rect.top).max(1);
        unsafe {
            let mut brush: *mut GpSolidFill = null_mut();
            if GdipCreateSolidFill(colorref_to_argb(fill), &mut brush) == 0 && !brush.is_null() {
                let _ = GdipFillEllipseI(
                    graphics.0,
                    brush.cast::<GpBrush>(),
                    rect.left,
                    rect.top,
                    width,
                    height,
                );
                let _ = GdipDeleteBrush(brush.cast::<GpBrush>());
            }
            let mut pen: *mut GpPen = null_mut();
            if GdipCreatePen1(colorref_to_argb(border), 1.0, UnitPixel, &mut pen) == 0
                && !pen.is_null()
            {
                let _ =
                    GdipDrawEllipseI(graphics.0, pen, rect.left, rect.top, width - 1, height - 1);
                let _ = GdipDeletePen(pen);
            }
        }
    }

    fn draw_colored_line(
        device_context: HDC,
        start: PhysicalPoint,
        end: PhysicalPoint,
        color: COLORREF,
        width: i32,
    ) {
        if let Some(graphics) = GdiPlusGraphics::from_hdc(device_context) {
            let mut pen: *mut GpPen = null_mut();
            unsafe {
                if GdipCreatePen1(
                    colorref_to_argb(color),
                    width.max(1) as f32,
                    UnitPixel,
                    &mut pen,
                ) == 0
                    && !pen.is_null()
                {
                    let _ = GdipDrawLineI(graphics.0, pen, start.x, start.y, end.x, end.y);
                    let _ = GdipDeletePen(pen);
                    return;
                }
            }
        }
        let pen = unsafe { CreatePen(PS_SOLID, width, color) } as HGDIOBJ;
        if pen.is_null() {
            return;
        }
        let previous = unsafe { SelectObject(device_context, pen) };
        draw_gdi_line(device_context, start, end);
        unsafe {
            if !previous.is_null() && previous != HGDI_ERROR {
                let _ = SelectObject(device_context, previous);
            }
            let _ = DeleteObject(pen);
        }
    }

    fn fill_rect(device_context: HDC, rect: &RECT, color: COLORREF) {
        let brush = unsafe { CreateSolidBrush(color) };
        if brush.is_null() {
            return;
        }
        unsafe {
            let _ = FillRect(device_context, rect, brush);
            let _ = DeleteObject(brush as HGDIOBJ);
        }
    }

    fn frame_rect(device_context: HDC, rect: &RECT, color: COLORREF) {
        if let Some(graphics) = GdiPlusGraphics::from_hdc(device_context) {
            let mut pen: *mut GpPen = null_mut();
            unsafe {
                if GdipCreatePen1(colorref_to_argb(color), 1.0, UnitPixel, &mut pen) == 0
                    && !pen.is_null()
                {
                    let _ = GdipDrawRectangleI(
                        graphics.0,
                        pen,
                        rect.left,
                        rect.top,
                        rect.right.saturating_sub(rect.left).saturating_sub(1),
                        rect.bottom.saturating_sub(rect.top).saturating_sub(1),
                    );
                    let _ = GdipDeletePen(pen);
                    return;
                }
            }
        }
        let brush = unsafe { CreateSolidBrush(color) };
        if brush.is_null() {
            return;
        }
        unsafe {
            let _ = FrameRect(device_context, rect, brush);
            let _ = DeleteObject(brush as HGDIOBJ);
        }
    }

    fn colorref_to_argb(color: COLORREF) -> u32 {
        let red = color & 0xff;
        let green = (color >> 8) & 0xff;
        let blue = (color >> 16) & 0xff;
        0xff00_0000 | (red << 16) | (green << 8) | blue
    }

    fn bitmap_info(width: u32, height: u32) -> BITMAPINFO {
        let mut info: BITMAPINFO = unsafe { zeroed() };
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            biSizeImage: width.saturating_mul(height).saturating_mul(4),
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        };
        info
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn bitmap_info_describes_top_down_bgra() {
            let info = bitmap_info(320, 200);
            assert_eq!(info.bmiHeader.biWidth, 320);
            assert_eq!(info.bmiHeader.biHeight, -200);
            assert_eq!(info.bmiHeader.biBitCount, 32);
        }

        #[test]
        fn dimmed_pixels_keep_alpha_and_apply_a_visible_black_overlay() {
            assert_eq!(
                dimmed_bgra(&[255, 128, 64, 255, 0, 10, 200, 128], 116),
                vec![139, 70, 35, 255, 0, 5, 109, 128]
            );
        }

        #[test]
        fn toolbar_prefers_below_and_flips_above_without_leaving_monitor() {
            let monitor = PhysicalRect::new(-1920, 0, 0, 1080).unwrap();
            let middle = PhysicalRect::new(-1500, 200, -800, 500).unwrap();
            let below = toolbar_bounds(monitor, middle).unwrap();
            assert_eq!(below.top, middle.bottom + TOOLBAR_GAP);
            assert!(below.left >= monitor.left && below.right <= monitor.right);

            let near_bottom = PhysicalRect::new(-1500, 800, -800, 1060).unwrap();
            let above = toolbar_bounds(monitor, near_bottom).unwrap();
            assert_eq!(above.bottom, near_bottom.top - TOOLBAR_GAP);
            assert!(above.top >= monitor.top && above.bottom <= monitor.bottom);
        }

        #[test]
        fn toolbar_buttons_keep_action_order_and_separate_semantic_groups() {
            let bounds = PhysicalRect::new(100, 100, 564, 140).unwrap();
            for index in 0..TOOLBAR_ITEMS.len() {
                let button = toolbar_button_rect(bounds, index).unwrap();
                assert_eq!(button.width(), Some(TOOLBAR_BUTTON_WIDTH as u32));
                if index > 0 {
                    let expected_gap = if TOOLBAR_GROUP_BREAKS.contains(&index) {
                        TOOLBAR_GROUP_GAP
                    } else {
                        0
                    };
                    assert_eq!(
                        button.left - toolbar_button_rect(bounds, index - 1).unwrap().right,
                        expected_gap
                    );
                }
            }
            assert_eq!(
                toolbar_button_rect(bounds, TOOLBAR_ITEMS.len() - 1)
                    .unwrap()
                    .right
                    + TOOLBAR_PADDING,
                bounds.right
            );
        }

        #[test]
        fn active_tool_parameter_bar_stays_on_monitor_and_separates_groups() {
            let monitor = PhysicalRect::new(0, 0, 1920, 1080).unwrap();
            let selection = PhysicalRect::new(300, 200, 900, 600).unwrap();
            let toolbar = toolbar_bounds(monitor, selection).unwrap();
            let line_bounds = parameter_bounds(monitor, toolbar, AnnotationTool::Arrow).unwrap();
            let mosaic_bounds = parameter_bounds(monitor, toolbar, AnnotationTool::Mosaic).unwrap();
            assert!(contains(
                monitor,
                PhysicalPoint::new(line_bounds.left, line_bounds.top)
            ));
            assert!(contains(
                monitor,
                PhysicalPoint::new(line_bounds.right - 1, line_bounds.bottom - 1)
            ));
            assert!(line_bounds.width() > mosaic_bounds.width());
            assert!(
                parameter_color_rect(line_bounds, ANNOTATION_COLORS.len() - 1)
                    .unwrap()
                    .right
                    < parameter_size_rect(line_bounds, 0, AnnotationTool::Arrow)
                        .unwrap()
                        .left
            );
        }

        #[test]
        fn annotation_hit_prefers_visible_geometry_and_text_bounds() {
            let mut rectangle =
                Annotation::new(AnnotationTool::Rectangle, PhysicalPoint::new(100, 100));
            rectangle.update(PhysicalPoint::new(240, 180));
            let text = Annotation::text(
                PhysicalPoint::new(300, 200),
                "可移动文字".to_owned(),
                AnnotationStyle::default(),
            );
            assert!(annotation_hit(&rectangle, PhysicalPoint::new(160, 140)));
            assert!(annotation_hit(&text, PhysicalPoint::new(320, 210)));
            assert!(!annotation_hit(&text, PhysicalPoint::new(100, 300)));
        }

        #[test]
        fn annotation_drag_delta_stays_inside_selection() {
            let selection = PhysicalRect::new(0, 0, 200, 100).unwrap();
            let points = [PhysicalPoint::new(20, 30), PhysicalPoint::new(80, 70)];
            assert_eq!(
                clamped_annotation_delta(&points, -100, 100, selection),
                (-20, 29)
            );
        }
    }
}
