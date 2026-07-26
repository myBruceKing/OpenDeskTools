use super::{BackendCapability, CaptureBackend, CaptureOptions};
use crate::infrastructure::screenshot::model::{
    CaptureBackendName, MonitorDescriptor, MonitorFrame,
};
use crate::infrastructure::screenshot::monitor::MonitorTopology;
use crate::infrastructure::screenshot::ScreenshotError;

pub struct WgcCaptureBackend {
    #[cfg(windows)]
    runtime: Option<windows_impl::WgcRuntime>,
}

impl std::fmt::Debug for WgcCaptureBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WgcCaptureBackend")
            .finish_non_exhaustive()
    }
}

impl Default for WgcCaptureBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl WgcCaptureBackend {
    pub fn new() -> Self {
        Self {
            #[cfg(windows)]
            runtime: None,
        }
    }

    pub fn prepare(&mut self, topology: &MonitorTopology) -> Result<(), ScreenshotError> {
        #[cfg(windows)]
        {
            if self.runtime.is_none() {
                self.runtime = Some(windows_impl::WgcRuntime::new()?);
            }
            self.runtime
                .as_mut()
                .expect("WGC runtime was initialized")
                .prepare(topology)
        }
        #[cfg(not(windows))]
        {
            let _ = topology;
            Err(ScreenshotError::UnsupportedPlatform)
        }
    }
}

impl CaptureBackend for WgcCaptureBackend {
    fn name(&self) -> CaptureBackendName {
        CaptureBackendName::WindowsGraphicsCapture
    }

    fn probe(&self, topology: &MonitorTopology) -> BackendCapability {
        if topology.monitors.is_empty() {
            return BackendCapability::unavailable("monitor topology is empty");
        }
        #[cfg(windows)]
        {
            windows_impl::probe()
        }
        #[cfg(not(windows))]
        {
            BackendCapability::unavailable("Windows Graphics Capture is Windows-only")
        }
    }

