pub mod annotation;
pub mod backend;
pub mod candidate;
pub mod crop;
pub mod model;
pub mod monitor;
pub mod overlay;
pub mod selection;
pub mod service;

#[cfg(debug_assertions)]
pub mod probe;

use thiserror::Error;

pub const MAX_SNAPSHOT_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_CAPTURED_IMAGE_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ScreenshotError {
    #[error("physical rectangle is invalid")]
    InvalidRect,
    #[error("monitor topology is empty or invalid")]
    InvalidTopology,
    #[error("monitor identifier is duplicated: {0}")]
    DuplicateMonitor(String),
    #[error("monitor enumeration failed")]
    MonitorEnumerationFailed,
    #[error("monitor metadata is unavailable")]
    MonitorMetadataUnavailable,
    #[error("monitor is unavailable: {0}")]
    MonitorUnavailable(String),
    #[error("capture backend is unavailable: {0}")]
    BackendUnavailable(String),
    #[error("capture frame is invalid")]
    InvalidFrame,
    #[error("capture selection is outside the available desktop")]
    SelectionOutsideDesktop,
    #[error("capture exceeds the supported memory limit")]
    MemoryLimit,
    #[error("capture arithmetic overflowed")]
    ArithmeticOverflow,
    #[error("another screenshot session is already active")]
    SessionAlreadyActive,
    #[error("screenshot overlay state is unavailable")]
    OverlayStateUnavailable,
    #[error("Windows capture operation failed: {0}")]
    WindowsApi(&'static str),
    #[error("screenshot capture is unavailable on this platform")]
    UnsupportedPlatform,
}
