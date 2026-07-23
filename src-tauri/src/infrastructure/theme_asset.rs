use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder, ImageFormat, ImageReader};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::storage::{StorageError, StorageService};

const THEME_BACKGROUND_DIRECTORY: &str = "files/themes/backgrounds";
const MAX_SOURCE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ENCODED_BYTES: u64 = 64 * 1024 * 1024;
const MAX_WIDTH: u32 = 16_384;
const MAX_HEIGHT: u32 = 16_384;
const MAX_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_RGBA_BYTES: u64 = 128 * 1024 * 1024;
const MAX_FILE_NAME_CHARS: usize = 160;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThemeBackgroundAsset {
    pub id: String,
    pub file_name: String,
    pub byte_size: u64,
    pub width: u32,
    pub height: u32,
}

impl ThemeBackgroundAsset {
    pub fn validate(&self) -> Result<(), ThemeAssetError> {
        if !canonical_asset_id(&self.id)
            || self.file_name.is_empty()
            || self.file_name.chars().count() > MAX_FILE_NAME_CHARS
            || self.file_name.contains(['/', '\\'])
            || self.byte_size == 0
            || self.byte_size > MAX_ENCODED_BYTES
        {
            return Err(ThemeAssetError::InvalidMetadata);
        }
        validate_dimensions(self.width, self.height)
    }
}

#[derive(Debug)]
pub struct ImportedThemeBackground {
    pub asset: ThemeBackgroundAsset,
    pub newly_created: bool,
}

