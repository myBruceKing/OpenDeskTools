use super::{BackendCapability, CaptureBackend, CaptureOptions};
use crate::infrastructure::screenshot::model::{
    CaptureBackendName, MonitorDescriptor, MonitorFrame,
};
use crate::infrastructure::screenshot::monitor::MonitorTopology;
use crate::infrastructure::screenshot::{ScreenshotError, MAX_SNAPSHOT_BYTES};

#[derive(Debug, Default)]
pub struct GdiCaptureBackend;

impl GdiCaptureBackend {
    pub fn new() -> Self {
        Self
    }
}

impl CaptureBackend for GdiCaptureBackend {
    fn name(&self) -> CaptureBackendName {
        CaptureBackendName::Gdi
    }

    fn probe(&self, topology: &MonitorTopology) -> BackendCapability {
        #[cfg(windows)]
        {
            windows_impl::probe(topology)
        }
        #[cfg(not(windows))]
        {
            let _ = topology;
            BackendCapability::unavailable("GDI is available only on Windows")
        }
    }

    fn capture_monitor(
        &mut self,
        monitor: &MonitorDescriptor,
        options: &CaptureOptions,
    ) -> Result<MonitorFrame, ScreenshotError> {
        #[cfg(windows)]
        {
            windows_impl::capture_monitor(monitor, options)
        }
        #[cfg(not(windows))]
        {
            let _ = (monitor, options);
            Err(ScreenshotError::UnsupportedPlatform)
        }
    }
}

#[cfg(windows)]
mod windows_impl {
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::ptr::{null, null_mut};
    use std::slice;

