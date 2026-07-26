#[cfg(windows)]
mod d3d11;
pub mod dxgi;
pub mod fake;
pub mod gdi;
pub mod wgc;

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::model::{
    BackendReport, CaptureBackendName, MonitorDescriptor, MonitorFrame, VirtualDesktopSnapshot,
};
use super::monitor::MonitorTopology;
use super::{ScreenshotError, MAX_SNAPSHOT_BYTES};
use crate::infrastructure::debug_qa;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CaptureOptions {
    pub include_cursor: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendCapability {
    pub available: bool,
    pub detail: String,
}

impl BackendCapability {
    pub fn available(detail: impl Into<String>) -> Self {
        Self {
            available: true,
            detail: detail.into(),
        }
    }

    pub fn unavailable(detail: impl Into<String>) -> Self {
        Self {
            available: false,
            detail: detail.into(),
        }
    }
}

pub trait CaptureBackend: Send {
    fn name(&self) -> CaptureBackendName;
    fn probe(&self, topology: &MonitorTopology) -> BackendCapability;
    fn capture_monitor(
        &mut self,
        monitor: &MonitorDescriptor,
        options: &CaptureOptions,
    ) -> Result<MonitorFrame, ScreenshotError>;
}

#[derive(Debug, Default)]
pub struct PreferredCaptureBackends {
    pub wgc: wgc::WgcCaptureBackend,
    pub dxgi: dxgi::DxgiCaptureBackend,
}

impl PreferredCaptureBackends {
    pub fn new() -> Self {
        Self {
            wgc: wgc::WgcCaptureBackend::new(),
            dxgi: dxgi::DxgiCaptureBackend::new(),
        }
    }

    pub fn prepare(&mut self, topology: &MonitorTopology) -> Result<(), ScreenshotError> {
        self.wgc.prepare(topology)
    }
}

pub fn capture_snapshot(
    backend: &mut dyn CaptureBackend,
    topology: &MonitorTopology,
    generation: u64,
    options: &CaptureOptions,
) -> Result<VirtualDesktopSnapshot, ScreenshotError> {
    let capability = backend.probe(topology);
    if !capability.available {
        return Err(ScreenshotError::BackendUnavailable(capability.detail));
    }
    let backend_name = backend.name();
    let mut frames = Vec::with_capacity(topology.monitors.len());
    let mut reports = Vec::with_capacity(topology.monitors.len());
    let mut total_bytes = 0usize;
    for monitor in &topology.monitors {
        let frame = backend.capture_monitor(monitor, options)?;
        frame.validate()?;
        total_bytes = total_bytes
            .checked_add(frame.byte_len())
            .ok_or(ScreenshotError::ArithmeticOverflow)?;
        if total_bytes > MAX_SNAPSHOT_BYTES {
            return Err(ScreenshotError::MemoryLimit);
        }
        reports.push(BackendReport {
            monitor_id: monitor.id.clone(),
            backend: backend_name.clone(),
            detail: capability.detail.clone(),
        });
        frames.push(frame);
    }
    let captured_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64);
    VirtualDesktopSnapshot::new(generation, captured_at_ms, frames, reports)
}

pub fn capture_preferred_snapshot(
    topology: &MonitorTopology,
    generation: u64,
    options: &CaptureOptions,
) -> Result<VirtualDesktopSnapshot, ScreenshotError> {
    let mut backends = PreferredCaptureBackends::new();
    capture_preferred_snapshot_with(&mut backends, topology, generation, options)
}

