use std::collections::HashMap;

use super::{BackendCapability, CaptureBackend, CaptureOptions};
use crate::infrastructure::screenshot::model::{
    CaptureBackendName, MonitorDescriptor, MonitorFrame,
};
use crate::infrastructure::screenshot::monitor::MonitorTopology;
use crate::infrastructure::screenshot::ScreenshotError;

#[derive(Debug, Clone)]
pub struct FakeCaptureBackend {
    frames: HashMap<String, MonitorFrame>,
}

impl FakeCaptureBackend {
    pub fn new(frames: Vec<MonitorFrame>) -> Result<Self, ScreenshotError> {
        let mut by_monitor = HashMap::with_capacity(frames.len());
        for frame in frames {
            frame.validate()?;
            let monitor_id = frame.monitor.id.clone();
            if by_monitor.insert(monitor_id.clone(), frame).is_some() {
                return Err(ScreenshotError::DuplicateMonitor(monitor_id));
            }
        }
        Ok(Self { frames: by_monitor })
    }
}

impl CaptureBackend for FakeCaptureBackend {
    fn name(&self) -> CaptureBackendName {
        CaptureBackendName::Fake
    }

    fn probe(&self, topology: &MonitorTopology) -> BackendCapability {
        let missing = topology
            .monitors
            .iter()
            .find(|monitor| !self.frames.contains_key(&monitor.id));
        match missing {
            Some(monitor) => {
                BackendCapability::unavailable(format!("fixture missing {}", monitor.id))
            }
            None => BackendCapability::available("deterministic fixture"),
        }
    }

    fn capture_monitor(
        &mut self,
        monitor: &MonitorDescriptor,
        _options: &CaptureOptions,
    ) -> Result<MonitorFrame, ScreenshotError> {
        self.frames
            .get(&monitor.id)
            .cloned()
            .ok_or_else(|| ScreenshotError::MonitorUnavailable(monitor.id.clone()))
    }
}
