use std::collections::HashMap;
use std::mem::{size_of, zeroed};
use std::ptr::{copy_nonoverlapping, null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use thiserror::Error;
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateFontW, CreateSolidBrush, DeleteDC, DeleteObject,
    DrawTextW, FillRect, GetMonitorInfoW, MonitorFromPoint, SelectObject, SetBkMode, SetTextColor,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION,
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DIB_RGB_COLORS, DT_CALCRECT,
    DT_EDITCONTROL, DT_END_ELLIPSIS, DT_NOPREFIX, DT_WORDBREAK, HGDIOBJ, MONITORINFO,
    MONITOR_DEFAULTTONEAREST, OUT_DEFAULT_PRECIS, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, GetWindowLongPtrW, IsWindow, LoadCursorW,
    PeekMessageW, PostMessageW, PostQuitMessage, PostThreadMessageW, RegisterClassW,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, TrackPopupMenu, TranslateMessage,
    UnregisterClassW, UpdateLayeredWindow, CREATESTRUCTW, CS_DBLCLKS, GWLP_USERDATA, HTCLIENT,
    HTTRANSPARENT, HWND_TOPMOST, IDC_ARROW, MA_NOACTIVATE, MF_SEPARATOR, MF_STRING, MSG,
    PM_NOREMOVE, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOSIZE, SW_SHOWNOACTIVATE, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, ULW_ALPHA, WM_APP, WM_CAPTURECHANGED, WM_CLOSE, WM_DESTROY, WM_DISPLAYCHANGE,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE,
    WM_NCDESTROY, WM_NCHITTEST, WM_RBUTTONUP, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::infrastructure::clipboard::ClipboardWriteContent;
use crate::infrastructure::clipboard_listener::ClipboardSequenceSuppressor;
use crate::infrastructure::clipboard_writer::ClipboardWriter;
use crate::infrastructure::image_output::save_rgba_with_dialog;
use crate::infrastructure::keyboard_hook::KeyboardHookBroker;

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
    b'P' as u16,
    b'i' as u16,
    b'n' as u16,
    b'I' as u16,
    b'm' as u16,
    b'a' as u16,
    b'g' as u16,
    b'e' as u16,
    0,
];
const RUNTIME_MESSAGE: u32 = WM_APP + 0x31;
const MENU_COMMAND_MESSAGE: u32 = WM_APP + 0x32;
const COMMAND_CAPACITY: usize = 32;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(3);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_PIN_MEMORY_BYTES: usize = 256 * 1024 * 1024;
const TEXT_CARD_WIDTH: i32 = 480;
const TEXT_CARD_MAX_HEIGHT: i32 = 640;
const TEXT_CARD_MIN_HEIGHT: i32 = 120;
const TEXT_CARD_PADDING: i32 = 24;
const TEXT_CARD_CORNER_RADIUS: i32 = 14;
const TEXT_CARD_BACKGROUND: u32 = 0x00F8_F8F8;
const TEXT_CARD_FOREGROUND: u32 = 0x0024_2424;
const MIN_VISIBLE_PIXELS: i32 = 32;
const MIN_SHORT_EDGE: f64 = 64.0;
const MAX_WORK_AREA_RATIO: f64 = 0.9;
const MENU_COPY_IMAGE: usize = 1;
const MENU_COPY_TEXT: usize = 2;
const MENU_SAVE: usize = 3;
const MENU_ORIGINAL_SIZE: usize = 4;
const MENU_OPACITY_100: usize = 5;
const MENU_OPACITY_75: usize = 6;
const MENU_OPACITY_50: usize = 7;
const MENU_CLOSE: usize = 8;
const HGDI_ERROR: HGDIOBJ = -1isize as HGDIOBJ;

#[derive(Debug, Clone)]
pub struct PinImageData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub source_text: Option<String>,
}

impl PinImageData {
    fn validate(&self) -> Result<(), NativeSurfaceError> {
        if self.width == 0 || self.height == 0 {
            return Err(NativeSurfaceError::InvalidImage);
        }
        let expected = usize::try_from(self.width)
            .ok()
            .and_then(|width| {
                usize::try_from(self.height)
                    .ok()
                    .and_then(|height| width.checked_mul(height))
            })
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(NativeSurfaceError::InvalidImage)?;
        if self.rgba.len() != expected || expected > MAX_PIN_MEMORY_BYTES {
            return Err(NativeSurfaceError::InvalidImage);
        }
        Ok(())
    }
}

