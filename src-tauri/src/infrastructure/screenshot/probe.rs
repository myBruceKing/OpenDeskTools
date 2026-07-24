use std::path::PathBuf;

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::backend::gdi::GdiCaptureBackend;
use super::backend::{capture_snapshot, CaptureOptions};
use super::model::{CaptureBackendName, DisplayRotation, PhysicalRect};
use super::monitor::MonitorTopology;
use super::ScreenshotError;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotProbeReport {
    pub captured_at_ms: u64,
    pub virtual_bounds: PhysicalRect,
    pub total_bytes: usize,
    pub monitors: Vec<ScreenshotProbeMonitor>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotProbeMonitor {
    pub id: String,
    pub bounds: PhysicalRect,
    pub work_bounds: PhysicalRect,
    pub dpi_x: u32,
    pub dpi_y: u32,
    pub rotation: DisplayRotation,
    pub is_primary: bool,
    pub backend: CaptureBackendName,
    pub width: u32,
    pub height: u32,
    pub byte_len: usize,
    pub non_black_pixels: usize,
    pub sha256: String,
}

pub fn run_gdi_probe() -> Result<ScreenshotProbeReport, ScreenshotError> {
    let topology = MonitorTopology::query()?;
    let mut backend = GdiCaptureBackend::new();
    let snapshot = capture_snapshot(&mut backend, &topology, 1, &CaptureOptions::default())?;
    let mut total_bytes = 0usize;
    let monitors = snapshot
        .frames
        .iter()
        .zip(snapshot.backend_report.iter())
        .map(|(frame, backend)| {
            total_bytes = total_bytes.saturating_add(frame.bgra.len());
            ScreenshotProbeMonitor {
                id: frame.monitor.id.clone(),
                bounds: frame.monitor.physical_bounds,
                work_bounds: frame.monitor.work_bounds,
                dpi_x: frame.monitor.dpi_x,
                dpi_y: frame.monitor.dpi_y,
                rotation: frame.monitor.rotation,
                is_primary: frame.monitor.is_primary,
                backend: backend.backend.clone(),
                width: frame.width,
                height: frame.height,
                byte_len: frame.bgra.len(),
                non_black_pixels: frame
                    .bgra
                    .chunks_exact(4)
                    .filter(|pixel| pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0)
                    .count(),
                sha256: format!("{:x}", Sha256::digest(&frame.bgra)),
            }
        })
        .collect();
    Ok(ScreenshotProbeReport {
        captured_at_ms: snapshot.captured_at_ms,
        virtual_bounds: snapshot.virtual_bounds,
        total_bytes,
        monitors,
    })
}

pub fn write_report(report: &ScreenshotProbeReport) -> Result<PathBuf, std::io::Error> {
    let path = std::env::temp_dir().join(format!(
        "OpenDeskTools-screenshot-probe-{}.json",
        std::process::id()
    ));
    let bytes = serde_json::to_vec_pretty(report).map_err(std::io::Error::other)?;
    std::fs::write(&path, bytes)?;
    Ok(path)
}
