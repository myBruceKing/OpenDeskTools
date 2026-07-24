//! F4 QR conversion using the newest **internal** clipboard history item.
//!
//! `ClipboardQRService` is a behavior reference only: OpenDeskTools keeps the
//! source selection, persistence and Windows clipboard write in its own Rust
//! services. The source is never read back from the mutable system clipboard.

use std::sync::Arc;

use thiserror::Error;

use super::clipboard::{
    ClipboardCaptureMetadata, ClipboardError, ClipboardService, ClipboardWriteContent,
};
use super::clipboard_writer::ClipboardWriter;

const QR_RENDER_SIZE: u32 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QrConversionKind {
    TextToImage,
    ImageToText,
}

impl QrConversionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TextToImage => "text_to_image",
            Self::ImageToText => "image_to_text",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrConversionResult {
    pub kind: QrConversionKind,
    pub system_clipboard_synced: bool,
}

#[derive(Debug, Error)]
pub enum QrError {
    #[error("the latest internal clipboard item is unavailable")]
    NoLatestItem,
    #[error("the latest internal clipboard item is not text or an image")]
    UnsupportedContent,
    #[error("the latest internal text is empty")]
    EmptyText,
    #[error("the text is too large for a QR code")]
    TextTooLarge,
    #[error("the latest internal image contains no readable QR code")]
    UnreadableImage,
    #[error("QR code payload is not UTF-8 text")]
    NonTextPayload,
    #[error("failed to persist the QR conversion result")]
    Clipboard(#[from] ClipboardError),
}

pub struct QrService {
    clipboard: Arc<ClipboardService>,
    writer: ClipboardWriter,
}

impl std::fmt::Debug for QrService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("QrService").finish_non_exhaustive()
    }
}

impl QrService {
    pub fn new(clipboard: Arc<ClipboardService>) -> Self {
        Self {
            clipboard,
            writer: ClipboardWriter::default(),
        }
    }

    pub fn convert_latest<F>(&self, suppress: F) -> Result<QrConversionResult, QrError>
    where
        F: FnMut(u32),
    {
        let input = self
            .clipboard
            .latest_content_for_write()
            .map_err(|error| match error {
                ClipboardError::NoLatestItem => QrError::NoLatestItem,
                other => QrError::Clipboard(other),
            })?;
        match input {
            ClipboardWriteContent::Text(text) => self.encode_text(text, suppress),
            ClipboardWriteContent::Image {
                width,
                height,
                rgba,
            } => self.decode_image(width, height, rgba, suppress),
            ClipboardWriteContent::Files { .. } => Err(QrError::UnsupportedContent),
        }
    }

    fn encode_text<F>(&self, text: String, suppress: F) -> Result<QrConversionResult, QrError>
    where
        F: FnMut(u32),
    {
        let text = text.trim();
        if text.is_empty() {
            return Err(QrError::EmptyText);
        }
        let (width, height, rgba) = render_qr_rgba(text)?;
        self.clipboard
            .record_image(width, height, rgba.clone(), generated_metadata())?;
        let system_clipboard_synced = self.sync_system_clipboard(
            &ClipboardWriteContent::Image {
                width,
                height,
                rgba,
            },
            suppress,
        );
        Ok(QrConversionResult {
            kind: QrConversionKind::TextToImage,
            system_clipboard_synced,
        })
    }

    pub fn decode_image<F>(
        &self,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        suppress: F,
    ) -> Result<QrConversionResult, QrError>
    where
        F: FnMut(u32),
    {
        let text = decode_qr_text(width, height, &rgba)?;
        self.clipboard
            .record_text(text.clone(), generated_metadata())?;
        let system_clipboard_synced =
            self.sync_system_clipboard(&ClipboardWriteContent::Text(text), suppress);
        Ok(QrConversionResult {
            kind: QrConversionKind::ImageToText,
            system_clipboard_synced,
        })
    }

    #[cfg(not(test))]
    fn sync_system_clipboard<F>(&self, content: &ClipboardWriteContent, suppress: F) -> bool
    where
        F: FnMut(u32),
    {
        self.writer.replace_current(0, content, suppress).is_ok()
    }

    #[cfg(test)]
    fn sync_system_clipboard<F>(&self, _content: &ClipboardWriteContent, _suppress: F) -> bool
    where
        F: FnMut(u32),
    {
        let _ = &self.writer;
        false
    }
}

fn generated_metadata() -> ClipboardCaptureMetadata {
    let captured_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        });
    ClipboardCaptureMetadata {
        captured_at_ms,
        source_application: Some("OpenDeskTools".to_owned()),
        source_process: Some("open-desk-tools.exe".to_owned()),
    }
}

