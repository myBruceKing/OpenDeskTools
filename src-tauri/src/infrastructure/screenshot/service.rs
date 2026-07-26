use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use thiserror::Error;

use super::annotation::apply_annotations;
use super::backend::gdi::GdiCaptureBackend;
use super::backend::wgc::WgcCaptureBackend;
use super::backend::{
    capture_preferred_snapshot_with, CaptureBackend, CaptureOptions, PreferredCaptureBackends,
};
use super::crop::crop_snapshot;
use super::model::CapturedImage;
use super::monitor::MonitorTopology;
use super::overlay::{self, CaptureAction};
use super::ScreenshotError;
use crate::infrastructure::clipboard::{ClipboardError, ClipboardService, ClipboardWriteContent};
use crate::infrastructure::clipboard_writer::{ClipboardWriter, ClipboardWriterError};
use crate::infrastructure::debug_qa;

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
    capture_backends: Mutex<PreferredCaptureBackends>,
    session_active: AtomicBool,
    generation: AtomicU64,
}

impl ScreenshotService {
    pub fn new(clipboard: Arc<ClipboardService>) -> Self {
        Self {
            clipboard,
            writer: ClipboardWriter::default(),
            capture_backends: Mutex::new(PreferredCaptureBackends::new()),
            session_active: AtomicBool::new(false),
            generation: AtomicU64::new(0),
        }
    }

    pub fn probe(&self) -> Result<(), ScreenshotError> {
        let topology = MonitorTopology::query()?;
        let wgc_capability = WgcCaptureBackend::new().probe(&topology);
        let dxgi_capability = super::backend::dxgi::DxgiCaptureBackend::new().probe(&topology);
        let gdi_capability = GdiCaptureBackend::new().probe(&topology);
        if !wgc_capability.available && !dxgi_capability.available && !gdi_capability.available {
            return Err(ScreenshotError::BackendUnavailable(format!(
                "WGC unavailable: {}; DXGI unavailable: {}; GDI unavailable: {}",
                wgc_capability.detail, dxgi_capability.detail, gdi_capability.detail
            )));
        }
        if wgc_capability.available {
            let mut backends = self.capture_backends.lock().map_err(|_| {
                ScreenshotError::BackendUnavailable(
                    "screenshot backend state is unavailable".to_owned(),
                )
            })?;
            if let Err(error) = backends.prepare(&topology) {
                debug_qa::trace!(format!(
                    "screenshot backend warmup result=fallback error={error}"
                ));
            } else {
                debug_qa::trace!("screenshot backend warmup result=success backend=wgc");
            }
        }
        overlay::probe()
    }

    pub fn capture_selection(&self) -> Result<ScreenshotCaptureOutcome, ScreenshotServiceError> {
        let started = Instant::now();
        debug_qa::trace!("screenshot session stage=requested");
        let _session = match self.begin_session() {
            Ok(session) => session,
            Err(error) => {
                debug_qa::trace!(format!("screenshot session result=rejected reason={error}"));
                return Err(error.into());
            }
        };
        debug_qa::trace!("screenshot session stage=started");
        let result = self.capture_selection_inner();
        match &result {
            Ok(ScreenshotCaptureOutcome::Cancelled) => debug_qa::trace!(format!(
                "screenshot session result=cancelled elapsed_ms={}",
                started.elapsed().as_millis()
            )),
            Ok(ScreenshotCaptureOutcome::Selected { action, image }) => debug_qa::trace!(format!(
                "screenshot session result=selected action={action:?} image={}x{} elapsed_ms={}",
                image.width,
                image.height,
                started.elapsed().as_millis()
            )),
            Err(error) => debug_qa::trace!(format!(
                "screenshot session result=error error={error} elapsed_ms={}",
                started.elapsed().as_millis()
            )),
        }
        result
    }

    fn capture_selection_inner(&self) -> Result<ScreenshotCaptureOutcome, ScreenshotServiceError> {
        let topology = MonitorTopology::query()?;
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let snapshot = {
            let mut backends = self.capture_backends.lock().map_err(|_| {
                ScreenshotError::BackendUnavailable(
                    "screenshot backend state is unavailable".to_owned(),
                )
            })?;
            Arc::new(capture_preferred_snapshot_with(
                &mut backends,
                &topology,
                generation,
                &CaptureOptions::default(),
            )?)
        };
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
        let record = self.clipboard.record_application_image(
            image.width,
            image.height,
            image.rgba.clone(),
        )?;
        Ok(record.retained)
    }

    pub fn copy_image<F>(
        &self,
        owner_window: usize,
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
        self.writer
            .replace_current(owner_window, &content, &mut suppress)?;
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