pub fn render_text_card(text: &str) -> Result<PinImageData, NativeSurfaceError> {
    let source_text = text.to_owned();
    let mut display_text = text.chars().take(4_000).collect::<String>();
    if text.chars().count() > 4_000 {
        display_text.push_str("\n…");
    }
    if display_text.trim().is_empty() {
        display_text = "（空白文字）".to_owned();
    }
    let wide: Vec<u16> = display_text.encode_utf16().collect();
    let face: Vec<u16> = "Microsoft YaHei UI\0".encode_utf16().collect();
    let screen_dc = unsafe { windows_sys::Win32::Graphics::Gdi::GetDC(null_mut()) };
    if screen_dc.is_null() {
        return Err(NativeSurfaceError::WindowsApi("GetDC"));
    }
    let memory_dc = unsafe { CreateCompatibleDC(screen_dc) };
    if memory_dc.is_null() {
        unsafe {
            let _ = windows_sys::Win32::Graphics::Gdi::ReleaseDC(null_mut(), screen_dc);
        }
        return Err(NativeSurfaceError::WindowsApi("CreateCompatibleDC"));
    }
    let font = unsafe {
        CreateFontW(
            -20,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            0,
            face.as_ptr(),
        )
    };
    if font.is_null() {
        unsafe {
            let _ = DeleteDC(memory_dc);
            let _ = windows_sys::Win32::Graphics::Gdi::ReleaseDC(null_mut(), screen_dc);
        }
        return Err(NativeSurfaceError::WindowsApi("CreateFontW"));
    }
    let previous_font = unsafe { SelectObject(memory_dc, font as HGDIOBJ) };
    let mut measured = RECT {
        left: 0,
        top: 0,
        right: TEXT_CARD_WIDTH - TEXT_CARD_PADDING * 2,
        bottom: 0,
    };
    unsafe {
        let _ = DrawTextW(
            memory_dc,
            wide.as_ptr(),
            wide.len() as i32,
            &mut measured,
            DT_CALCRECT | DT_WORDBREAK | DT_EDITCONTROL | DT_NOPREFIX,
        );
    }
    let height =
        (measured.bottom + TEXT_CARD_PADDING * 2).clamp(TEXT_CARD_MIN_HEIGHT, TEXT_CARD_MAX_HEIGHT);
    let mut bits = null_mut();
    let info = bitmap_info(TEXT_CARD_WIDTH, height);
    let bitmap =
        unsafe { CreateDIBSection(memory_dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0) };
    if bitmap.is_null() || bits.is_null() {
        unsafe {
            let _ = SelectObject(memory_dc, previous_font);
            let _ = DeleteObject(font as HGDIOBJ);
            let _ = DeleteDC(memory_dc);
            let _ = windows_sys::Win32::Graphics::Gdi::ReleaseDC(null_mut(), screen_dc);
        }
        return Err(NativeSurfaceError::WindowsApi("CreateDIBSection"));
    }
    let previous_bitmap = unsafe { SelectObject(memory_dc, bitmap as HGDIOBJ) };
    let card = RECT {
        left: 0,
        top: 0,
        right: TEXT_CARD_WIDTH,
        bottom: height,
    };
    let background = unsafe { CreateSolidBrush(TEXT_CARD_BACKGROUND) };
    if !background.is_null() {
        unsafe {
            let _ = FillRect(memory_dc, &card, background);
            let _ = DeleteObject(background as HGDIOBJ);
        }
    }
    let mut text_rect = RECT {
        left: TEXT_CARD_PADDING,
        top: TEXT_CARD_PADDING,
        right: TEXT_CARD_WIDTH - TEXT_CARD_PADDING,
        bottom: height - TEXT_CARD_PADDING,
    };
    unsafe {
        let _ = SetBkMode(memory_dc, TRANSPARENT as i32);
        let _ = SetTextColor(memory_dc, TEXT_CARD_FOREGROUND);
        let _ = DrawTextW(
            memory_dc,
            wide.as_ptr(),
            wide.len() as i32,
            &mut text_rect,
            DT_WORDBREAK | DT_EDITCONTROL | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
    }
    let byte_count = usize::try_from(TEXT_CARD_WIDTH)
        .ok()
        .and_then(|width| width.checked_mul(usize::try_from(height).ok()?))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(NativeSurfaceError::InvalidImage)?;
    let bgra = unsafe { std::slice::from_raw_parts(bits.cast::<u8>(), byte_count) };
    let mut rgba = vec![0u8; byte_count];
    for (index, pixel) in bgra.chunks_exact(4).enumerate() {
        let offset = index * 4;
        rgba[offset] = pixel[2];
        rgba[offset + 1] = pixel[1];
        rgba[offset + 2] = pixel[0];
        rgba[offset + 3] = 255;
    }
    apply_rounded_alpha(
        &mut rgba,
        TEXT_CARD_WIDTH as u32,
        height as u32,
        TEXT_CARD_CORNER_RADIUS,
    );
    unsafe {
        let _ = SelectObject(memory_dc, previous_bitmap);
        let _ = SelectObject(memory_dc, previous_font);
        let _ = DeleteObject(bitmap as HGDIOBJ);
        let _ = DeleteObject(font as HGDIOBJ);
        let _ = DeleteDC(memory_dc);
        let _ = windows_sys::Win32::Graphics::Gdi::ReleaseDC(null_mut(), screen_dc);
    }
    let image = PinImageData {
        width: TEXT_CARD_WIDTH as u32,
        height: height as u32,
        rgba,
        source_text: Some(source_text),
    };
    image.validate()?;
    Ok(image)
}

fn apply_rounded_alpha(rgba: &mut [u8], width: u32, height: u32, radius: i32) {
    let width_i32 = width as i32;
    let height_i32 = height as i32;
    let radius_squared = radius * radius;
    for y in 0..height_i32 {
        for x in 0..width_i32 {
            let corner_x = if x < radius {
                radius - x
            } else if x >= width_i32 - radius {
                x - (width_i32 - radius - 1)
            } else {
                0
            };
            let corner_y = if y < radius {
                radius - y
            } else if y >= height_i32 - radius {
                y - (height_i32 - radius - 1)
            } else {
                0
            };
            if corner_x > 0
                && corner_y > 0
                && corner_x * corner_x + corner_y * corner_y > radius_squared
            {
                let offset = ((y as u32 * width + x as u32) * 4 + 3) as usize;
                rgba[offset] = 0;
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum NativeSurfaceError {
    #[error("native image surface startup failed: {0}")]
    Startup(String),
    #[error("native image surface command queue is unavailable")]
    CommandQueueUnavailable,
    #[error("native image surface command timed out")]
    CommandTimedOut,
    #[error("native image data is invalid or exceeds the memory budget")]
    InvalidImage,
    #[error("native image surface memory budget is exhausted")]
    MemoryBudgetExceeded,
    #[error("Windows operation failed: {0}")]
    WindowsApi(&'static str),
}

#[derive(Debug)]
pub struct NativeImageSurfaceRuntime {
    commands: SyncSender<RuntimeCommand>,
    thread_id: u32,
    accepting: AtomicBool,
    shutdown: std::sync::Arc<AtomicBool>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl NativeImageSurfaceRuntime {
    pub fn start(
        keyboard_hook: Arc<KeyboardHookBroker>,
        suppressor: ClipboardSequenceSuppressor,
    ) -> Result<Self, NativeSurfaceError> {
        let (commands, receiver) = sync_channel(COMMAND_CAPACITY);
        let (ready_tx, ready_rx) = sync_channel(1);
        let shutdown = std::sync::Arc::new(AtomicBool::new(false));
        let thread_shutdown = std::sync::Arc::clone(&shutdown);
        let join = thread::Builder::new()
            .name("native-image-surfaces".to_owned())
            .spawn(move || {
                runtime_thread(
                    receiver,
                    ready_tx,
                    thread_shutdown,
                    keyboard_hook,
                    suppressor,
                )
            })
            .map_err(|error| NativeSurfaceError::Startup(error.to_string()))?;
        let thread_id = match ready_rx.recv_timeout(STARTUP_TIMEOUT) {
            Ok(Ok(thread_id)) => thread_id,
            Ok(Err(error)) => {
                let _ = join.join();
                return Err(error);
            }
            Err(error) => {
                return Err(NativeSurfaceError::Startup(format!(
                    "runtime readiness timed out: {error}"
                )));
            }
        };
        Ok(Self {
            commands,
            thread_id,
            accepting: AtomicBool::new(true),
            shutdown,
            join: Mutex::new(Some(join)),
        })
    }

    pub fn open(&self, image: PinImageData) -> Result<u64, NativeSurfaceError> {
        image.validate()?;
        if !self.accepting.load(Ordering::Acquire) {
            return Err(NativeSurfaceError::CommandQueueUnavailable);
        }
        let (reply_tx, reply_rx) = sync_channel(1);
        self.commands
            .try_send(RuntimeCommand::Open {
                image,
                reply: reply_tx,
            })
            .map_err(|error| match error {
                TrySendError::Full(_) | TrySendError::Disconnected(_) => {
                    NativeSurfaceError::CommandQueueUnavailable
                }
            })?;
        post_runtime_message(self.thread_id)?;
        match reply_rx.recv_timeout(COMMAND_TIMEOUT) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => Err(NativeSurfaceError::CommandTimedOut),
            Err(RecvTimeoutError::Disconnected) => Err(NativeSurfaceError::CommandQueueUnavailable),
        }
    }
}

impl Drop for NativeImageSurfaceRuntime {
    fn drop(&mut self) {
        self.accepting.store(false, Ordering::Release);
        self.shutdown.store(true, Ordering::Release);
        let _ = post_runtime_message(self.thread_id);
        if let Ok(mut join) = self.join.lock() {
            if let Some(handle) = join.take() {
                let _ = handle.join();
            }
        }
    }
}

#[derive(Debug)]
enum RuntimeCommand {
    Open {
        image: PinImageData,
        reply: SyncSender<Result<u64, NativeSurfaceError>>,
    },
}

struct RuntimeState {
    module: HINSTANCE,
    keyboard_hook: Arc<KeyboardHookBroker>,
    clipboard_writer: Arc<ClipboardWriter>,
    suppressor: ClipboardSequenceSuppressor,
    windows: HashMap<isize, Box<PinWindowContext>>,
    next_pin_id: u64,
    image_memory_bytes: usize,
}

impl RuntimeState {
    fn new(
        module: HINSTANCE,
        keyboard_hook: Arc<KeyboardHookBroker>,
        suppressor: ClipboardSequenceSuppressor,
    ) -> Self {
        Self {
            module,
            keyboard_hook,
            clipboard_writer: Arc::new(ClipboardWriter::default()),
            suppressor,
            windows: HashMap::new(),
            next_pin_id: 1,
            image_memory_bytes: 0,
        }
    }

    fn open(&mut self, image: PinImageData) -> Result<u64, NativeSurfaceError> {
        let next_memory = self
            .image_memory_bytes
            .checked_add(image.rgba.len())
            .ok_or(NativeSurfaceError::MemoryBudgetExceeded)?;
        if next_memory > MAX_PIN_MEMORY_BYTES {
            return Err(NativeSurfaceError::MemoryBudgetExceeded);
        }
        let cursor = cursor_position().unwrap_or(POINT { x: 0, y: 0 });
        let work_area = monitor_work_area(cursor)?;
        let geometry = PinGeometry::initial(image.width, image.height, work_area, cursor)?;
        let pin_id = self.next_pin_id;
        self.next_pin_id = self.next_pin_id.wrapping_add(1).max(1);
        let mut context = Box::new(PinWindowContext {
            pin_id,
            image,
            geometry,
            opacity: 255,
            drag_offset: None,
            menu_anchor: None,
            keyboard_hook: std::sync::Arc::clone(&self.keyboard_hook),
            escape_generation: None,
            clipboard_writer: Arc::clone(&self.clipboard_writer),
            suppressor: self.suppressor.clone(),
        });
        let window = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                CLASS_NAME.as_ptr(),
                CLASS_NAME.as_ptr(),
                WS_POPUP,
                geometry.x,
                geometry.y,
                geometry.width,
                geometry.height,
                null_mut(),
                null_mut(),
                self.module,
                (&mut *context as *mut PinWindowContext).cast(),
            )
        };
        if window.is_null() {
            return Err(NativeSurfaceError::WindowsApi("CreateWindowExW"));
        }
        if let Err(error) = render_layered(window, &context) {
            unsafe {
                DestroyWindow(window);
            }
            return Err(error);
        }
        unsafe {
            ShowWindow(window, SW_SHOWNOACTIVATE);
        }
        self.image_memory_bytes = next_memory;
        self.windows.insert(window as isize, context);
        Ok(pin_id)
    }

    fn remove_destroyed_windows(&mut self) {
        let mut released = 0usize;
        self.windows.retain(|handle, context| {
            let alive = unsafe { IsWindow(*handle as HWND) } != 0;
            if !alive {
                released = released.saturating_add(context.image.rgba.len());
            }
            alive
        });
        self.image_memory_bytes = self.image_memory_bytes.saturating_sub(released);
    }

    fn destroy_all(&mut self) {
        let handles = self
            .windows
            .keys()
            .copied()
            .map(|handle| handle as HWND)
            .collect::<Vec<_>>();
        for handle in handles {
            unsafe {
                DestroyWindow(handle);
            }
        }
        self.windows.clear();
        self.image_memory_bytes = 0;
    }
}

struct PinWindowContext {
    #[allow(dead_code)]
    pin_id: u64,
    image: PinImageData,
    geometry: PinGeometry,
    opacity: u8,
    drag_offset: Option<(i32, i32)>,
    menu_anchor: Option<POINT>,
    keyboard_hook: std::sync::Arc<KeyboardHookBroker>,
    escape_generation: Option<u64>,
    clipboard_writer: Arc<ClipboardWriter>,
    suppressor: ClipboardSequenceSuppressor,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PinGeometry {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    scale: f64,
}

impl PinGeometry {
    fn initial(
        image_width: u32,
        image_height: u32,
        work: RECT,
        cursor: POINT,
    ) -> Result<Self, NativeSurfaceError> {
        let max_scale = max_scale(image_width, image_height, work)?;
        let min_scale = min_scale(image_width, image_height).min(max_scale);
        let scale = 1.0_f64.min(max_scale).max(min_scale);
        let (width, height) = scaled_size(image_width, image_height, scale)?;
        let x = cursor.x - width / 2;
        let y = cursor.y - height / 2;
        Ok(Self {
            x,
            y,
            width,
            height,
            scale,
        }
        .fully_inside(work))
    }

    fn resize_around(
        self,
        image_width: u32,
        image_height: u32,
        factor: f64,
        anchor: POINT,
        work: RECT,
    ) -> Result<Self, NativeSurfaceError> {
        let max_scale = max_scale(image_width, image_height, work)?;
        let min_scale = min_scale(image_width, image_height).min(max_scale);
        let scale = (self.scale * factor).clamp(min_scale, max_scale);
        let (width, height) = scaled_size(image_width, image_height, scale)?;
        let relative_x = (anchor.x - self.x) as f64 / self.width.max(1) as f64;
        let relative_y = (anchor.y - self.y) as f64 / self.height.max(1) as f64;
        let x = anchor.x - (relative_x * width as f64).round() as i32;
        let y = anchor.y - (relative_y * height as f64).round() as i32;
        Ok(Self {
            x,
            y,
            width,
            height,
            scale,
        }
        .at_least_partly_visible(work))
    }

    fn fully_inside(mut self, work: RECT) -> Self {
        self.x = self.x.clamp(work.left, work.right - self.width);
        self.y = self.y.clamp(work.top, work.bottom - self.height);
        self
    }

    fn at_least_partly_visible(mut self, work: RECT) -> Self {
        let visible_x = MIN_VISIBLE_PIXELS.min(self.width);
        let visible_y = MIN_VISIBLE_PIXELS.min(self.height);
        self.x = self
            .x
            .clamp(work.left - self.width + visible_x, work.right - visible_x);
        self.y = self
            .y
            .clamp(work.top - self.height + visible_y, work.bottom - visible_y);
        self
    }
}

fn runtime_thread(
    receiver: Receiver<RuntimeCommand>,
    ready: SyncSender<Result<u32, NativeSurfaceError>>,
    shutdown: Arc<AtomicBool>,
    keyboard_hook: Arc<KeyboardHookBroker>,
    suppressor: ClipboardSequenceSuppressor,
) {
    let result = initialize_runtime();
    let (module, thread_id) = match result {
        Ok(value) => value,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    if ready.send(Ok(thread_id)).is_err() {
        unregister_window_class(module);
        return;
    }
    let mut state = RuntimeState::new(module, keyboard_hook, suppressor);
    let mut message: MSG = unsafe { zeroed() };
    loop {
        let result = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
        if result <= 0 {
            break;
        }
        if message.message == RUNTIME_MESSAGE {
            while let Ok(command) = receiver.try_recv() {
                match command {
                    RuntimeCommand::Open { image, reply } => {
                        let _ = reply.send(state.open(image));
                    }
                }
            }
            if shutdown.load(Ordering::Acquire) {
                state.destroy_all();
                unsafe { PostQuitMessage(0) };
            }
        } else {
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            state.remove_destroyed_windows();
        }
    }
    state.destroy_all();
    unregister_window_class(module);
}

fn initialize_runtime() -> Result<(HINSTANCE, u32), NativeSurfaceError> {
    let mut message: MSG = unsafe { zeroed() };
    unsafe {
        let _ = PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
    }
    let module = unsafe { GetModuleHandleW(null()) };
    if module.is_null() {
        return Err(NativeSurfaceError::WindowsApi("GetModuleHandleW"));
    }
    let class = WNDCLASSW {
        style: CS_DBLCLKS,
        lpfnWndProc: Some(window_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: module,
        hIcon: null_mut(),
        hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
        hbrBackground: null_mut(),
        lpszMenuName: null(),
        lpszClassName: CLASS_NAME.as_ptr(),
    };
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err(NativeSurfaceError::WindowsApi("RegisterClassW"));
    }
    Ok((module, unsafe { GetCurrentThreadId() }))
}

fn unregister_window_class(module: HINSTANCE) {
    unsafe {
        let _ = UnregisterClassW(CLASS_NAME.as_ptr(), module);
    }
}

fn post_runtime_message(thread_id: u32) -> Result<(), NativeSurfaceError> {
    if unsafe { PostThreadMessageW(thread_id, RUNTIME_MESSAGE, 0, 0) } == 0 {
        Err(NativeSurfaceError::CommandQueueUnavailable)
    } else {
        Ok(())
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
    if message == WM_CLOSE {
        let context = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut PinWindowContext;
        SetWindowLongPtrW(window, GWLP_USERDATA, 0);
        if !context.is_null() {
            disarm_escape_close(&mut *context);
        }
        DestroyWindow(window);
        return 0;
    }
    let context = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut PinWindowContext;
    if context.is_null() {
        return DefWindowProcW(window, message, wparam, lparam);
    }
    let context = &mut *context;
    match message {
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_NCHITTEST => hit_test(context, point_from_lparam(lparam)),
        WM_LBUTTONDOWN => {
            arm_escape_close(window, context);
            if let Some(cursor) = cursor_position() {
                let _ = SetCapture(window);
                context.drag_offset =
                    Some((cursor.x - context.geometry.x, cursor.y - context.geometry.y));
            }
            0
        }
        WM_MOUSEMOVE => {
            if let (Some(cursor), Some((offset_x, offset_y))) =
                (cursor_position(), context.drag_offset)
            {
                context.geometry.x = cursor.x - offset_x;
                context.geometry.y = cursor.y - offset_y;
                if let Ok(work) = monitor_work_area(cursor) {
                    context.geometry = context.geometry.at_least_partly_visible(work);
                }
                let _ = position_layered(window, context.geometry);
            }
            0
        }
        WM_LBUTTONUP => {
            context.drag_offset = None;
            let _ = ReleaseCapture();
            0
        }
        WM_CAPTURECHANGED => {
            context.drag_offset = None;
            0
        }
        WM_MOUSEWHEEL => {
            handle_wheel(window, context, wparam, point_from_lparam(lparam));
            0
        }
        WM_RBUTTONUP => {
            arm_escape_close(window, context);
            let cursor = cursor_position().unwrap_or_else(|| point_from_lparam(lparam));
            context.menu_anchor = Some(cursor);
            let command = show_context_menu(window, cursor, context.image.source_text.is_some());
            if command != 0 {
                let _ = PostMessageW(window, MENU_COMMAND_MESSAGE, command, 0);
            }
            0
        }
        MENU_COMMAND_MESSAGE => {
            apply_context_menu_command(window, context, wparam);
            0
        }
        WM_DISPLAYCHANGE => {
            let center = POINT {
                x: context.geometry.x + context.geometry.width / 2,
                y: context.geometry.y + context.geometry.height / 2,
            };
            if let Ok(work) = monitor_work_area(center) {
                context.geometry = context.geometry.at_least_partly_visible(work);
                let _ = position_layered(window, context.geometry);
            }
            0
        }
        WM_DESTROY => 0,
        WM_NCDESTROY => {
            disarm_escape_close(context);
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
            DefWindowProcW(window, message, wparam, lparam)
        }
        _ => DefWindowProcW(window, message, wparam, lparam),
    }
}

fn arm_escape_close(window: HWND, context: &mut PinWindowContext) {
    if let Some(generation) = context.escape_generation.take() {
        let _ = context.keyboard_hook.unregister_surface_escape(generation);
    }
    let window_handle = window as usize;
    context.escape_generation = context
        .keyboard_hook
        .register_surface_escape(move |_| unsafe {
            let _ = PostMessageW(window_handle as HWND, WM_CLOSE, 0, 0);
        })
        .ok();
}

fn disarm_escape_close(context: &mut PinWindowContext) {
    if let Some(generation) = context.escape_generation.take() {
        let _ = context.keyboard_hook.unregister_surface_escape(generation);
    }
}

fn handle_wheel(window: HWND, context: &mut PinWindowContext, wparam: WPARAM, cursor: POINT) {
    let wheel_delta = ((wparam >> 16) as u16) as i16;
    if wheel_delta == 0 {
        return;
    }
    let steps = f64::from(wheel_delta) / 120.0;
    if unsafe { GetKeyState(VK_CONTROL as i32) } < 0 {
        let opacity = i32::from(context.opacity) + (steps.signum() as i32 * 26);
        context.opacity = opacity.clamp(26, 255) as u8;
    } else if let Ok(work) = monitor_work_area(cursor) {
        let factor = 1.1_f64.powf(steps);
        if let Ok(geometry) = context.geometry.resize_around(
            context.image.width,
            context.image.height,
            factor,
            cursor,
            work,
        ) {
            context.geometry = geometry;
        }
    }
    let _ = render_layered(window, context);
}

fn show_context_menu(window: HWND, cursor: POINT, has_source_text: bool) -> usize {
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return 0;
    }
    append_menu(menu, MENU_COPY_IMAGE, "复制图像");
    if has_source_text {
        append_menu(menu, MENU_COPY_TEXT, "复制文字");
    }
    append_menu(menu, MENU_SAVE, "保存 PNG...");
    unsafe {
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, null());
    }
    append_menu(menu, MENU_ORIGINAL_SIZE, "原始大小");
    unsafe {
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, null());
    }
    append_menu(menu, MENU_OPACITY_100, "透明度 100%");
    append_menu(menu, MENU_OPACITY_75, "透明度 75%");
    append_menu(menu, MENU_OPACITY_50, "透明度 50%");
    unsafe {
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, null());
    }
    append_menu(menu, MENU_CLOSE, "关闭贴图");
    let command = unsafe {
        TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            cursor.x,
            cursor.y,
            0,
            window,
            null(),
        )
    };
    unsafe {
        let _ = DestroyMenu(menu);
    }
    command as usize
}

fn apply_context_menu_command(window: HWND, context: &mut PinWindowContext, command: usize) {
    let cursor = context
        .menu_anchor
        .take()
        .or_else(cursor_position)
        .unwrap_or(POINT {
            x: context.geometry.x + context.geometry.width / 2,
            y: context.geometry.y + context.geometry.height / 2,
        });
    match command {
        MENU_COPY_IMAGE => {
            dispatch_clipboard_write(
                Arc::clone(&context.clipboard_writer),
                context.suppressor.clone(),
                ClipboardWriteContent::Image {
                    width: context.image.width,
                    height: context.image.height,
                    rgba: context.image.rgba.clone(),
                },
            );
        }
        MENU_COPY_TEXT => {
            if let Some(text) = context.image.source_text.clone() {
                dispatch_clipboard_write(
                    Arc::clone(&context.clipboard_writer),
                    context.suppressor.clone(),
                    ClipboardWriteContent::Text(text),
                );
            }
        }
        MENU_SAVE => {
            let image = context.image.clone();
            let _ = thread::Builder::new()
                .name("pin-image-save".to_owned())
                .spawn(move || {
                    let _ = save_rgba_with_dialog(
                        "OpenDeskTools-贴图.png",
                        image.width,
                        image.height,
                        &image.rgba,
                    );
                });
        }
        MENU_ORIGINAL_SIZE => {
            if let Ok(work) = monitor_work_area(cursor) {
                let max = max_scale(context.image.width, context.image.height, work).unwrap_or(1.0);
                let target = 1.0_f64.min(max);
                let factor = target / context.geometry.scale.max(f64::EPSILON);
                if let Ok(geometry) = context.geometry.resize_around(
                    context.image.width,
                    context.image.height,
                    factor,
                    cursor,
                    work,
                ) {
                    context.geometry = geometry;
                    let _ = render_layered(window, context);
                }
            }
        }
        MENU_OPACITY_100 => {
            context.opacity = 255;
            let _ = render_layered(window, context);
        }
        MENU_OPACITY_75 => {
            context.opacity = 191;
            let _ = render_layered(window, context);
        }
        MENU_OPACITY_50 => {
            context.opacity = 128;
            let _ = render_layered(window, context);
        }
        MENU_CLOSE => unsafe {
            let _ = PostMessageW(window, WM_CLOSE, 0, 0);
        },
        _ => {}
    }
}

fn dispatch_clipboard_write(
    writer: Arc<ClipboardWriter>,
    suppressor: ClipboardSequenceSuppressor,
    content: ClipboardWriteContent,
) {
    let _ = thread::Builder::new()
        .name("pin-surface-copy".to_owned())
        .spawn(move || {
            let _ = writer.replace_current(0, &content, |sequence| {
                suppressor.suppress_sequence(sequence)
            });
        });
}

fn append_menu(menu: HWND, id: usize, label: &str) {
    let wide: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = AppendMenuW(menu, MF_STRING, id, wide.as_ptr());
    }
}

fn hit_test(context: &PinWindowContext, point: POINT) -> LRESULT {
    let local_x = point.x - context.geometry.x;
    let local_y = point.y - context.geometry.y;
    if local_x < 0
        || local_y < 0
        || local_x >= context.geometry.width
        || local_y >= context.geometry.height
    {
        return HTTRANSPARENT as LRESULT;
    }
    let source_x =
        (local_x as u64 * u64::from(context.image.width) / context.geometry.width as u64) as u32;
    let source_y =
        (local_y as u64 * u64::from(context.image.height) / context.geometry.height as u64) as u32;
    let offset = ((u64::from(source_y) * u64::from(context.image.width) + u64::from(source_x)) * 4
        + 3) as usize;
    if context.image.rgba.get(offset).copied().unwrap_or(0) <= 4 {
        HTTRANSPARENT as LRESULT
    } else {
        HTCLIENT as LRESULT
    }
}

fn render_layered(window: HWND, context: &PinWindowContext) -> Result<(), NativeSurfaceError> {
    let pixels = scaled_premultiplied_bgra(
        &context.image,
        context.geometry.width,
        context.geometry.height,
    )?;
    let screen_dc = unsafe { windows_sys::Win32::Graphics::Gdi::GetDC(null_mut()) };
    if screen_dc.is_null() {
        return Err(NativeSurfaceError::WindowsApi("GetDC"));
    }
    let memory_dc = unsafe { CreateCompatibleDC(screen_dc) };
    if memory_dc.is_null() {
        unsafe {
            let _ = windows_sys::Win32::Graphics::Gdi::ReleaseDC(null_mut(), screen_dc);
        }
        return Err(NativeSurfaceError::WindowsApi("CreateCompatibleDC"));
    }
    let mut bits = null_mut();
    let info = bitmap_info(context.geometry.width, context.geometry.height);
    let bitmap =
        unsafe { CreateDIBSection(memory_dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0) };
    if bitmap.is_null() || bits.is_null() {
        unsafe {
            let _ = DeleteDC(memory_dc);
            let _ = windows_sys::Win32::Graphics::Gdi::ReleaseDC(null_mut(), screen_dc);
        }
        return Err(NativeSurfaceError::WindowsApi("CreateDIBSection"));
    }
    unsafe {
        copy_nonoverlapping(pixels.as_ptr(), bits.cast(), pixels.len());
    }
    let previous = unsafe { SelectObject(memory_dc, bitmap as HGDIOBJ) };
    if previous.is_null() || previous == HGDI_ERROR {
        unsafe {
            let _ = DeleteObject(bitmap as HGDIOBJ);
            let _ = DeleteDC(memory_dc);
            let _ = windows_sys::Win32::Graphics::Gdi::ReleaseDC(null_mut(), screen_dc);
        }
        return Err(NativeSurfaceError::WindowsApi("SelectObject"));
    }
    let destination = POINT {
        x: context.geometry.x,
        y: context.geometry.y,
    };
    let source = POINT { x: 0, y: 0 };
    let size = SIZE {
        cx: context.geometry.width,
        cy: context.geometry.height,
    };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: context.opacity,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let succeeded = unsafe {
        UpdateLayeredWindow(
            window,
            screen_dc,
            &destination,
            &size,
            memory_dc,
            &source,
            0,
            &blend,
            ULW_ALPHA,
        )
    } != 0;
    unsafe {
        let _ = SelectObject(memory_dc, previous);
        let _ = DeleteObject(bitmap as HGDIOBJ);
        let _ = DeleteDC(memory_dc);
        let _ = windows_sys::Win32::Graphics::Gdi::ReleaseDC(null_mut(), screen_dc);
    }
    if succeeded {
        Ok(())
    } else {
        Err(NativeSurfaceError::WindowsApi("UpdateLayeredWindow"))
    }
}

fn position_layered(window: HWND, geometry: PinGeometry) -> Result<(), NativeSurfaceError> {
    if unsafe {
        SetWindowPos(
            window,
            HWND_TOPMOST,
            geometry.x,
            geometry.y,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        )
    } == 0
    {
        Err(NativeSurfaceError::WindowsApi("SetWindowPos"))
    } else {
        Ok(())
    }
}

fn scaled_premultiplied_bgra(
    image: &PinImageData,
    target_width: i32,
    target_height: i32,
) -> Result<Vec<u8>, NativeSurfaceError> {
    let width = usize::try_from(target_width).map_err(|_| NativeSurfaceError::InvalidImage)?;
    let height = usize::try_from(target_height).map_err(|_| NativeSurfaceError::InvalidImage)?;
    let byte_count = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .filter(|bytes| *bytes <= MAX_PIN_MEMORY_BYTES)
        .ok_or(NativeSurfaceError::MemoryBudgetExceeded)?;
    let mut output = vec![0u8; byte_count];
    for target_y in 0..height {
        let source_y = target_y * image.height as usize / height;
        for target_x in 0..width {
            let source_x = target_x * image.width as usize / width;
            let source = (source_y * image.width as usize + source_x) * 4;
            let destination = (target_y * width + target_x) * 4;
            let alpha = u16::from(image.rgba[source + 3]);
            output[destination] = ((u16::from(image.rgba[source + 2]) * alpha + 127) / 255) as u8;
            output[destination + 1] =
                ((u16::from(image.rgba[source + 1]) * alpha + 127) / 255) as u8;
            output[destination + 2] = ((u16::from(image.rgba[source]) * alpha + 127) / 255) as u8;
            output[destination + 3] = alpha as u8;
        }
    }
    Ok(output)
}

fn bitmap_info(width: i32, height: i32) -> BITMAPINFO {
    let mut info: BITMAPINFO = unsafe { zeroed() };
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width,
        biHeight: -height,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        biSizeImage: (width as u32)
            .saturating_mul(height as u32)
            .saturating_mul(4),
        biXPelsPerMeter: 0,
        biYPelsPerMeter: 0,
        biClrUsed: 0,
        biClrImportant: 0,
    };
    info
}

