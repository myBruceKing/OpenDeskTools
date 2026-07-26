use super::{BackendCapability, CaptureBackend, CaptureOptions};
use crate::infrastructure::screenshot::model::{
    CaptureBackendName, MonitorDescriptor, MonitorFrame,
};
use crate::infrastructure::screenshot::monitor::MonitorTopology;
use crate::infrastructure::screenshot::ScreenshotError;

pub struct DxgiCaptureBackend {
    #[cfg(windows)]
    runtime: Option<windows_impl::DxgiRuntime>,
}

impl std::fmt::Debug for DxgiCaptureBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DxgiCaptureBackend")
            .finish_non_exhaustive()
    }
}

impl Default for DxgiCaptureBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl DxgiCaptureBackend {
    pub fn new() -> Self {
        Self {
            #[cfg(windows)]
            runtime: None,
        }
    }
}

impl CaptureBackend for DxgiCaptureBackend {
    fn name(&self) -> CaptureBackendName {
        CaptureBackendName::Dxgi
    }

    fn probe(&self, topology: &MonitorTopology) -> BackendCapability {
        if topology.monitors.is_empty() {
            return BackendCapability::unavailable("monitor topology is empty");
        }
        #[cfg(windows)]
        {
            windows_impl::probe(topology)
        }
        #[cfg(not(windows))]
        {
            BackendCapability::unavailable("DXGI desktop duplication is Windows-only")
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
                self.runtime = Some(windows_impl::DxgiRuntime::new()?);
            }
            self.runtime
                .as_mut()
                .expect("DXGI runtime was initialized")
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
    use std::time::{Duration, Instant};

    use windows::core::Interface;
    use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
    use windows::Win32::Graphics::Dxgi::Common::DXGI_MODE_ROTATION_IDENTITY;
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication,
        IDXGIResource, DXGI_ERROR_NOT_FOUND, DXGI_OUTDUPL_FRAME_INFO,
    };

    use super::*;
    use crate::infrastructure::debug_qa;
    use crate::infrastructure::screenshot::backend::d3d11::{read_bgra_texture, DeviceResources};

    const FRAME_WAIT: Duration = Duration::from_millis(100);

    pub(super) struct DxgiRuntime {
        factory: IDXGIFactory1,
        devices: HashMap<u32, DeviceResources>,
    }

    impl DxgiRuntime {
        pub(super) fn new() -> Result<Self, ScreenshotError> {
            let factory = unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }
                .map_err(|error| dxgi_error("CreateDXGIFactory1", error))?;
            Ok(Self {
                factory,
                devices: HashMap::new(),
            })
        }