#[derive(Debug, Error)]
pub enum ThemeAssetError {
    #[error("theme background metadata is invalid")]
    InvalidMetadata,
    #[error("theme background source is not a regular file")]
    InvalidSource,
    #[error("theme background format is not supported")]
    UnsupportedFormat,
    #[error("theme background exceeds the supported size")]
    TooLarge,
    #[error("theme background is unavailable")]
    Missing,
    #[error("theme background is corrupt")]
    Corrupt,
    #[error("theme background file operation failed")]
    Io(#[from] std::io::Error),
    #[error("theme background decoding failed")]
    Decode(#[from] image::ImageError),
    #[error("theme background storage failed")]
    Storage(#[from] StorageError),
}

#[derive(Debug)]
pub struct ThemeAssetService {
    root: PathBuf,
}

impl ThemeAssetService {
    pub fn initialize(storage: &StorageService) -> Result<Self, ThemeAssetError> {
        let root = storage.resolve_relative_path(THEME_BACKGROUND_DIRECTORY)?;
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn import(&self, source: &Path) -> Result<ImportedThemeBackground, ThemeAssetError> {
        let metadata = fs::symlink_metadata(source).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ThemeAssetError::Missing
            } else {
                ThemeAssetError::Io(error)
            }
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || is_reparse_point(&metadata) {
            return Err(ThemeAssetError::InvalidSource);
        }
        if metadata.len() == 0 {
            return Err(ThemeAssetError::Corrupt);
        }
        if metadata.len() > MAX_SOURCE_BYTES {
            return Err(ThemeAssetError::TooLarge);
        }

        let file = File::open(source)?;
        let opened_metadata = file.metadata()?;
        if !opened_metadata.is_file() || is_reparse_point(&opened_metadata) {
            return Err(ThemeAssetError::InvalidSource);
        }
        let mut source_bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(MAX_SOURCE_BYTES + 1)
            .read_to_end(&mut source_bytes)?;
        if source_bytes.len() as u64 > MAX_SOURCE_BYTES {
            return Err(ThemeAssetError::TooLarge);
        }

        let format =
            image::guess_format(&source_bytes).map_err(|_| ThemeAssetError::UnsupportedFormat)?;
        if !matches!(
            format,
            ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
        ) {
            return Err(ThemeAssetError::UnsupportedFormat);
        }

        let dimensions_reader = ImageReader::new(Cursor::new(source_bytes.as_slice()))
            .with_guessed_format()
            .map_err(ThemeAssetError::Io)?;
        let (width, height) = dimensions_reader.into_dimensions()?;
        validate_dimensions(width, height)?;

        let decoded = image::load_from_memory_with_format(&source_bytes, format)?;
        if decoded.width() != width || decoded.height() != height {
            return Err(ThemeAssetError::Corrupt);
        }
        let rgba = decoded.into_rgba8();
        let rgba_bytes =
            u64::try_from(rgba.as_raw().len()).map_err(|_| ThemeAssetError::TooLarge)?;
        if rgba_bytes > MAX_RGBA_BYTES {
            return Err(ThemeAssetError::TooLarge);
        }

        let mut png_bytes = Vec::new();
        PngEncoder::new(&mut png_bytes).write_image(
            rgba.as_raw(),
            width,
            height,
            ExtendedColorType::Rgba8,
        )?;
        if png_bytes.is_empty() || png_bytes.len() as u64 > MAX_ENCODED_BYTES {
            return Err(ThemeAssetError::TooLarge);
        }

        let id = format!("{:x}", Sha256::digest(&png_bytes));
        let destination = self.path_for_id(&id)?;
        if destination.exists() && self.validate_file(&destination, &id).is_err() {
            fs::remove_file(&destination)?;
        }
        let newly_created = if destination.exists() {
            false
        } else {
            self.write_atomic(&destination, &id, &png_bytes)?
        };
        let file_name = safe_file_name(source);
        let asset = ThemeBackgroundAsset {
            id,
            file_name,
            byte_size: png_bytes.len() as u64,
            width,
            height,
        };
        asset.validate()?;
        Ok(ImportedThemeBackground {
            asset,
            newly_created,
        })
    }

    pub fn read(&self, asset: &ThemeBackgroundAsset) -> Result<Vec<u8>, ThemeAssetError> {
        asset.validate()?;
        let path = self.path_for_id(&asset.id)?;
        let bytes = self.validate_file(&path, &asset.id)?;
        if bytes.len() as u64 != asset.byte_size {
            return Err(ThemeAssetError::Corrupt);
        }
        let reader = ImageReader::new(Cursor::new(bytes.as_slice()))
            .with_guessed_format()
            .map_err(ThemeAssetError::Io)?;
        let (width, height) = reader.into_dimensions()?;
        if width != asset.width || height != asset.height {
            return Err(ThemeAssetError::Corrupt);
        }
        Ok(bytes)
    }

    pub fn remove(&self, id: &str) -> Result<(), ThemeAssetError> {
        let path = self.path_for_id(id)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn reconcile(&self, active_id: Option<&str>) -> Result<usize, ThemeAssetError> {
        if let Some(id) = active_id {
            let path = self.path_for_id(id)?;
            self.validate_file(&path, id)?;
        }
        let mut removed = 0;
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let keep = active_id.is_some_and(|id| name == format!("{id}.png"));
            if keep {
                continue;
            }
            let metadata = entry.metadata()?;
            if metadata.is_file() {
                fs::remove_file(entry.path())?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn path_for_id(&self, id: &str) -> Result<PathBuf, ThemeAssetError> {
        if !canonical_asset_id(id) {
            return Err(ThemeAssetError::InvalidMetadata);
        }
        Ok(self.root.join(format!("{id}.png")))
    }

    fn validate_file(&self, path: &Path, expected_id: &str) -> Result<Vec<u8>, ThemeAssetError> {
        let metadata = fs::symlink_metadata(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ThemeAssetError::Missing
            } else {
                ThemeAssetError::Io(error)
            }
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || is_reparse_point(&metadata) {
            return Err(ThemeAssetError::Corrupt);
        }
        if metadata.len() == 0 || metadata.len() > MAX_ENCODED_BYTES {
            return Err(ThemeAssetError::Corrupt);
        }
        let file = File::open(path)?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(MAX_ENCODED_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_ENCODED_BYTES
            || format!("{:x}", Sha256::digest(&bytes)) != expected_id
            || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        {
            return Err(ThemeAssetError::Corrupt);
        }
        Ok(bytes)
    }

    fn write_atomic(
        &self,
        destination: &Path,
        id: &str,
        bytes: &[u8],
    ) -> Result<bool, ThemeAssetError> {
        let nonce = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary = self
            .root
            .join(format!(".{id}.{}.{}.tmp", std::process::id(), nonce));
        let result = (|| -> Result<bool, ThemeAssetError> {
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            let mut writer = BufWriter::new(file);
            writer.write_all(bytes)?;
            writer.flush()?;
            writer.get_ref().sync_all()?;
            drop(writer);
            match fs::rename(&temporary, destination) {
                Ok(()) => Ok(true),
                Err(_) if destination.exists() => {
                    self.validate_file(destination, id)?;
                    let _ = fs::remove_file(&temporary);
                    Ok(false)
                }
                Err(error) => Err(error.into()),
            }
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

fn safe_file_name(path: &Path) -> String {
    let candidate = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("背景图片");
    let mut name = candidate
        .chars()
        .take(MAX_FILE_NAME_CHARS)
        .collect::<String>();
    name.retain(|character| character != '/' && character != '\\' && !character.is_control());
    if name.is_empty() {
        "背景图片".to_owned()
    } else {
        name
    }
}

fn canonical_asset_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), ThemeAssetError> {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(ThemeAssetError::TooLarge)?;
    let rgba_bytes = pixels.checked_mul(4).ok_or(ThemeAssetError::TooLarge)?;
    if width == 0
        || height == 0
        || width > MAX_WIDTH
        || height > MAX_HEIGHT
        || pixels > MAX_PIXELS
        || rgba_bytes > MAX_RGBA_BYTES
    {
        return Err(ThemeAssetError::TooLarge);
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use image::{DynamicImage, RgbaImage};
    use tempfile::tempdir;

    use super::*;

    fn service() -> (tempfile::TempDir, ThemeAssetService) {
        let temp = tempdir().unwrap();
        let storage = StorageService::initialize(temp.path()).unwrap();
        let service = ThemeAssetService::initialize(&storage).unwrap();
        (temp, service)
    }

    fn source_image(path: &Path, format: ImageFormat) {
        let image = DynamicImage::ImageRgba8(
            RgbaImage::from_raw(2, 1, vec![10, 20, 30, 255, 40, 50, 60, 255]).unwrap(),
        );
        image.save_with_format(path, format).unwrap();
    }

    #[test]
    fn png_jpeg_and_webp_are_imported_as_content_addressed_png() {
        let (temp, service) = service();
        for (name, format) in [
            ("source.png", ImageFormat::Png),
            ("source.jpg", ImageFormat::Jpeg),
            ("source.webp", ImageFormat::WebP),
        ] {
            let path = temp.path().join(name);
            source_image(&path, format);
            let imported = service.import(&path).unwrap();
            assert_eq!(imported.asset.file_name, name);
            assert_eq!((imported.asset.width, imported.asset.height), (2, 1));
            assert!(service
                .read(&imported.asset)
                .unwrap()
                .starts_with(b"\x89PNG\r\n\x1a\n"));
        }
    }

    #[test]
    fn duplicate_import_reuses_file_and_reconcile_keeps_only_active_asset() {
        let (temp, service) = service();
        let first_path = temp.path().join("first.png");
        source_image(&first_path, ImageFormat::Png);
        let first = service.import(&first_path).unwrap();
        let duplicate = service.import(&first_path).unwrap();
        assert!(first.newly_created);
        assert!(!duplicate.newly_created);
        assert_eq!(first.asset.id, duplicate.asset.id);

        let second_path = temp.path().join("second.png");
        let second_image =
            DynamicImage::ImageRgba8(RgbaImage::from_raw(1, 1, vec![90, 80, 70, 255]).unwrap());
        second_image
            .save_with_format(&second_path, ImageFormat::Png)
            .unwrap();
        let second = service.import(&second_path).unwrap();
        assert_eq!(service.reconcile(Some(&second.asset.id)).unwrap(), 1);
        assert!(matches!(
            service.read(&first.asset),
            Err(ThemeAssetError::Missing)
        ));
        assert!(service.read(&second.asset).is_ok());
    }

    #[test]
    fn corrupt_and_unsupported_sources_are_rejected_without_managed_output() {
        let (temp, service) = service();
        let corrupt = temp.path().join("not-an-image.png");
        fs::write(&corrupt, b"not an image").unwrap();
        assert!(matches!(
            service.import(&corrupt),
            Err(ThemeAssetError::UnsupportedFormat)
        ));
        assert_eq!(fs::read_dir(&service.root).unwrap().count(), 0);
    }

    #[test]
    fn metadata_and_ids_reject_paths_and_invalid_ranges() {
        let (_temp, service) = service();
        let invalid = ThemeBackgroundAsset {
            id: "../outside".to_owned(),
            file_name: "../outside.png".to_owned(),
            byte_size: 1,
            width: 1,
            height: 1,
        };
        assert!(matches!(
            service.read(&invalid),
            Err(ThemeAssetError::InvalidMetadata)
        ));
        assert!(matches!(
            service.remove("../outside"),
            Err(ThemeAssetError::InvalidMetadata)
        ));
    }
}