fn cursor_position() -> Option<POINT> {
    let mut point = POINT { x: 0, y: 0 };
    (unsafe { GetCursorPos(&mut point) } != 0).then_some(point)
}

fn monitor_work_area(point: POINT) -> Result<RECT, NativeSurfaceError> {
    let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_null() {
        return Err(NativeSurfaceError::WindowsApi("MonitorFromPoint"));
    }
    let mut info: MONITORINFO = unsafe { zeroed() };
    info.cbSize = size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return Err(NativeSurfaceError::WindowsApi("GetMonitorInfoW"));
    }
    Ok(info.rcWork)
}

fn max_scale(image_width: u32, image_height: u32, work: RECT) -> Result<f64, NativeSurfaceError> {
    let work_width = work.right - work.left;
    let work_height = work.bottom - work.top;
    if image_width == 0 || image_height == 0 || work_width <= 0 || work_height <= 0 {
        return Err(NativeSurfaceError::InvalidImage);
    }
    Ok(
        ((f64::from(work_width) * MAX_WORK_AREA_RATIO) / f64::from(image_width))
            .min((f64::from(work_height) * MAX_WORK_AREA_RATIO) / f64::from(image_height)),
    )
}

fn min_scale(image_width: u32, image_height: u32) -> f64 {
    (MIN_SHORT_EDGE / f64::from(image_width)).max(MIN_SHORT_EDGE / f64::from(image_height))
}

