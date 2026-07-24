use std::collections::HashSet;

use super::model::{DisplayRotation, MonitorDescriptor, PhysicalRect};
use super::ScreenshotError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorTopology {
    pub virtual_bounds: PhysicalRect,
    pub monitors: Vec<MonitorDescriptor>,
}

impl MonitorTopology {
    pub fn new(monitors: Vec<MonitorDescriptor>) -> Result<Self, ScreenshotError> {
        let first = monitors.first().ok_or(ScreenshotError::InvalidTopology)?;
        let mut virtual_bounds = first.physical_bounds;
        let mut identifiers = HashSet::with_capacity(monitors.len());
        for monitor in &monitors {
            monitor.validate()?;
            if !identifiers.insert(monitor.id.clone()) {
                return Err(ScreenshotError::DuplicateMonitor(monitor.id.clone()));
            }
            virtual_bounds = virtual_bounds.union(monitor.physical_bounds);
        }
        Ok(Self {
            virtual_bounds,
            monitors,
        })
    }

    #[cfg(windows)]
    pub fn query() -> Result<Self, ScreenshotError> {
        windows_impl::query()
    }

    #[cfg(not(windows))]
    pub fn query() -> Result<Self, ScreenshotError> {
        Err(ScreenshotError::UnsupportedPlatform)
    }
}

#[cfg(windows)]
mod windows_impl {
    use std::mem::{size_of, zeroed};
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{FreeLibrary, BOOL, LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        CreateDCW, DeleteDC, EnumDisplayMonitors, EnumDisplaySettingsExW, GetDeviceCaps,
        GetMonitorInfoW, DEVMODEW, DMDO_180, DMDO_270, DMDO_90, DMDO_DEFAULT,
        ENUM_CURRENT_SETTINGS, HDC, HMONITOR, LOGPIXELSX, LOGPIXELSY, MONITORINFO, MONITORINFOEXW,
    };
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows_sys::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;

    use super::*;

    const DEFAULT_DPI: u32 = 96;
    const MDT_EFFECTIVE_DPI: i32 = 0;
    type GetDpiForMonitorFn = unsafe extern "system" fn(HMONITOR, i32, *mut u32, *mut u32) -> i32;

    struct EnumerationContext {
        monitors: Vec<MonitorDescriptor>,
        error: Option<ScreenshotError>,
    }

    pub(super) fn query() -> Result<MonitorTopology, ScreenshotError> {
        let mut context = EnumerationContext {
            monitors: Vec::new(),
            error: None,
        };
        let success = unsafe {
            EnumDisplayMonitors(
                null_mut(),
                null(),
                Some(enumerate_monitor),
                (&mut context as *mut EnumerationContext) as LPARAM,
            )
        };
        if success == 0 {
            return Err(context
                .error
                .unwrap_or(ScreenshotError::MonitorEnumerationFailed));
        }
        if let Some(error) = context.error {
            return Err(error);
        }
        MonitorTopology::new(context.monitors)
    }

    unsafe extern "system" fn enumerate_monitor(
        monitor_handle: HMONITOR,
        _device_context: HDC,
        _monitor_rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let context = &mut *(data as *mut EnumerationContext);
        match describe_monitor(monitor_handle) {
            Ok(monitor) => {
                context.monitors.push(monitor);
                1
            }
            Err(error) => {
                context.error = Some(error);
                0
            }
        }
    }

    unsafe fn describe_monitor(
        monitor_handle: HMONITOR,
    ) -> Result<MonitorDescriptor, ScreenshotError> {
        let mut info: MONITORINFOEXW = zeroed();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        let info_ptr = (&mut info as *mut MONITORINFOEXW).cast::<MONITORINFO>();
        if GetMonitorInfoW(monitor_handle, info_ptr) == 0 {
            return Err(ScreenshotError::MonitorMetadataUnavailable);
        }

        let id = wide_array_to_string(&info.szDevice);
        if id.is_empty() {
            return Err(ScreenshotError::MonitorMetadataUnavailable);
        }
        let physical_bounds = rect_from_win32(info.monitorInfo.rcMonitor)?;
        let work_bounds = rect_from_win32(info.monitorInfo.rcWork)?;
        let (dpi_x, dpi_y) = monitor_dpi(monitor_handle, &info.szDevice);
        let rotation = display_rotation(&info.szDevice);

        Ok(MonitorDescriptor {
            id,
            physical_bounds,
            work_bounds,
            dpi_x,
            dpi_y,
            rotation,
            is_primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        })
    }

