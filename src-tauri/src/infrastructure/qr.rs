//! F4 QR conversion using the newest **internal** clipboard history item.
//!
//! `ClipboardQRService` is a behavior reference only: OpenDeskTools keeps the
//! source selection, persistence and Windows clipboard write in its own Rust
//! services. The source is never read back from the mutable system clipboard.

use std::sync::Arc;

use ::image::{imageops, GrayImage};
use thiserror::Error;

use super::clipboard::{ClipboardError, ClipboardService, ClipboardWriteContent};
use super::clipboard_writer::ClipboardWriter;

const QR_RENDER_SIZE: u32 = 300;
const QR_MIN_DECODE_SIDE: u32 = 480;
const QR_MAX_UPSCALE: u32 = 4;
const QR_MAX_PREPROCESS_PIXELS: u64 = 4_000_000;

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

    pub fn convert_latest<F>(
        &self,
        owner_window: usize,
        suppress: F,
    ) -> Result<QrConversionResult, QrError>
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
            ClipboardWriteContent::Text(text) => self.encode_text(owner_window, text, suppress),
            ClipboardWriteContent::Image {
                width,
                height,
                rgba,
            } => self.decode_image(owner_window, width, height, rgba, suppress),
            ClipboardWriteContent::Files { .. } => Err(QrError::UnsupportedContent),
        }
    }

    fn encode_text<F>(
        &self,
        owner_window: usize,
        text: String,
        suppress: F,
    ) -> Result<QrConversionResult, QrError>
    where
        F: FnMut(u32),
    {
        let text = text.trim();
        if text.is_empty() {
            return Err(QrError::EmptyText);
        }
        let (width, height, rgba) = render_qr_rgba(text)?;
        self.clipboard
            .record_application_image(width, height, rgba.clone())?;
        let system_clipboard_synced = self.sync_system_clipboard(
            owner_window,
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
        owner_window: usize,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        suppress: F,
    ) -> Result<QrConversionResult, QrError>
    where
        F: FnMut(u32),
    {
        let text = decode_qr_text(width, height, &rgba)?;
        self.clipboard.record_application_text(text.clone())?;
        let system_clipboard_synced =
            self.sync_system_clipboard(owner_window, &ClipboardWriteContent::Text(text), suppress);
        Ok(QrConversionResult {
            kind: QrConversionKind::ImageToText,
            system_clipboard_synced,
        })
    }

    #[cfg(not(test))]
    fn sync_system_clipboard<F>(
        &self,
        owner_window: usize,
        content: &ClipboardWriteContent,
        suppress: F,
    ) -> bool
    where
        F: FnMut(u32),
    {
        self.writer
            .replace_current(owner_window, content, suppress)
            .is_ok()
    }

    #[cfg(test)]
    fn sync_system_clipboard<F>(
        &self,
        _owner_window: usize,
        _content: &ClipboardWriteContent,
        _suppress: F,
    ) -> bool
    where
        F: FnMut(u32),
    {
        let _ = &self.writer;
        false
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
    let expected = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .and_then(|bytes| usize::try_from(bytes).ok())
        .ok_or(QrError::UnreadableImage)?;
    if width == 0 || height == 0 || rgba.len() != expected {
        return Err(QrError::UnreadableImage);
    }

    let grayscale = rgba_to_grayscale(rgba);
    let mut saw_non_text_payload = false;
    if let Some(text) = decode_gray_qr(width, height, &grayscale, &mut saw_non_text_payload) {
        return Ok(text);
    }

    let normalized = normalize_grayscale(&grayscale);
    if normalized != grayscale {
        if let Some(text) = decode_gray_qr(width, height, &normalized, &mut saw_non_text_payload) {
            return Ok(text);
        }
    }

    let threshold = otsu_threshold(&normalized);
    for offset in [0_i8, -12, 12] {
        let binary = threshold_grayscale(&normalized, threshold.saturating_add_signed(offset));
        if let Some(text) = decode_gray_qr(width, height, &binary, &mut saw_non_text_payload) {
            return Ok(text);
        }
    }

    let sharpened = sharpen_grayscale(width, height, &normalized);
    if let Some(text) = decode_gray_qr(width, height, &sharpened, &mut saw_non_text_payload) {
        return Ok(text);
    }
    let sharpened_binary = threshold_grayscale(&sharpened, otsu_threshold(&sharpened));
    if let Some(text) = decode_gray_qr(width, height, &sharpened_binary, &mut saw_non_text_payload)
    {
        return Ok(text);
    }

    if let Some((scaled_width, scaled_height, nearest, smooth)) =
        upscale_low_resolution_grayscale(width, height, &normalized)
    {
        for scaled in [&nearest, &smooth] {
            if let Some(text) = decode_gray_qr(
                scaled_width,
                scaled_height,
                scaled,
                &mut saw_non_text_payload,
            ) {
                return Ok(text);
            }
            let binary = threshold_grayscale(scaled, otsu_threshold(scaled));
            if let Some(text) = decode_gray_qr(
                scaled_width,
                scaled_height,
                &binary,
                &mut saw_non_text_payload,
            ) {
                return Ok(text);
            }
        }

        let scaled_sharp = sharpen_grayscale(scaled_width, scaled_height, &smooth);
        if let Some(text) = decode_gray_qr(
            scaled_width,
            scaled_height,
            &scaled_sharp,
            &mut saw_non_text_payload,
        ) {
            return Ok(text);
        }
        let adaptive = adaptive_threshold_grayscale(scaled_width, scaled_height, &scaled_sharp);
        if let Some(text) = decode_gray_qr(
            scaled_width,
            scaled_height,
            &adaptive,
            &mut saw_non_text_payload,
        ) {
            return Ok(text);
        }
    }

    if saw_non_text_payload {
        Err(QrError::NonTextPayload)
    } else {
        Err(QrError::UnreadableImage)
    }
}

fn rgba_to_grayscale(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .map(|pixel| {
            let luminance =
                (u32::from(pixel[0]) * 299 + u32::from(pixel[1]) * 587 + u32::from(pixel[2]) * 114)
                    / 1_000;
            let alpha = u32::from(pixel[3]);
            ((luminance * alpha + 255 * (255 - alpha)) / 255) as u8
        })
        .collect()
}

fn decode_gray_qr(
    width: u32,
    height: u32,
    grayscale: &[u8],
    saw_non_text_payload: &mut bool,
) -> Option<String> {
    let mut decoder = quircs::Quirc::default();
    for candidate in decoder.identify(width as usize, height as usize, grayscale) {
        let Ok(code) = candidate else {
            continue;
        };
        let Ok(decoded) = code.decode() else {
            continue;
        };
        let Ok(text) = String::from_utf8(decoded.payload) else {
            *saw_non_text_payload = true;
            continue;
        };
        let text = text.trim();
        if !text.is_empty() {
            return Some(text.to_owned());
        }
    }
    None
}

fn normalize_grayscale(grayscale: &[u8]) -> Vec<u8> {
    if grayscale.is_empty() {
        return Vec::new();
    }
    let mut histogram = [0_usize; 256];
    for value in grayscale {
        histogram[usize::from(*value)] += 1;
    }
    let tail = (grayscale.len() / 100).max(1);
    let mut cumulative = 0_usize;
    let mut low = 0_u8;
    for (value, count) in histogram.iter().enumerate() {
        cumulative += count;
        if cumulative >= tail {
            low = value as u8;
            break;
        }
    }
    cumulative = 0;
    let mut high = 255_u8;
    for (value, count) in histogram.iter().enumerate().rev() {
        cumulative += count;
        if cumulative >= tail {
            high = value as u8;
            break;
        }
    }
    if high <= low.saturating_add(8) {
        return grayscale.to_vec();
    }
    let range = u32::from(high - low);
    grayscale
        .iter()
        .map(|value| {
            if *value <= low {
                0
            } else if *value >= high {
                255
            } else {
                ((u32::from(*value - low) * 255) / range) as u8
            }
        })
        .collect()
}

fn otsu_threshold(grayscale: &[u8]) -> u8 {
    let mut histogram = [0_u64; 256];
    for value in grayscale {
        histogram[usize::from(*value)] += 1;
    }
    let total = grayscale.len() as u64;
    let sum = histogram
        .iter()
        .enumerate()
        .map(|(value, count)| value as u64 * count)
        .sum::<u64>();
    let mut background_count = 0_u64;
    let mut background_sum = 0_u64;
    let mut best_threshold = 127_u8;
    let mut best_variance = 0_f64;
    for (threshold, count) in histogram.iter().enumerate() {
        background_count += count;
        if background_count == 0 {
            continue;
        }
        let foreground_count = total.saturating_sub(background_count);
        if foreground_count == 0 {
            break;
        }
        background_sum += threshold as u64 * count;
        let background_mean = background_sum as f64 / background_count as f64;
        let foreground_mean = (sum - background_sum) as f64 / foreground_count as f64;
        let difference = background_mean - foreground_mean;
        let variance = background_count as f64 * foreground_count as f64 * difference * difference;
        if variance > best_variance {
            best_variance = variance;
            best_threshold = threshold as u8;
        }
    }
    best_threshold
}

fn threshold_grayscale(grayscale: &[u8], threshold: u8) -> Vec<u8> {
    grayscale
        .iter()
        .map(|value| if *value <= threshold { 0 } else { 255 })
        .collect()
}

fn sharpen_grayscale(width: u32, height: u32, grayscale: &[u8]) -> Vec<u8> {
    if width < 3 || height < 3 {
        return grayscale.to_vec();
    }
    let width = width as usize;
    let height = height as usize;
    let mut sharpened = grayscale.to_vec();
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let offset = y * width + x;
            let value = i32::from(grayscale[offset]) * 5
                - i32::from(grayscale[offset - 1])
                - i32::from(grayscale[offset + 1])
                - i32::from(grayscale[offset - width])
                - i32::from(grayscale[offset + width]);
            sharpened[offset] = value.clamp(0, 255) as u8;
        }
    }
    sharpened
}