fn scaled_size(
    image_width: u32,
    image_height: u32,
    scale: f64,
) -> Result<(i32, i32), NativeSurfaceError> {
    let width = (f64::from(image_width) * scale).round().max(1.0);
    let height = (f64::from(image_height) * scale).round().max(1.0);
    if width > f64::from(i32::MAX) || height > f64::from(i32::MAX) {
        return Err(NativeSurfaceError::InvalidImage);
    }
    Ok((width as i32, height as i32))
}

fn point_from_lparam(lparam: LPARAM) -> POINT {
    POINT {
        x: (lparam as u32 as u16 as i16) as i32,
        y: ((lparam as u32 >> 16) as u16 as i16) as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work_area() -> RECT {
        RECT {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1080,
        }
    }

    #[test]
    fn initial_geometry_preserves_aspect_and_stays_inside_negative_work_area() {
        let geometry =
            PinGeometry::initial(1600, 900, work_area(), POINT { x: -960, y: 540 }).unwrap();
        assert_eq!(geometry.width, 1600);
        assert_eq!(geometry.height, 900);
        assert!(geometry.x >= -1920);
        assert!(geometry.y >= 0);
        assert!(geometry.x + geometry.width <= 0);
        assert!(geometry.y + geometry.height <= 1080);
    }

    #[test]
    fn resize_uses_cursor_anchor_and_enforces_work_area_limit() {
        let initial =
            PinGeometry::initial(800, 400, work_area(), POINT { x: -960, y: 540 }).unwrap();
        let resized = initial
            .resize_around(800, 400, 20.0, POINT { x: -960, y: 540 }, work_area())
            .unwrap();
        assert_eq!(resized.width * 400, resized.height * 800);
        assert!(resized.width <= 1728);
        assert!(resized.height <= 972);
    }

    #[test]
    fn premultiplication_keeps_transparent_pixels_colorless() {
        let image = PinImageData {
            width: 2,
            height: 1,
            rgba: vec![255, 128, 64, 0, 200, 100, 50, 128],
            source_text: None,
        };
        let bgra = scaled_premultiplied_bgra(&image, 2, 1).unwrap();
        assert_eq!(&bgra[..4], &[0, 0, 0, 0]);
        assert_eq!(&bgra[4..], &[25, 50, 100, 128]);
    }

    #[test]
    fn text_card_corner_mask_keeps_center_opaque_and_corners_transparent() {
        let mut rgba = vec![255; 40 * 30 * 4];
        apply_rounded_alpha(&mut rgba, 40, 30, 8);
        assert_eq!(rgba[3], 0);
        let center = ((15 * 40 + 20) * 4 + 3) as usize;
        assert_eq!(rgba[center], 255);
    }
}