    fn rect_from_win32(rect: RECT) -> Result<PhysicalRect, ScreenshotError> {
        PhysicalRect::new(rect.left, rect.top, rect.right, rect.bottom)
            .map_err(|_| ScreenshotError::MonitorMetadataUnavailable)
    }

    unsafe fn monitor_dpi(monitor_handle: HMONITOR, device_name: &[u16; 32]) -> (u32, u32) {
        if let Some(dpi) = monitor_dpi_from_shcore(monitor_handle) {
            return dpi;
        }
        monitor_dpi_from_device_context(device_name).unwrap_or((DEFAULT_DPI, DEFAULT_DPI))
    }

    unsafe fn monitor_dpi_from_shcore(monitor_handle: HMONITOR) -> Option<(u32, u32)> {
        let library_name = wide_null("shcore.dll");
        let module = LoadLibraryW(library_name.as_ptr());
        if module.is_null() {
            return None;
        }
        let result = (|| {
            let raw = GetProcAddress(module, c"GetDpiForMonitor".as_ptr().cast());
            let raw = raw?;
            let function: GetDpiForMonitorFn = std::mem::transmute(raw);
            let mut dpi_x = 0u32;
            let mut dpi_y = 0u32;
            let status = function(monitor_handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
            (status >= 0 && dpi_x > 0 && dpi_y > 0).then_some((dpi_x, dpi_y))
        })();
        let _ = FreeLibrary(module);
        result
    }

    unsafe fn monitor_dpi_from_device_context(device_name: &[u16; 32]) -> Option<(u32, u32)> {
        let driver = wide_null("DISPLAY");
        let device_context = CreateDCW(
            driver.as_ptr(),
            device_name.as_ptr(),
            null(),
            null::<DEVMODEW>(),
        );
        if device_context.is_null() {
            return None;
        }
        let dpi_x = GetDeviceCaps(device_context, LOGPIXELSX as i32);
        let dpi_y = GetDeviceCaps(device_context, LOGPIXELSY as i32);
        let _ = DeleteDC(device_context);
        (dpi_x > 0 && dpi_y > 0).then_some((dpi_x as u32, dpi_y as u32))
    }

    unsafe fn display_rotation(device_name: &[u16; 32]) -> DisplayRotation {
        let mut mode: DEVMODEW = zeroed();
        mode.dmSize = size_of::<DEVMODEW>() as u16;
        if EnumDisplaySettingsExW(device_name.as_ptr(), ENUM_CURRENT_SETTINGS, &mut mode, 0) == 0 {
            return DisplayRotation::Unknown;
        }
        match mode.Anonymous1.Anonymous2.dmDisplayOrientation {
            DMDO_DEFAULT => DisplayRotation::Identity,
            DMDO_90 => DisplayRotation::Rotate90,
            DMDO_180 => DisplayRotation::Rotate180,
            DMDO_270 => DisplayRotation::Rotate270,
            _ => DisplayRotation::Unknown,
        }
    }

    fn wide_array_to_string(value: &[u16]) -> String {
        let end = value
            .iter()
            .position(|character| *character == 0)
            .unwrap_or(value.len());
        String::from_utf16_lossy(&value[..end])
    }

    fn wide_null(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(id: &str, bounds: PhysicalRect) -> MonitorDescriptor {
        MonitorDescriptor {
            id: id.to_owned(),
            physical_bounds: bounds,
            work_bounds: bounds,
            dpi_x: 96,
            dpi_y: 96,
            rotation: DisplayRotation::Identity,
            is_primary: id == "primary",
        }
    }

    #[test]
    fn topology_unions_negative_and_positive_monitor_bounds() {
        let topology = MonitorTopology::new(vec![
            monitor("left", PhysicalRect::new(-1280, 0, 0, 1024).unwrap()),
            monitor("primary", PhysicalRect::new(0, -200, 1920, 1080).unwrap()),
        ])
        .unwrap();
        assert_eq!(
            topology.virtual_bounds,
            PhysicalRect::new(-1280, -200, 1920, 1080).unwrap()
        );
    }

    #[test]
    fn topology_rejects_empty_and_duplicate_monitor_ids() {
        assert_eq!(
            MonitorTopology::new(Vec::new()),
            Err(ScreenshotError::InvalidTopology)
        );
        let bounds = PhysicalRect::new(0, 0, 10, 10).unwrap();
        assert_eq!(
            MonitorTopology::new(vec![monitor("same", bounds), monitor("same", bounds)]),
            Err(ScreenshotError::DuplicateMonitor("same".to_owned()))
        );
    }
}