pub fn capture_preferred_snapshot_with(
    backends: &mut PreferredCaptureBackends,
    topology: &MonitorTopology,
    generation: u64,
    options: &CaptureOptions,
) -> Result<VirtualDesktopSnapshot, ScreenshotError> {
    let started = Instant::now();
    let mut gdi = gdi::GdiCaptureBackend::new();
    let wgc_capability = backends.wgc.probe(topology);
    let dxgi_capability = backends.dxgi.probe(topology);
    let gdi_capability = gdi.probe(topology);
    if !wgc_capability.available && !dxgi_capability.available && !gdi_capability.available {
        return Err(ScreenshotError::BackendUnavailable(format!(
            "WGC unavailable: {}; DXGI unavailable: {}; GDI unavailable: {}",
            wgc_capability.detail, dxgi_capability.detail, gdi_capability.detail
        )));
    }

    let mut frames = Vec::with_capacity(topology.monitors.len());
    let mut reports = Vec::with_capacity(topology.monitors.len());
    let mut total_bytes = 0usize;
    for monitor in &topology.monitors {
        let mut failures = Vec::new();
        let captured = if wgc_capability.available {
            match backends.wgc.capture_monitor(monitor, options) {
                Ok(frame) => Some((
                    frame,
                    CaptureBackendName::WindowsGraphicsCapture,
                    wgc_capability.detail.clone(),
                )),
                Err(error) => {
                    debug_qa::trace!(format!(
                        "screenshot monitor={} backend=wgc result=fallback error={error}",
                        monitor.id
                    ));
                    failures.push(format!("WGC: {error}"));
                    None
                }
            }
        } else {
            failures.push(format!("WGC unavailable: {}", wgc_capability.detail));
            None
        };
        let captured = match captured {
            Some(captured) => captured,
            None if dxgi_capability.available => {
                match backends.dxgi.capture_monitor(monitor, options) {
                    Ok(frame) => (
                        frame,
                        CaptureBackendName::Dxgi,
                        format!("DXGI fallback after {}", failures.join("; ")),
                    ),
                    Err(error) => {
                        debug_qa::trace!(format!(
                            "screenshot monitor={} backend=dxgi result=fallback error={error}",
                            monitor.id
                        ));
                        failures.push(format!("DXGI: {error}"));
                        let frame = gdi.capture_monitor(monitor, options).map_err(|gdi_error| {
                            ScreenshotError::BackendUnavailable(format!(
                                "capture failed for {}: {}; GDI: {gdi_error}",
                                monitor.id,
                                failures.join("; ")
                            ))
                        })?;
                        (
                            frame,
                            CaptureBackendName::Gdi,
                            format!("GDI fallback after {}", failures.join("; ")),
                        )
                    }
                }
            }
            None => {
                failures.push(format!("DXGI unavailable: {}", dxgi_capability.detail));
                let frame = gdi.capture_monitor(monitor, options).map_err(|gdi_error| {
                    ScreenshotError::BackendUnavailable(format!(
                        "capture failed for {}: {}; GDI: {gdi_error}",
                        monitor.id,
                        failures.join("; ")
                    ))
                })?;
                (
                    frame,
                    CaptureBackendName::Gdi,
                    format!("GDI fallback after {}", failures.join("; ")),
                )
            }
        };
        let (frame, backend, detail) = captured;
        frame.validate()?;
        total_bytes = total_bytes
            .checked_add(frame.byte_len())
            .ok_or(ScreenshotError::ArithmeticOverflow)?;
        if total_bytes > MAX_SNAPSHOT_BYTES {
            return Err(ScreenshotError::MemoryLimit);
        }
        reports.push(BackendReport {
            monitor_id: monitor.id.clone(),
            backend,
            detail,
        });
        frames.push(frame);
    }
    let captured_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64);
    let snapshot = VirtualDesktopSnapshot::new(generation, captured_at_ms, frames, reports)?;
    let wgc_count = snapshot
        .backend_report
        .iter()
        .filter(|report| report.backend == CaptureBackendName::WindowsGraphicsCapture)
        .count();
    let dxgi_count = snapshot
        .backend_report
        .iter()
        .filter(|report| report.backend == CaptureBackendName::Dxgi)
        .count();
    let gdi_count = snapshot
        .backend_report
        .len()
        .saturating_sub(wgc_count + dxgi_count);
    debug_qa::trace!(format!(
        "screenshot snapshot result=success wgc_monitors={wgc_count} dxgi_monitors={dxgi_count} gdi_monitors={gdi_count} elapsed_ms={}",
        started.elapsed().as_millis()
    ));
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::fake::FakeCaptureBackend;
    use super::*;
    use crate::infrastructure::screenshot::model::{DisplayRotation, PhysicalRect};

    fn frame(id: &str, bounds: PhysicalRect, bgra: [u8; 4]) -> MonitorFrame {
        let width = bounds.width().unwrap();
        let height = bounds.height().unwrap();
        MonitorFrame {
            monitor: MonitorDescriptor {
                id: id.to_owned(),
                physical_bounds: bounds,
                work_bounds: bounds,
                dpi_x: 96,
                dpi_y: 96,
                rotation: DisplayRotation::Identity,
                is_primary: true,
            },
            width,
            height,
            stride: width as usize * 4,
            bgra: bgra.repeat(width as usize * height as usize),
        }
    }

    #[test]
    fn fake_backend_exercises_replaceable_capture_contract() {
        let bounds = PhysicalRect::new(-2, 3, 0, 4).unwrap();
        let expected = frame("fixture", bounds, [1, 2, 3, 255]);
        let topology = MonitorTopology::new(vec![expected.monitor.clone()]).unwrap();
        let mut backend = FakeCaptureBackend::new(vec![expected.clone()]).unwrap();
        let snapshot =
            capture_snapshot(&mut backend, &topology, 9, &CaptureOptions::default()).unwrap();
        assert_eq!(snapshot.generation, 9);
        assert_eq!(snapshot.frames, vec![expected]);
        assert_eq!(snapshot.backend_report[0].backend, CaptureBackendName::Fake);
    }
}
