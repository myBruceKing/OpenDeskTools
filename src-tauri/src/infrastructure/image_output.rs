use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;

static SAVE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageSaveOutcome {
    Cancelled,
    Saved(PathBuf),
}

#[derive(Debug, Error)]
pub enum ImageOutputError {
    #[error("image dimensions or pixel data are invalid")]
    InvalidImage,
    #[error("image encoding failed")]
    Encode(#[from] png::EncodingError),
    #[error("image file operation failed")]
    Io(#[from] std::io::Error),
    #[cfg(not(windows))]
    #[error("the native image save dialog is unavailable on this platform")]
    UnsupportedPlatform,
}

pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, ImageOutputError> {
    let expected = usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(usize::try_from(height).ok()?))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(ImageOutputError::InvalidImage)?;
    if width == 0 || height == 0 || rgba.len() != expected {
        return Err(ImageOutputError::InvalidImage);
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(rgba)?;
    }
    Ok(bytes)
}

#[cfg(windows)]
pub fn save_rgba_with_dialog(
    suggested_name: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<ImageSaveOutcome, ImageOutputError> {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("PNG 图片", &["png"])
        .set_file_name(suggested_name)
        .save_file()
    else {
        return Ok(ImageSaveOutcome::Cancelled);
    };
    save_rgba_to_path(&path, width, height, rgba)?;
    Ok(ImageSaveOutcome::Saved(path))
}

#[cfg(not(windows))]
pub fn save_rgba_with_dialog(
    _suggested_name: &str,
    _width: u32,
    _height: u32,
    _rgba: &[u8],
) -> Result<ImageSaveOutcome, ImageOutputError> {
    Err(ImageOutputError::UnsupportedPlatform)
}

pub fn save_rgba_to_path(
    path: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(), ImageOutputError> {
    let bytes = encode_png(width, height, rgba)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("screenshot.png");
    let nonce = SAVE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(".{file_name}.{}.{}.tmp", std::process::id(), nonce));
    let result = (|| -> Result<(), ImageOutputError> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let mut writer = BufWriter::new(file);
        writer.write_all(&bytes)?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        drop(writer);
        replace_file(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), std::io::Error> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source: Vec<u16> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), std::io::Error> {
    fs::rename(source, destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_encoder_rejects_mismatched_rgba_and_emits_png() {
        assert!(matches!(
            encode_png(2, 1, &[0; 4]),
            Err(ImageOutputError::InvalidImage)
        ));
        assert!(encode_png(1, 1, &[12, 34, 56, 255])
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn file_save_replaces_destination_with_complete_png() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("capture.png");
        fs::write(&path, b"old").unwrap();
        save_rgba_to_path(&path, 1, 1, &[1, 2, 3, 255]).unwrap();
        assert!(fs::read(path).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}
