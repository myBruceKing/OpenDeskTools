use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

use super::annotation::apply_annotations;
use super::backend::gdi::GdiCaptureBackend;
use super::backend::{capture_snapshot, CaptureBackend, CaptureOptions};
use super::crop::crop_snapshot;
use super::model::CapturedImage;
use super::monitor::MonitorTopology;
use super::overlay::{self, CaptureAction};
use super::ScreenshotError;
use crate::infrastructure::clipboard::{
    ClipboardCaptureMetadata, ClipboardError, ClipboardService, ClipboardWriteContent,
};
use crate::infrastructure::clipboard_writer::{ClipboardWriter, ClipboardWriterError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenshotCaptureOutcome {
    Cancelled,
    Selected {
        image: CapturedImage,
        action: CaptureAction,
    },
}

#[derive(Debug, Error)]
pub enum ScreenshotServiceError {
    #[error(transparent)]
    Screenshot(#[from] ScreenshotError),
    #[error("failed to store screenshot in clipboard history: {0}")]
    Clipboard(#[from] ClipboardError),
    #[error("failed to update the system clipboard: {0}")]
    Writer(#[from] ClipboardWriterError),
}

#[derive(Debug)]
pub struct ScreenshotService {
    clipboard: Arc<ClipboardService>,
    writer: ClipboardWriter,
    session_active: AtomicBool,
    generation: AtomicU64,
}

impl ScreenshotService {
    pub fn new(clipboard: Arc<ClipboardService>) -> Self {
        Self {
            clipboard,
            writer: ClipboardWriter::default(),
            session_active: AtomicBool::new(false),
            generation: AtomicU64::new(0),
        }
    }

    pub fn probe(&self) -> Result<(), ScreenshotError> {
        let topology = MonitorTopology::query()?;
        let backend = GdiCaptureBackend::new();
        let capability = backend.probe(&topology);
        if !capability.available {
            return Err(ScreenshotError::BackendUnavailable(capability.detail));
        }
        overlay::probe()
    }

    pub fn capture_selection(&self) -> Result<ScreenshotCaptureOutcome, ScreenshotServiceError> {
        let _session = self.begin_session()?;
        let topology = MonitorTopology::query()?;
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let mut backend = GdiCaptureBackend::new();
        let snapshot = Arc::new(capture_snapshot(
            &mut backend,
            &topology,
            generation,
            &CaptureOptions::default(),
        )?);
        let Some(selection) = overlay::select(Arc::clone(&snapshot))? else {
            return Ok(ScreenshotCaptureOutcome::Cancelled);
        };
        let mut image = crop_snapshot(&snapshot, selection.rect)?;
        apply_annotations(&mut image, selection.rect, &selection.annotations)?;
        Ok(ScreenshotCaptureOutcome::Selected {
            image,
            action: selection.action,
        })
    }

    pub fn record_image(&self, image: &CapturedImage) -> Result<bool, ScreenshotServiceError> {
        let captured_at_ms = now_ms();
        let record = self.clipboard.record_image(
            image.width,
            image.height,
            image.rgba.clone(),
            ClipboardCaptureMetadata {
                captured_at_ms,
                source_application: Some("OpenDeskTools".to_owned()),
                source_process: Some("open-desk-tools.exe".to_owned()),
            },
        )?;
        Ok(record.retained)
    }

    pub fn copy_image<F>(
        &self,
        image: &CapturedImage,
        mut suppress: F,
    ) -> Result<(), ScreenshotServiceError>
    where
        F: FnMut(u32),
    {
        let content = ClipboardWriteContent::Image {
            width: image.width,
            height: image.height,
            rgba: image.rgba.clone(),
        };
        self.writer.replace_current(0, &content, &mut suppress)?;
        Ok(())
    }

    pub fn session_active(&self) -> bool {
        self.session_active.load(Ordering::Acquire)
    }

    fn begin_session(&self) -> Result<SessionGuard<'_>, ScreenshotError> {
        self.session_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| ScreenshotError::SessionAlreadyActive)?;
        Ok(SessionGuard {
            active: &self.session_active,
        })
    }
}

#[derive(Debug)]
struct SessionGuard<'a> {
    active: &'a AtomicBool,
}

impl Drop for SessionGuard<'_> {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Release);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    use crate::infrastructure::storage::StorageService;

    #[test]
    fn session_gate_rejects_overlap_and_reopens_after_drop() {
        let directory = tempdir().unwrap();
        let storage = Arc::new(StorageService::initialize(directory.path()).unwrap());
        let clipboard = Arc::new(ClipboardService::try_initialize(storage).unwrap());
        let service = ScreenshotService::new(clipboard);
        let first = service.begin_session().unwrap();
        assert!(service.session_active());
        assert_eq!(
            service.begin_session().unwrap_err(),
            ScreenshotError::SessionAlreadyActive
        );
        drop(first);
        assert!(!service.session_active());
        assert!(service.begin_session().is_ok());
    }
}
