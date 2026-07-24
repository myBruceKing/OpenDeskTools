use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{ScreenshotError, MAX_SNAPSHOT_BYTES};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhysicalPoint {
    pub x: i32,
    pub y: i32,
}

impl PhysicalPoint {
    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhysicalRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PhysicalRect {
    pub fn new(left: i32, top: i32, right: i32, bottom: i32) -> Result<Self, ScreenshotError> {
        let rect = Self {
            left,
            top,
            right,
            bottom,
        };
        if rect.width().is_none() || rect.height().is_none() {
            return Err(ScreenshotError::InvalidRect);
        }
        Ok(rect)
    }

    pub fn width(self) -> Option<u32> {
        positive_distance(self.left, self.right)
    }

    pub fn height(self) -> Option<u32> {
        positive_distance(self.top, self.bottom)
    }

    pub fn intersection(self, other: Self) -> Option<Self> {
        Self::new(
            self.left.max(other.left),
            self.top.max(other.top),
            self.right.min(other.right),
            self.bottom.min(other.bottom),
        )
        .ok()
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }
}

fn positive_distance(start: i32, end: i32) -> Option<u32> {
    let distance = i64::from(end).checked_sub(i64::from(start))?;
    if distance <= 0 {
        return None;
    }
    u32::try_from(distance).ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayRotation {
    Identity,
    Rotate90,
    Rotate180,
    Rotate270,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorDescriptor {
    pub id: String,
    pub physical_bounds: PhysicalRect,
    pub work_bounds: PhysicalRect,
    pub dpi_x: u32,
    pub dpi_y: u32,
    pub rotation: DisplayRotation,
    pub is_primary: bool,
}

impl MonitorDescriptor {
    pub fn validate(&self) -> Result<(), ScreenshotError> {
        if self.id.trim().is_empty()
            || self.dpi_x == 0
            || self.dpi_y == 0
            || self.physical_bounds.width().is_none()
            || self.physical_bounds.height().is_none()
            || self.work_bounds.width().is_none()
            || self.work_bounds.height().is_none()
            || self
                .physical_bounds
                .intersection(self.work_bounds)
                .is_none()
        {
            return Err(ScreenshotError::InvalidTopology);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureBackendName {
    Fake,
    Gdi,
    Dxgi,
    WindowsGraphicsCapture,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendReport {
    pub monitor_id: String,
    pub backend: CaptureBackendName,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorFrame {
    pub monitor: MonitorDescriptor,
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub bgra: Vec<u8>,
}

impl MonitorFrame {
    pub fn validate(&self) -> Result<(), ScreenshotError> {
        self.monitor.validate()?;
        let expected_width = self
            .monitor
            .physical_bounds
            .width()
            .ok_or(ScreenshotError::InvalidFrame)?;
        let expected_height = self
            .monitor
            .physical_bounds
            .height()
            .ok_or(ScreenshotError::InvalidFrame)?;
        if self.width != expected_width || self.height != expected_height {
            return Err(ScreenshotError::InvalidFrame);
        }
        let minimum_stride = usize::try_from(self.width)
            .ok()
            .and_then(|width| width.checked_mul(4))
            .ok_or(ScreenshotError::ArithmeticOverflow)?;
        if self.stride < minimum_stride {
            return Err(ScreenshotError::InvalidFrame);
        }
        let expected_bytes = self
            .stride
            .checked_mul(
                usize::try_from(self.height).map_err(|_| ScreenshotError::ArithmeticOverflow)?,
            )
            .ok_or(ScreenshotError::ArithmeticOverflow)?;
        if self.bgra.len() != expected_bytes {
            return Err(ScreenshotError::InvalidFrame);
        }
        Ok(())
    }

    pub fn byte_len(&self) -> usize {
        self.bgra.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualDesktopSnapshot {
    pub generation: u64,
    pub virtual_bounds: PhysicalRect,
    pub frames: Vec<MonitorFrame>,
    pub captured_at_ms: u64,
    pub backend_report: Vec<BackendReport>,
}

impl VirtualDesktopSnapshot {
    pub fn new(
        generation: u64,
        captured_at_ms: u64,
        frames: Vec<MonitorFrame>,
        backend_report: Vec<BackendReport>,
    ) -> Result<Self, ScreenshotError> {
        let first = frames.first().ok_or(ScreenshotError::InvalidTopology)?;
        if !backend_report.is_empty() && backend_report.len() != frames.len() {
            return Err(ScreenshotError::InvalidFrame);
        }
        let mut virtual_bounds = first.monitor.physical_bounds;
        let mut monitor_ids = HashSet::with_capacity(frames.len());
        let mut total_bytes = 0usize;
        for frame in &frames {
            frame.validate()?;
            if !monitor_ids.insert(frame.monitor.id.clone()) {
                return Err(ScreenshotError::DuplicateMonitor(frame.monitor.id.clone()));
            }
            virtual_bounds = virtual_bounds.union(frame.monitor.physical_bounds);
            total_bytes = total_bytes
                .checked_add(frame.byte_len())
                .ok_or(ScreenshotError::ArithmeticOverflow)?;
            if total_bytes > MAX_SNAPSHOT_BYTES {
                return Err(ScreenshotError::MemoryLimit);
            }
        }
        for (frame, report) in frames.iter().zip(backend_report.iter()) {
            if report.monitor_id != frame.monitor.id {
                return Err(ScreenshotError::InvalidFrame);
            }
        }
        Ok(Self {
            generation,
            virtual_bounds,
            frames,
            captured_at_ms,
            backend_report,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl CapturedImage {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, ScreenshotError> {
        let expected = usize::try_from(width)
            .ok()
            .and_then(|width| width.checked_mul(usize::try_from(height).ok()?))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(ScreenshotError::ArithmeticOverflow)?;
        if width == 0 || height == 0 || rgba.len() != expected {
            return Err(ScreenshotError::InvalidFrame);
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
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

    fn frame(id: &str, bounds: PhysicalRect) -> MonitorFrame {
        let width = bounds.width().unwrap();
        let height = bounds.height().unwrap();
        let stride = width as usize * 4;
        MonitorFrame {
            monitor: monitor(id, bounds),
            width,
            height,
            stride,
            bgra: vec![0; stride * height as usize],
        }
    }

    #[test]
    fn physical_rect_supports_negative_virtual_desktop_coordinates() {
        let left = PhysicalRect::new(-1920, -120, 0, 960).unwrap();
        let right = PhysicalRect::new(0, 0, 2560, 1440).unwrap();
        assert_eq!(left.width(), Some(1920));
        assert_eq!(left.height(), Some(1080));
        assert_eq!(left.intersection(right), None);
        assert_eq!(
            left.union(right),
            PhysicalRect::new(-1920, -120, 2560, 1440).unwrap()
        );
    }

    #[test]
    fn physical_rect_rejects_empty_and_inverted_bounds() {
        assert_eq!(
            PhysicalRect::new(0, 0, 0, 1),
            Err(ScreenshotError::InvalidRect)
        );
        assert_eq!(
            PhysicalRect::new(10, 0, 5, 1),
            Err(ScreenshotError::InvalidRect)
        );
    }

    #[test]
    fn snapshot_derives_virtual_bounds_and_rejects_duplicate_monitors() {
        let left_bounds = PhysicalRect::new(-2, 0, 0, 2).unwrap();
        let right_bounds = PhysicalRect::new(0, 0, 3, 2).unwrap();
        let snapshot = VirtualDesktopSnapshot::new(
            3,
            100,
            vec![frame("left", left_bounds), frame("right", right_bounds)],
            Vec::new(),
        )
        .unwrap();
        assert_eq!(
            snapshot.virtual_bounds,
            PhysicalRect::new(-2, 0, 3, 2).unwrap()
        );
        assert_eq!(
            VirtualDesktopSnapshot::new(
                4,
                101,
                vec![frame("same", left_bounds), frame("same", right_bounds)],
                Vec::new(),
            ),
            Err(ScreenshotError::DuplicateMonitor("same".to_owned()))
        );
    }

    #[test]
    fn monitor_frame_requires_exact_dimensions_stride_and_byte_count() {
        let bounds = PhysicalRect::new(0, 0, 2, 2).unwrap();
        let mut value = frame("display", bounds);
        value.bgra.pop();
        assert_eq!(value.validate(), Err(ScreenshotError::InvalidFrame));
    }
}