    fn capture_monitor(
        &mut self,
        monitor: &MonitorDescriptor,
        options: &CaptureOptions,
    ) -> Result<MonitorFrame, ScreenshotError> {
        #[cfg(windows)]
        {
            if self.runtime.is_none() {
                self.runtime = Some(windows_impl::WgcRuntime::new()?);
            }
            self.runtime
                .as_mut()
                .expect("WGC runtime was initialized")
                .capture_monitor(monitor, options)
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
    use std::collections::HashMap;
    use std::thread;
    use std::time::{Duration, Instant};

    use windows::core::{factory, Interface};
    use windows::Graphics::Capture::{
        Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem,
        GraphicsCaptureSession,
    };
    use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
    use windows::Graphics::DirectX::DirectXPixelFormat;
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
    use windows::Win32::Graphics::Dxgi::IDXGIDevice;
    use windows::Win32::Graphics::Gdi::{MonitorFromPoint, HMONITOR, MONITOR_DEFAULTTONULL};
    use windows::Win32::System::WinRT::Direct3D11::{
        CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
    };
    use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

    use super::*;
    use crate::infrastructure::debug_qa;
    use crate::infrastructure::screenshot::backend::d3d11::{
        d3d_error, read_bgra_texture, DeviceResources,
    };
    use crate::infrastructure::screenshot::model::PhysicalRect;

    const FRAME_WAIT: Duration = Duration::from_millis(200);

    struct CachedCaptureItem {
        bounds: PhysicalRect,
        item: GraphicsCaptureItem,
    }

    pub(super) struct WgcRuntime {
        resources: DeviceResources,
        items: HashMap<String, CachedCaptureItem>,
    }

    impl WgcRuntime {
        pub(super) fn new() -> Result<Self, ScreenshotError> {
            let resources = DeviceResources::default_hardware()?;
            Ok(Self {
                resources,
                items: HashMap::new(),
            })
        }

        pub(super) fn prepare(
            &mut self,
            topology: &MonitorTopology,
        ) -> Result<(), ScreenshotError> {
            for monitor in &topology.monitors {
                let _ = self.capture_item(monitor)?;
            }
            Ok(())
        }

        pub(super) fn capture_monitor(
            &mut self,
            monitor: &MonitorDescriptor,
            options: &CaptureOptions,
        ) -> Result<MonitorFrame, ScreenshotError> {
            let item = self.capture_item(monitor)?;
            let direct3d_device = create_direct3d_device(&self.resources)?;
            capture_monitor(monitor, options, &self.resources, &direct3d_device, item)
        }

        fn capture_item(
            &mut self,
            monitor: &MonitorDescriptor,
        ) -> Result<GraphicsCaptureItem, ScreenshotError> {
            if let Some(cached) = self.items.get(&monitor.id) {
                if cached.bounds == monitor.physical_bounds
                    && capture_item_matches_monitor(&cached.item, monitor)
                {
                    return Ok(cached.item.clone());
                }
            }
            self.items.remove(&monitor.id);

            let monitor_handle = monitor_handle(monitor)?;
            let item_interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
                .map_err(|error| wgc_error("GraphicsCaptureItem activation factory", error))?;
            let item: GraphicsCaptureItem =
                unsafe { item_interop.CreateForMonitor(monitor_handle) }
                    .map_err(|error| wgc_error("CreateForMonitor", error))?;
            self.items.insert(
                monitor.id.clone(),
                CachedCaptureItem {
                    bounds: monitor.physical_bounds,
                    item: item.clone(),
                },
            );
            Ok(item)
        }
    }

    fn capture_item_matches_monitor(
        item: &GraphicsCaptureItem,
        monitor: &MonitorDescriptor,
    ) -> bool {
        let Some(expected_width) = monitor.physical_bounds.width() else {
            return false;
        };
        let Some(expected_height) = monitor.physical_bounds.height() else {
            return false;
        };
        let Ok(expected_width) = i32::try_from(expected_width) else {
            return false;
        };
        let Ok(expected_height) = i32::try_from(expected_height) else {
            return false;
        };
        item.Size()
            .is_ok_and(|size| size.Width == expected_width && size.Height == expected_height)
    }

    pub(super) fn probe() -> BackendCapability {
        debug_qa::trace!("screenshot wgc probe stage=loading_factory");
        match factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>() {
            Ok(_) => BackendCapability::available("Windows Graphics Capture"),
            Err(error) => {
                BackendCapability::unavailable(format!("WGC activation factory failed: {error}"))
            }
        }
    }

    fn capture_monitor(
        monitor: &MonitorDescriptor,
        options: &CaptureOptions,
        resources: &DeviceResources,
        direct3d_device: &IDirect3DDevice,
        item: GraphicsCaptureItem,
    ) -> Result<MonitorFrame, ScreenshotError> {
        monitor.validate()?;
        let size = item
            .Size()
            .map_err(|error| wgc_error("GraphicsCaptureItem::Size", error))?;
        let expected_width = i32::try_from(
            monitor
                .physical_bounds
                .width()
                .ok_or(ScreenshotError::InvalidTopology)?,
        )
        .map_err(|_| ScreenshotError::InvalidTopology)?;
        let expected_height = i32::try_from(
            monitor
                .physical_bounds
                .height()
                .ok_or(ScreenshotError::InvalidTopology)?,
        )
        .map_err(|_| ScreenshotError::InvalidTopology)?;
        if size.Width != expected_width || size.Height != expected_height {
            return Err(ScreenshotError::BackendUnavailable(format!(
                "WGC item size mismatch for {}: item={}x{}, expected={}x{}",
                monitor.id, size.Width, size.Height, expected_width, expected_height
            )));
        }

        let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            direct3d_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )
        .map_err(|error| wgc_error("Direct3D11CaptureFramePool::CreateFreeThreaded", error))?;
        let session = frame_pool
            .CreateCaptureSession(&item)
            .map_err(|error| wgc_error("CreateCaptureSession", error))?;
        if !options.include_cursor {
            session
                .SetIsCursorCaptureEnabled(false)
                .map_err(|error| wgc_error("SetIsCursorCaptureEnabled(false)", error))?;
        }
        session
            .StartCapture()
            .map_err(|error| wgc_error("GraphicsCaptureSession::StartCapture", error))?;
        let _session_guard = CaptureSessionGuard {
            session: &session,
            frame_pool: &frame_pool,
        };

        let started = Instant::now();
        let frame = loop {
            match frame_pool.TryGetNextFrame() {
                Ok(frame) => break frame,
                Err(_) if started.elapsed() < FRAME_WAIT => thread::sleep(Duration::from_millis(1)),
                Err(error) => {
                    return Err(ScreenshotError::BackendUnavailable(format!(
                        "WGC timed out waiting for a frame on {} after {} ms: {error}",
                        monitor.id,
                        FRAME_WAIT.as_millis()
                    )));
                }
            }
        };
        let _frame_guard = CaptureFrameGuard(&frame);
        let content_size = frame
            .ContentSize()
            .map_err(|error| wgc_error("Direct3D11CaptureFrame::ContentSize", error))?;
        debug_qa::trace!(format!(
            "screenshot wgc frame monitor={} content={}x{} elapsed_ms={}",
            monitor.id,
            content_size.Width,
            content_size.Height,
            started.elapsed().as_millis()
        ));
        let surface = frame
            .Surface()
            .map_err(|error| wgc_error("Direct3D11CaptureFrame::Surface", error))?;
        let access: IDirect3DDxgiInterfaceAccess = surface
            .cast()
            .map_err(|error| wgc_error("IDirect3DDxgiInterfaceAccess cast", error))?;
        let texture: ID3D11Texture2D = unsafe { access.GetInterface() }
            .map_err(|error| wgc_error("IDirect3DDxgiInterfaceAccess::GetInterface", error))?;
        read_bgra_texture("WGC", monitor, resources, &texture)
    }

    fn create_direct3d_device(
        resources: &DeviceResources,
    ) -> Result<IDirect3DDevice, ScreenshotError> {
        let dxgi_device: IDXGIDevice = resources
            .device
            .cast()
            .map_err(|error| d3d_error("ID3D11Device to IDXGIDevice cast", error))?;
        let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device) }
            .map_err(|error| wgc_error("CreateDirect3D11DeviceFromDXGIDevice", error))?;
        inspectable
            .cast()
            .map_err(|error| wgc_error("IDirect3DDevice cast", error))
    }

    fn monitor_handle(monitor: &MonitorDescriptor) -> Result<HMONITOR, ScreenshotError> {
        let point = POINT {
            x: monitor.physical_bounds.left,
            y: monitor.physical_bounds.top,
        };
        let handle = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONULL) };
        if handle.is_invalid() {
            return Err(ScreenshotError::MonitorUnavailable(monitor.id.clone()));
        }
        Ok(handle)
    }

    fn wgc_error(stage: &str, error: windows::core::Error) -> ScreenshotError {
        ScreenshotError::BackendUnavailable(format!("WGC {stage} failed: {error}"))
    }

    struct CaptureSessionGuard<'a> {
        session: &'a GraphicsCaptureSession,
        frame_pool: &'a Direct3D11CaptureFramePool,
    }

    impl Drop for CaptureSessionGuard<'_> {
        fn drop(&mut self) {
            let _ = self.session.Close();
            let _ = self.frame_pool.Close();
        }
    }

    struct CaptureFrameGuard<'a>(&'a Direct3D11CaptureFrame);

    impl Drop for CaptureFrameGuard<'_> {
        fn drop(&mut self) {
            let _ = self.0.Close();
        }
    }
}
