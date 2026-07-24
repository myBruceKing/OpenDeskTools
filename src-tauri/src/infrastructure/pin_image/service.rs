use std::sync::Arc;

use thiserror::Error;

use super::window_runtime::{
    render_text_card, NativeImageSurfaceRuntime, NativeSurfaceError, PinImageData,
};
use crate::infrastructure::clipboard::{ClipboardError, ClipboardService, ClipboardWriteContent};
use crate::infrastructure::clipboard_listener::ClipboardSequenceSuppressor;
use crate::infrastructure::keyboard_hook::KeyboardHookBroker;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinImageOutcome {
    pub pin_id: u64,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Error)]
pub enum PinImageError {
    #[error("clipboard history does not contain pinnable image or text")]
    ImageUnavailable,
    #[error("failed to read the latest clipboard history item: {0}")]
    Clipboard(#[from] ClipboardError),
    #[error("native image surface is unavailable: {0}")]
    NativeSurface(#[from] NativeSurfaceError),
}

#[derive(Debug)]
pub struct PinImageService {
    clipboard: Arc<ClipboardService>,
    runtime: Option<NativeImageSurfaceRuntime>,
    startup_error: Option<String>,
}

impl PinImageService {
    pub fn new(
        clipboard: Arc<ClipboardService>,
        keyboard_hook: Arc<KeyboardHookBroker>,
        suppressor: ClipboardSequenceSuppressor,
    ) -> Self {
        match NativeImageSurfaceRuntime::start(keyboard_hook, suppressor) {
            Ok(runtime) => Self {
                clipboard,
                runtime: Some(runtime),
                startup_error: None,
            },
            Err(error) => Self {
                clipboard,
                runtime: None,
                startup_error: Some(error.to_string()),
            },
        }
    }

    pub fn probe(&self) -> Result<(), PinImageError> {
        self.runtime
            .as_ref()
            .map(|_| ())
            .ok_or_else(|| {
                NativeSurfaceError::Startup(
                    self.startup_error
                        .clone()
                        .unwrap_or_else(|| "runtime did not start".to_owned()),
                )
            })
            .map_err(PinImageError::from)
    }

    pub fn pin_latest(&self) -> Result<PinImageOutcome, PinImageError> {
        let content = match self.clipboard.latest_pinnable_content_for_write() {
            Ok(content) => content,
            Err(ClipboardError::NoLatestItem) => return Err(PinImageError::ImageUnavailable),
            Err(error) => return Err(PinImageError::Clipboard(error)),
        };
        let image = match content {
            ClipboardWriteContent::Image {
                width,
                height,
                rgba,
            } => PinImageData {
                width,
                height,
                rgba,
                source_text: None,
            },
            ClipboardWriteContent::Text(text) => render_text_card(&text)?,
            ClipboardWriteContent::Files { .. } => return Err(PinImageError::ImageUnavailable),
        };
        self.open_image(image)
    }

    pub fn pin_rgba(
        &self,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> Result<PinImageOutcome, PinImageError> {
        self.open_image(PinImageData {
            width,
            height,
            rgba,
            source_text: None,
        })
    }

    fn open_image(&self, image: PinImageData) -> Result<PinImageOutcome, PinImageError> {
        let width = image.width;
        let height = image.height;
        let runtime = self.runtime.as_ref().ok_or_else(|| {
            NativeSurfaceError::Startup(
                self.startup_error
                    .clone()
                    .unwrap_or_else(|| "runtime did not start".to_owned()),
            )
        })?;
        let pin_id = runtime.open(image)?;
        Ok(PinImageOutcome {
            pin_id,
            width,
            height,
        })
    }
}