    use windows_sys::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleDC, CreateDCW, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush,
        SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HBITMAP,
        HDC, HGDIOBJ, SRCCOPY,
    };

    use super::*;

    const HGDI_ERROR: HGDIOBJ = -1isize as HGDIOBJ;

    pub(super) fn probe(topology: &MonitorTopology) -> BackendCapability {
        for monitor in &topology.monitors {
            match OwnedDc::for_monitor(&monitor.id) {
                Ok(_) => {}
                Err(_) => {
                    return BackendCapability::unavailable(format!(
                        "display DC unavailable for {}",
                        monitor.id
                    ));
                }
            }
        }
        BackendCapability::available("GDI BitBlt with CAPTUREBLT")
    }

    pub(super) fn capture_monitor(
        monitor: &MonitorDescriptor,
        options: &CaptureOptions,
    ) -> Result<MonitorFrame, ScreenshotError> {
        if options.include_cursor {
            return Err(ScreenshotError::BackendUnavailable(
                "GDI cursor composition is not implemented".to_owned(),
            ));
        }
        monitor.validate()?;
        let width = monitor
            .physical_bounds
            .width()
            .ok_or(ScreenshotError::InvalidTopology)?;
        let height = monitor
            .physical_bounds
            .height()
            .ok_or(ScreenshotError::InvalidTopology)?;
        let (width_i32, height_i32, stride, byte_len) = bitmap_layout(width, height)?;

        let source_dc = OwnedDc::for_monitor(&monitor.id)?;
        let memory_dc = OwnedDc::compatible(source_dc.0)?;
        let mut bits = null_mut::<c_void>();
        let bitmap_info = top_down_bitmap_info(width_i32, height_i32, byte_len)?;
        let bitmap = unsafe {
            CreateDIBSection(
                memory_dc.0,
                &bitmap_info,
                DIB_RGB_COLORS,
                &mut bits,
                null_mut(),
                0,
            )
        };
        let bitmap = OwnedBitmap::new(bitmap)?;
        if bits.is_null() {
            return Err(ScreenshotError::WindowsApi("CreateDIBSection bits"));
        }
        let selected = SelectedObject::new(memory_dc.0, bitmap.0 as HGDIOBJ)?;
        let copied = unsafe {
            BitBlt(
                memory_dc.0,
                0,
                0,
                width_i32,
                height_i32,
                source_dc.0,
                0,
                0,
                SRCCOPY | CAPTUREBLT,
            )
        };
        if copied == 0 {
            return Err(ScreenshotError::WindowsApi("BitBlt"));
        }
        unsafe {
            let _ = GdiFlush();
        }

        let mut bgra = unsafe { slice::from_raw_parts(bits.cast::<u8>(), byte_len) }.to_vec();
        // BI_RGB screen captures do not define the alpha byte. The desktop is
        // opaque, so normalize it before the frame leaves the backend.
        for alpha in bgra.iter_mut().skip(3).step_by(4) {
            *alpha = 255;
        }
        drop(selected);
        drop(bitmap);

        let frame = MonitorFrame {
            monitor: monitor.clone(),
            width,
            height,
            stride,
            bgra,
        };
        frame.validate()?;
        Ok(frame)
    }

    fn bitmap_layout(width: u32, height: u32) -> Result<(i32, i32, usize, usize), ScreenshotError> {
        let width_i32 = i32::try_from(width).map_err(|_| ScreenshotError::MemoryLimit)?;
        let height_i32 = i32::try_from(height).map_err(|_| ScreenshotError::MemoryLimit)?;
        if width_i32 <= 0 || height_i32 <= 0 {
            return Err(ScreenshotError::InvalidFrame);
        }
        let stride = usize::try_from(width)
            .ok()
            .and_then(|width| width.checked_mul(4))
            .ok_or(ScreenshotError::ArithmeticOverflow)?;
        let byte_len = stride
            .checked_mul(usize::try_from(height).map_err(|_| ScreenshotError::ArithmeticOverflow)?)
            .ok_or(ScreenshotError::ArithmeticOverflow)?;
        if byte_len > MAX_SNAPSHOT_BYTES {
            return Err(ScreenshotError::MemoryLimit);
        }
        Ok((width_i32, height_i32, stride, byte_len))
    }

    fn top_down_bitmap_info(
        width: i32,
        height: i32,
        byte_len: usize,
    ) -> Result<BITMAPINFO, ScreenshotError> {
        let image_size = u32::try_from(byte_len).map_err(|_| ScreenshotError::MemoryLimit)?;
        let mut info: BITMAPINFO = unsafe { zeroed() };
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: height
                .checked_neg()
                .ok_or(ScreenshotError::ArithmeticOverflow)?,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            biSizeImage: image_size,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        };
        Ok(info)
    }

    struct OwnedDc(HDC);

    impl OwnedDc {
        fn for_monitor(monitor_id: &str) -> Result<Self, ScreenshotError> {
            let driver = wide_null("DISPLAY");
            let device = wide_null(monitor_id);
            let handle = unsafe { CreateDCW(driver.as_ptr(), device.as_ptr(), null(), null()) };
            if handle.is_null() {
                return Err(ScreenshotError::MonitorUnavailable(monitor_id.to_owned()));
            }
            Ok(Self(handle))
        }

        fn compatible(source: HDC) -> Result<Self, ScreenshotError> {
            let handle = unsafe { CreateCompatibleDC(source) };
            if handle.is_null() {
                return Err(ScreenshotError::WindowsApi("CreateCompatibleDC"));
            }
            Ok(Self(handle))
        }
    }

    impl Drop for OwnedDc {
        fn drop(&mut self) {
            unsafe {
                let _ = DeleteDC(self.0);
            }
        }
    }

    struct OwnedBitmap(HBITMAP);

    impl OwnedBitmap {
        fn new(handle: HBITMAP) -> Result<Self, ScreenshotError> {
            if handle.is_null() {
                return Err(ScreenshotError::WindowsApi("CreateDIBSection"));
            }
            Ok(Self(handle))
        }
    }

    impl Drop for OwnedBitmap {
        fn drop(&mut self) {
            unsafe {
                let _ = DeleteObject(self.0 as HGDIOBJ);
            }
        }
    }

    struct SelectedObject {
        device_context: HDC,
        previous: HGDIOBJ,
    }

    impl SelectedObject {
        fn new(device_context: HDC, object: HGDIOBJ) -> Result<Self, ScreenshotError> {
            let previous = unsafe { SelectObject(device_context, object) };
            if previous.is_null() || previous == HGDI_ERROR {
                return Err(ScreenshotError::WindowsApi("SelectObject"));
            }
            Ok(Self {
                device_context,
                previous,
            })
        }
    }

    impl Drop for SelectedObject {
        fn drop(&mut self) {
            unsafe {
                let _ = SelectObject(self.device_context, self.previous);
            }
        }
    }

    fn wide_null(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn bitmap_layout_is_top_down_and_checked() {
            assert_eq!(bitmap_layout(3, 2).unwrap(), (3, 2, 12, 24));
            let info = top_down_bitmap_info(3, 2, 24).unwrap();
            assert_eq!(info.bmiHeader.biWidth, 3);
            assert_eq!(info.bmiHeader.biHeight, -2);
            assert_eq!(info.bmiHeader.biBitCount, 32);
            assert_eq!(info.bmiHeader.biSizeImage, 24);
        }
    }
}