fn render_qr_rgba(text: &str) -> Result<(u32, u32, Vec<u8>), QrError> {
    use qrcodegen::{QrCode, QrCodeEcc};

    let qr = QrCode::encode_text(text, QrCodeEcc::Medium).map_err(|_| QrError::TextTooLarge)?;
    let modules = u32::try_from(qr.size()).map_err(|_| QrError::TextTooLarge)?;
    let quiet_zone = 4_u32;
    let full_modules = modules
        .checked_add(quiet_zone * 2)
        .ok_or(QrError::TextTooLarge)?;
    let scale = (QR_RENDER_SIZE / full_modules).max(1);
    let rendered_size = full_modules
        .checked_mul(scale)
        .ok_or(QrError::TextTooLarge)?;
    let padding = QR_RENDER_SIZE.saturating_sub(rendered_size) / 2;
    let bytes = usize::try_from(u64::from(QR_RENDER_SIZE) * u64::from(QR_RENDER_SIZE) * 4)
        .map_err(|_| QrError::TextTooLarge)?;
    let mut rgba = vec![255_u8; bytes];
    for y in 0..modules {
        for x in 0..modules {
            if !qr.get_module(x as i32, y as i32) {
                continue;
            }
            let left = padding + (x + quiet_zone) * scale;
            let top = padding + (y + quiet_zone) * scale;
            for pixel_y in top..top + scale {
                for pixel_x in left..left + scale {
                    let offset = usize::try_from(
                        (u64::from(pixel_y) * u64::from(QR_RENDER_SIZE) + u64::from(pixel_x)) * 4,
                    )
                    .map_err(|_| QrError::TextTooLarge)?;
                    rgba[offset..offset + 4].copy_from_slice(&[0, 0, 0, 255]);
                }
            }
        }
    }
    Ok((QR_RENDER_SIZE, QR_RENDER_SIZE, rgba))
}

fn decode_qr_text(width: u32, height: u32, rgba: &[u8]) -> Result<String, QrError> {
    let expected = usize::try_from(u64::from(width) * u64::from(height) * 4)
        .map_err(|_| QrError::UnreadableImage)?;
    if width == 0 || height == 0 || rgba.len() != expected {
        return Err(QrError::UnreadableImage);
    }
    for contrast in [100_i32, 150, 50] {
        let grayscale = rgba
            .chunks_exact(4)
            .map(|pixel| {
                let luminance = (i32::from(pixel[0]) * 299
                    + i32::from(pixel[1]) * 587
                    + i32::from(pixel[2]) * 114)
                    / 1_000;
                ((luminance - 128) * contrast / 100 + 128).clamp(0, 255) as u8
            })
            .collect::<Vec<_>>();
        let mut decoder = quircs::Quirc::default();
        for code in decoder.identify(width as usize, height as usize, &grayscale) {
            let code = code.map_err(|_| QrError::UnreadableImage)?;
            let decoded = code.decode().map_err(|_| QrError::UnreadableImage)?;
            let text = String::from_utf8(decoded.payload).map_err(|_| QrError::NonTextPayload)?;
            let text = text.trim();
            if !text.is_empty() {
                return Ok(text.to_owned());
            }
        }
    }
    Err(QrError::UnreadableImage)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::tempdir;

    use super::*;
    use crate::infrastructure::clipboard::{ClipboardContentKind, ClipboardHistoryQuery};
    use crate::infrastructure::storage::StorageService;

    #[test]
    fn generated_qr_round_trips_utf8_text_with_a_quiet_zone() {
        let (width, height, rgba) = render_qr_rgba("https://example.com/中文?ok=1").unwrap();
        assert_eq!((width, height), (QR_RENDER_SIZE, QR_RENDER_SIZE));
        assert_eq!(
            decode_qr_text(width, height, &rgba).unwrap(),
            "https://example.com/中文?ok=1"
        );
    }

    #[test]
    fn invalid_and_blank_images_are_not_misreported_as_qr_text() {
        assert!(matches!(
            decode_qr_text(1, 1, &[255, 255, 255, 255]),
            Err(QrError::UnreadableImage)
        ));
        assert!(matches!(
            decode_qr_text(1, 1, &[255]),
            Err(QrError::UnreadableImage)
        ));
    }

    #[test]
    fn converts_the_latest_internal_record_without_reading_system_clipboard() {
        let temp = tempdir().unwrap();
        let storage = Arc::new(StorageService::initialize(temp.path().join("data")).unwrap());
        let clipboard = Arc::new(ClipboardService::initialize(storage));
        clipboard
            .record_text(
                "https://example.com/internal-only".to_owned(),
                ClipboardCaptureMetadata {
                    captured_at_ms: 10,
                    source_application: None,
                    source_process: None,
                },
            )
            .unwrap();
        let service = QrService::new(Arc::clone(&clipboard));

        let encoded = service.convert_latest(|_| {}).unwrap();
        assert_eq!(encoded.kind, QrConversionKind::TextToImage);
        assert!(!encoded.system_clipboard_synced);
        let latest = clipboard
            .history(ClipboardHistoryQuery {
                favorites_only: false,
                search: None,
                limit: 1,
            })
            .unwrap()
            .items
            .pop()
            .unwrap();
        assert_eq!(latest.kind, ClipboardContentKind::Image);

        let decoded = service.convert_latest(|_| {}).unwrap();
        assert_eq!(decoded.kind, QrConversionKind::ImageToText);
        let latest = clipboard
            .history(ClipboardHistoryQuery {
                favorites_only: false,
                search: None,
                limit: 1,
            })
            .unwrap()
            .items
            .pop()
            .unwrap();
        assert_eq!(
            latest.text_content.as_deref(),
            Some("https://example.com/internal-only")
        );
    }
}
