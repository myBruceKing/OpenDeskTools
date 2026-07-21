use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Cursor, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};
use thiserror::Error;

use super::storage::{StorageError, StorageService};

pub const MAX_IMAGE_WIDTH: u32 = 16_384;
pub const MAX_IMAGE_HEIGHT: u32 = 16_384;
pub const MAX_IMAGE_PIXELS: u64 = 32 * 1024 * 1024;
pub const MAX_RGBA_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_PNG_BYTES: u64 = 64 * 1024 * 1024;
const IMAGE_DIRECTORY: &str = "files/clipboard/images";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum ImageError {
    #[error("image dimensions or data are invalid")]
    InvalidImage,
    #[error("encoded image exceeds the supported size")]
    TooLarge,
    #[error("image reference is invalid")]
    InvalidReference,
    #[error("managed image is unavailable")]
    Missing,
    #[error("managed image is corrupt")]
    Corrupt,
    #[error("image file operation failed")]
    Io(#[from] std::io::Error),
    #[error("image encoding failed")]
    Encode(#[from] png::EncodingError),
    #[error("managed storage operation failed")]
    Storage(#[from] StorageError),
}

#[derive(Debug, Clone)]
pub struct StoredImage {
    pub hash: String,
    pub reference: String,
    pub byte_size: u64,
    pub newly_created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReconcileResult {
    pub missing_references: Vec<String>,
    pub removed_files: usize,
    pub cleanup_failures: usize,
}

#[derive(Debug)]
pub struct ImageService {
    root: PathBuf,
}

impl ImageService {
    pub fn initialize(storage: &StorageService) -> Result<Self, ImageError> {
        let root = storage.resolve_relative_path(IMAGE_DIRECTORY)?;
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn store_rgba(
        &self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<StoredImage, ImageError> {
        validate_rgba(width, height, rgba)?;
        let mut png_bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_bytes, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header()?;
            writer.write_image_data(rgba)?;
        }
        if u64::try_from(png_bytes.len()).map_err(|_| ImageError::TooLarge)? > MAX_PNG_BYTES {
            return Err(ImageError::TooLarge);
        }
        self.store_png_bytes(&png_bytes)
    }

    fn store_png_bytes(&self, bytes: &[u8]) -> Result<StoredImage, ImageError> {
        let hash = format!("{:x}", Sha256::digest(bytes));
        let file_name = format!("{hash}.png");
        let destination = self.root.join(&file_name);
        let reference = format!("{IMAGE_DIRECTORY}/{file_name}");
        if destination.exists() {
            match self.validate_managed_file(&destination, &hash) {
                Ok(_) => {}
                Err(_) => fs::remove_file(&destination)?,
            }
        }
        if destination.exists() {
            return Ok(StoredImage {
                hash,
                reference,
                byte_size: bytes.len() as u64,
                newly_created: false,
            });
        }
        let nonce = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary = self
            .root
            .join(format!(".{hash}.{}.{}.tmp", std::process::id(), nonce));
        let write_result = (|| -> Result<bool, ImageError> {
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            let mut writer = BufWriter::new(file);
            writer.write_all(bytes)?;
            writer.flush()?;
            writer.get_ref().sync_all()?;
            drop(writer);
            match fs::rename(&temporary, &destination) {
                Ok(()) => Ok(true),
                Err(_error) if destination.exists() => {
                    self.validate_managed_file(&destination, &hash)?;
                    let _ = fs::remove_file(&temporary);
                    Ok(false)
                }
                Err(error) => Err(error.into()),
            }
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        let newly_created = write_result?;
        Ok(StoredImage {
            hash,
            reference,
            byte_size: bytes.len() as u64,
            newly_created,
        })
    }

    pub fn read(&self, reference: &str) -> Result<Vec<u8>, ImageError> {
        let path = self.path_for_reference(reference)?;
        let expected_hash = reference
            .rsplit('/')
            .next()
            .and_then(|name| name.strip_suffix(".png"))
            .ok_or(ImageError::InvalidReference)?;
        self.validate_managed_file(&path, expected_hash)
    }

    pub fn decode_rgba(&self, reference: &str) -> Result<DecodedImage, ImageError> {
        let bytes = self.read(reference)?;
        let decoder = png::Decoder::new(Cursor::new(bytes));
        let mut reader = decoder.read_info().map_err(|_| ImageError::Corrupt)?;
        let info = reader.info();
        if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
            return Err(ImageError::Corrupt);
        }
        let width = info.width;
        let height = info.height;
        let output_size = reader.output_buffer_size();
        if output_size > MAX_RGBA_BYTES {
            return Err(ImageError::TooLarge);
        }
        let mut rgba = vec![0; output_size];
        let frame = reader
            .next_frame(&mut rgba)
            .map_err(|_| ImageError::Corrupt)?;
        rgba.truncate(frame.buffer_size());
        validate_rgba(width, height, &rgba)?;
        Ok(DecodedImage {
            width,
            height,
            rgba,
        })
    }

    pub fn remove(&self, reference: &str) -> Result<(), ImageError> {
        let path = self.path_for_reference(reference)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn reconcile(&self, references: &[String]) -> ReconcileResult {
        let mut valid_expected = std::collections::HashSet::new();
        let mut result = ReconcileResult::default();
        for reference in references {
            match self.read(reference) {
                Ok(_) => {
                    valid_expected.insert(reference.clone());
                }
                Err(_) => result.missing_references.push(reference.clone()),
            }
        }
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(_) => {
                result.cleanup_failures += 1;
                return result;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    result.cleanup_failures += 1;
                    continue;
                }
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let reference = format!("{IMAGE_DIRECTORY}/{name}");
            if !valid_expected.contains(&reference) {
                match fs::remove_file(entry.path()) {
                    Ok(()) => result.removed_files += 1,
                    Err(_) => result.cleanup_failures += 1,
                }
            }
        }
        result
    }

    fn path_for_reference(&self, reference: &str) -> Result<PathBuf, ImageError> {
        let prefix = format!("{IMAGE_DIRECTORY}/");
        let name = reference
            .strip_prefix(&prefix)
            .ok_or(ImageError::InvalidReference)?;
        if !canonical_hash_file_name(name) {
            return Err(ImageError::InvalidReference);
        }
        Ok(self.root.join(name))
    }

    fn validate_managed_file(
        &self,
        path: &PathBuf,
        expected_hash: &str,
    ) -> Result<Vec<u8>, ImageError> {
        let metadata = fs::symlink_metadata(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ImageError::Missing
            } else {
                ImageError::Io(error)
            }
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || is_reparse_point(&metadata) {
            return Err(ImageError::InvalidReference);
        }
        if metadata.len() == 0 {
            return Err(ImageError::Corrupt);
        }
        if metadata.len() > MAX_PNG_BYTES {
            return Err(ImageError::TooLarge);
        }
        let file = File::open(path)?;
        let opened_metadata = file.metadata()?;
        if !opened_metadata.is_file() || is_reparse_point(&opened_metadata) {
            return Err(ImageError::InvalidReference);
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(MAX_PNG_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_PNG_BYTES {
            return Err(ImageError::TooLarge);
        }
        if format!("{:x}", Sha256::digest(&bytes)) != expected_hash {
            return Err(ImageError::Corrupt);
        }
        validate_png_structure(&bytes)?;
        Ok(bytes)
    }
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn validate_png_structure(bytes: &[u8]) -> Result<(), ImageError> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(ImageError::Corrupt);
    }
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder.read_info().map_err(|_| ImageError::Corrupt)?;
    let info = reader.info();
    let pixels = u64::from(info.width)
        .checked_mul(u64::from(info.height))
        .ok_or(ImageError::Corrupt)?;
    if info.width == 0
        || info.height == 0
        || info.width > MAX_IMAGE_WIDTH
        || info.height > MAX_IMAGE_HEIGHT
        || pixels > MAX_IMAGE_PIXELS
    {
        return Err(ImageError::TooLarge);
    }
    let output_size = reader.output_buffer_size();
    if output_size > MAX_RGBA_BYTES {
        return Err(ImageError::TooLarge);
    }
    let mut decoded = vec![0; output_size];
    reader
        .next_frame(&mut decoded)
        .map_err(|_| ImageError::Corrupt)?;
    Ok(())
}

fn canonical_hash_file_name(name: &str) -> bool {
    let Some(hash) = name.strip_suffix(".png") else {
        return false;
    };
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<(), ImageError> {
    if width == 0 || height == 0 || width > MAX_IMAGE_WIDTH || height > MAX_IMAGE_HEIGHT {
        return Err(ImageError::InvalidImage);
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(ImageError::InvalidImage)?;
    if pixels > MAX_IMAGE_PIXELS {
        return Err(ImageError::TooLarge);
    }
    let bytes = pixels
        .checked_mul(4)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(ImageError::TooLarge)?;
    if bytes > MAX_RGBA_BYTES || bytes != rgba.len() {
        return Err(ImageError::InvalidImage);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::storage::StorageService;
    use tempfile::tempdir;

    fn service() -> (tempfile::TempDir, ImageService) {
        let temp = tempdir().unwrap();
        let storage = StorageService::initialize(temp.path()).unwrap();
        let service = ImageService::initialize(&storage).unwrap();
        (temp, service)
    }

    #[test]
    fn content_addressed_write_read_and_duplicate_are_stable() {
        let (_temp, service) = service();
        let first = service.store_rgba(1, 1, &[1, 2, 3, 255]).unwrap();
        let second = service.store_rgba(1, 1, &[1, 2, 3, 255]).unwrap();
        assert!(first.newly_created);
        assert!(!second.newly_created);
        assert_eq!(first.reference, second.reference);
        assert!(service
            .read(&first.reference)
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n"));

        let path = service.path_for_reference(&first.reference).unwrap();
        fs::write(&path, b"corrupt existing hash target").unwrap();
        let repaired = service.store_rgba(1, 1, &[1, 2, 3, 255]).unwrap();
        assert!(repaired.newly_created);
        assert!(service.read(&repaired.reference).is_ok());
    }

    #[test]
    fn canonical_reference_blocks_traversal_and_reconcile_handles_temp_orphan_and_missing() {
        let (_temp, service) = service();
        assert!(matches!(
            service.read("../secret"),
            Err(ImageError::InvalidReference)
        ));
        let stored = service.store_rgba(1, 1, &[1, 2, 3, 255]).unwrap();
        let missing = format!("{IMAGE_DIRECTORY}/{}.png", "a".repeat(64));
        fs::write(service.root.join("stale.tmp"), b"x").unwrap();
        let orphan = service.root.join(format!("{}.png", "b".repeat(64)));
        fs::write(&orphan, b"x").unwrap();
        let result = service.reconcile(&[stored.reference.clone(), missing.clone()]);
        assert_eq!(result.missing_references, vec![missing]);
        assert_eq!(result.removed_files, 2);
        assert!(service.read(&stored.reference).is_ok());
    }

    #[test]
    fn read_rejects_corrupt_hash_mismatch_nonfiles_and_reconcile_degrades_cleanup_failures() {
        let (_temp, service) = service();
        let stored = service.store_rgba(1, 1, &[7, 8, 9, 255]).unwrap();
        let path = service.path_for_reference(&stored.reference).unwrap();
        fs::write(&path, b"not a png").unwrap();
        assert!(matches!(
            service.read(&stored.reference),
            Err(ImageError::Corrupt)
        ));
        let result = service.reconcile(std::slice::from_ref(&stored.reference));
        assert_eq!(result.missing_references, vec![stored.reference]);
        assert!(!path.exists());

        let directory_name = format!("{}.png", "c".repeat(64));
        fs::create_dir(service.root.join(directory_name)).unwrap();
        let result = service.reconcile(&[]);
        assert_eq!(result.cleanup_failures, 1);
    }

    #[cfg(windows)]
    #[test]
    fn read_rejects_symlink_or_reparse_reference() {
        use std::os::windows::fs::symlink_file;
        let (_temp, service) = service();
        let stored = service.store_rgba(1, 1, &[3, 4, 5, 255]).unwrap();
        let original = service.path_for_reference(&stored.reference).unwrap();
        let target = service.root.join("real.png");
        fs::rename(&original, &target).unwrap();
        if symlink_file(&target, &original).is_err() {
            return;
        }
        assert!(matches!(
            service.read(&stored.reference),
            Err(ImageError::InvalidReference)
        ));
    }
}