fn upscale_low_resolution_grayscale(
    width: u32,
    height: u32,
    grayscale: &[u8],
) -> Option<(u32, u32, Vec<u8>, Vec<u8>)> {
    let shortest = width.min(height);
    if shortest == 0 || shortest >= QR_MIN_DECODE_SIDE {
        return None;
    }
    let scale = QR_MIN_DECODE_SIDE
        .saturating_add(shortest - 1)
        .checked_div(shortest)?
        .clamp(2, QR_MAX_UPSCALE);
    let scaled_width = width.checked_mul(scale)?;
    let scaled_height = height.checked_mul(scale)?;
    if u64::from(scaled_width) * u64::from(scaled_height) > QR_MAX_PREPROCESS_PIXELS {
        return None;
    }
    let image = GrayImage::from_raw(width, height, grayscale.to_vec())?;
    let nearest = imageops::resize(
        &image,
        scaled_width,
        scaled_height,
        imageops::FilterType::Nearest,
    )
    .into_raw();
    let smooth = imageops::resize(
        &image,
        scaled_width,
        scaled_height,
        imageops::FilterType::CatmullRom,
    )
    .into_raw();
    Some((scaled_width, scaled_height, nearest, smooth))
}

fn adaptive_threshold_grayscale(width: u32, height: u32, grayscale: &[u8]) -> Vec<u8> {
    let width = width as usize;
    let height = height as usize;
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let stride = width + 1;
    let mut integral = vec![0_u64; stride * (height + 1)];
    for y in 0..height {
        let mut row_sum = 0_u64;
        for x in 0..width {
            row_sum += u64::from(grayscale[y * width + x]);
            integral[(y + 1) * stride + x + 1] = integral[y * stride + x + 1] + row_sum;
        }
    }
    let radius = (width.min(height) / 40).clamp(4, 24);
    let mut output = vec![255_u8; grayscale.len()];
    for y in 0..height {
        let top = y.saturating_sub(radius);
        let bottom = (y + radius + 1).min(height);
        for x in 0..width {
            let left = x.saturating_sub(radius);
            let right = (x + radius + 1).min(width);
            let sum = integral[bottom * stride + right] + integral[top * stride + left]
                - integral[top * stride + right]
                - integral[bottom * stride + left];
            let count = (bottom - top) * (right - left);
            let mean = (sum / count as u64) as u8;
            output[y * width + x] = if grayscale[y * width + x].saturating_add(7) < mean {
                0
            } else {
                255
            };
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::tempdir;

    use super::*;
    use crate::infrastructure::clipboard::{
        ClipboardCaptureMetadata, ClipboardContentKind, ClipboardHistoryQuery,
    };
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
    fn low_resolution_blurred_qr_is_recovered_by_preprocessing() {
        let payload = "https://example.com/pay?id=202608310001&source=OpenDeskTools";
        let (width, height, rgba) = render_qr_rgba(payload).unwrap();
        let image = ::image::RgbaImage::from_raw(width, height, rgba).unwrap();
        let reduced = imageops::resize(&image, 116, 116, imageops::FilterType::Triangle);
        let blurred = imageops::blur(&reduced, 0.65);
        let mut canvas =
            ::image::RgbaImage::from_pixel(170, 145, ::image::Rgba([232, 230, 224, 255]));
        imageops::overlay(&mut canvas, &blurred, 24, 14);

        assert_eq!(
            decode_qr_text(canvas.width(), canvas.height(), canvas.as_raw()).unwrap(),
            payload
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

        let encoded = service.convert_latest(1, |_| {}).unwrap();
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

        let decoded = service.convert_latest(1, |_| {}).unwrap();
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
