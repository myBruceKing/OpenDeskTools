pub mod fake;
pub mod gdi;

use std::time::{SystemTime, UNIX_EPOCH};

use super::model::{
    BackendReport, CaptureBackendName, MonitorDescriptor, MonitorFrame, VirtualDesktopSnapshot,
};
use super::monitor::MonitorTopology;
use super::{ScreenshotError, MAX_SNAPSHOT_BYTES};

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