        pub(super) fn capture_monitor(
            &mut self,
            monitor: &MonitorDescriptor,
            options: &CaptureOptions,
        ) -> Result<MonitorFrame, ScreenshotError> {
            if options.include_cursor {
                return Err(ScreenshotError::BackendUnavailable(
                    "DXGI cursor composition is not implemented".to_owned(),
                ));
            }
            monitor.validate()?;

            let (adapter_index, adapter, output) = find_output(&self.factory, &monitor.id)?;
            let resources = match self.devices.get(&adapter_index) {
                Some(resources) => resources.clone(),
                None => {
                    let resources = DeviceResources::for_adapter(&adapter)?;
                    self.devices.insert(adapter_index, resources.clone());
                    resources
                }
            };
            capture_output(monitor, &resources, &output)
        }
    }

    pub(super) fn probe(topology: &MonitorTopology) -> BackendCapability {
        let factory = match unsafe { CreateDXGIFactory1::<IDXGIFactory1>() } {
            Ok(factory) => factory,
            Err(error) => {
                return BackendCapability::unavailable(format!(
                    "CreateDXGIFactory1 failed: {error}"
                ))
            }
        };
        for monitor in &topology.monitors {
            if let Err(error) = find_output(&factory, &monitor.id) {
                return BackendCapability::unavailable(error.to_string());
            }
        }
        BackendCapability::available("DXGI Desktop Duplication")
    }

    fn capture_output(
        monitor: &MonitorDescriptor,
        resources: &DeviceResources,
        output: &IDXGIOutput1,
    ) -> Result<MonitorFrame, ScreenshotError> {
        let duplication = unsafe { output.DuplicateOutput(&resources.device) }
            .map_err(|error| dxgi_error("DuplicateOutput", error))?;
        let duplication_desc = unsafe { duplication.GetDesc() };
        if duplication_desc.Rotation != DXGI_MODE_ROTATION_IDENTITY {
            return Err(ScreenshotError::BackendUnavailable(format!(
                "DXGI output {} is rotated; using GDI fallback",
                monitor.id
            )));
        }

        let started = Instant::now();
        let (frame_info, desktop_resource) = loop {
            let elapsed = started.elapsed();
            let Some(remaining) = FRAME_WAIT.checked_sub(elapsed) else {
                return Err(ScreenshotError::BackendUnavailable(format!(
                    "DXGI timed out waiting for a desktop frame on {}",
                    monitor.id
                )));
            };
            let timeout_ms = remaining.as_millis().clamp(1, u128::from(u32::MAX)) as u32;
            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut desktop_resource: Option<IDXGIResource> = None;
            unsafe {
                duplication.AcquireNextFrame(timeout_ms, &mut frame_info, &mut desktop_resource)
            }
            .map_err(|error| dxgi_error("AcquireNextFrame", error))?;
            if frame_info.LastPresentTime != 0 && frame_info.AccumulatedFrames > 0 {
                break (frame_info, desktop_resource);
            }
            let _ = unsafe { duplication.ReleaseFrame() };
        };
        debug_qa::trace!(format!(
            "screenshot dxgi frame monitor={} last_present={} accumulated={} protected_masked={} pointer_visible={}",
            monitor.id,
            frame_info.LastPresentTime,
            frame_info.AccumulatedFrames,
            frame_info.ProtectedContentMaskedOut.as_bool(),
            frame_info.PointerPosition.Visible.as_bool()
        ));
        let _frame_guard = AcquiredFrame::new(&duplication);
        let desktop_resource = desktop_resource.ok_or_else(|| {
            ScreenshotError::BackendUnavailable(
                "DXGI AcquireNextFrame returned no desktop resource".to_owned(),
            )
        })?;
        let desktop_texture: ID3D11Texture2D = desktop_resource
            .cast()
            .map_err(|error| dxgi_error("desktop resource cast", error))?;
        read_bgra_texture("DXGI", monitor, resources, &desktop_texture)
    }

    fn find_output(
        factory: &IDXGIFactory1,
        monitor_id: &str,
    ) -> Result<(u32, IDXGIAdapter1, IDXGIOutput1), ScreenshotError> {
        let mut adapter_index = 0u32;
        loop {
            let adapter = match unsafe { factory.EnumAdapters1(adapter_index) } {
                Ok(adapter) => adapter,
                Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(error) => return Err(dxgi_error("EnumAdapters1", error)),
            };
            let mut output_index = 0u32;
            loop {
                let output = match unsafe { adapter.EnumOutputs(output_index) } {
                    Ok(output) => output,
                    Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                    Err(error) => return Err(dxgi_error("EnumOutputs", error)),
                };
                let description = unsafe { output.GetDesc() }
                    .map_err(|error| dxgi_error("IDXGIOutput::GetDesc", error))?;
                if description.AttachedToDesktop.as_bool()
                    && wide_array_to_string(&description.DeviceName)
                        .eq_ignore_ascii_case(monitor_id)
                {
                    let output: IDXGIOutput1 = output
                        .cast()
                        .map_err(|error| dxgi_error("IDXGIOutput1 cast", error))?;
                    return Ok((adapter_index, adapter, output));
                }
                output_index = output_index
                    .checked_add(1)
                    .ok_or(ScreenshotError::ArithmeticOverflow)?;
            }
            adapter_index = adapter_index
                .checked_add(1)
                .ok_or(ScreenshotError::ArithmeticOverflow)?;
        }
        Err(ScreenshotError::MonitorUnavailable(monitor_id.to_owned()))
    }

    fn dxgi_error(stage: &str, error: windows::core::Error) -> ScreenshotError {
        ScreenshotError::BackendUnavailable(format!("DXGI {stage} failed: {error}"))
    }

    fn wide_array_to_string(value: &[u16]) -> String {
        let end = value
            .iter()
            .position(|character| *character == 0)
            .unwrap_or(value.len());
        String::from_utf16_lossy(&value[..end])
    }

    struct AcquiredFrame<'a> {
        duplication: &'a IDXGIOutputDuplication,
    }

    impl<'a> AcquiredFrame<'a> {
        fn new(duplication: &'a IDXGIOutputDuplication) -> Self {
            Self { duplication }
        }
    }

    impl Drop for AcquiredFrame<'_> {
        fn drop(&mut self) {
            let _ = unsafe { self.duplication.ReleaseFrame() };
        }
    }
}
