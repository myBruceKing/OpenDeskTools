use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};
use thiserror::Error;

use super::storage::{StorageError, StorageService};

const ICON_DIRECTORY: &str = "files/source-icons";
// History rows use up to 40 CSS pixels and Windows commonly renders the
// WebView at 150-200% DPI. A 96 px cache keeps the browser on the downscale
// path for those slots while preserving the existing PNG/reference contract.
const ICON_SIZE: u32 = 96;
#[cfg(windows)]
const ICON_EXTRACTION_SIZE: u16 = 256;
const _: () = assert!(ICON_SIZE >= 96);
#[cfg(windows)]
const _: () = assert!(ICON_EXTRACTION_SIZE as u32 > ICON_SIZE);
const MAX_ICON_BYTES: u64 = 1024 * 1024;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum SourceIconError {
    #[error("source icon storage failed")]
    Storage(#[from] StorageError),
    #[error("source icon I/O failed")]
    Io(#[from] std::io::Error),
    #[error("source icon encoding failed")]
    Encode(#[from] png::EncodingError),
    #[error("source icon is unavailable")]
    Unavailable,
    #[error("source icon reference is invalid")]
    InvalidReference,
}

#[derive(Debug)]
pub struct SourceIconService {
    directory: PathBuf,
    storage: std::sync::Arc<StorageService>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SourceIconReconcileResult {
    pub invalid_references: Vec<String>,
}

impl SourceIconService {
    pub fn initialize(storage: std::sync::Arc<StorageService>) -> Result<Self, SourceIconError> {
        let directory = storage.resolve_relative_path(ICON_DIRECTORY)?;
        fs::create_dir_all(&directory)?;
        Ok(Self { directory, storage })
    }

    pub fn cache_executable(&self, executable: &Path) -> Result<Option<String>, SourceIconError> {
        self.cache_icon(executable, 0)
    }

    /// Caches a concrete Shell icon location. Shortcut resolution supplies an
    /// explicit resource path and index; ordinary clipboard sources use index
    /// zero through [`Self::cache_executable`].
    pub fn cache_icon(&self, icon_path: &Path, icon_index: i32) -> Result<Option<String>, SourceIconError> {
        #[cfg(not(windows))]
        {
            let _ = (icon_path, icon_index);
            Ok(None)
        }
        #[cfg(windows)]
        {
            let Some(rgba) = extract_windows_icon(icon_path, icon_index) else {
                return Ok(None);
            };
            let png = encode_png(ICON_SIZE, ICON_SIZE, &rgba)?;
            let hash = format!("{:x}", Sha256::digest(&png));
            let file_name = format!("{hash}.png");
            let destination = self.directory.join(&file_name);
            if !destination.exists() {
                let temp_name = format!(
                    ".{hash}.{}.{}.tmp",
                    std::process::id(),
                    TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
                );
                let temp = self.directory.join(temp_name);
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temp)?;
                let write_result = (|| {
                    file.write_all(&png)?;
                    file.flush()?;
                    file.sync_all()
                })();
                drop(file);
                if let Err(error) = write_result {
                    let _ = fs::remove_file(&temp);
                    return Err(error.into());
                }
                if let Err(error) = fs::rename(&temp, &destination) {
                    if destination.exists() {
                        let _ = fs::remove_file(&temp);
                    } else {
                        let _ = fs::remove_file(&temp);
                        return Err(error.into());
                    }
                }
            }
            Ok(Some(format!("{ICON_DIRECTORY}/{file_name}")))
        }
    }

    pub fn read(&self, reference: &str) -> Result<Vec<u8>, SourceIconError> {
        validate_reference(reference)?;
        let path = self.storage.resolve_relative_path(reference)?;
        let metadata = fs::metadata(&path).map_err(|_| SourceIconError::Unavailable)?;
        if metadata.len() == 0 || metadata.len() > MAX_ICON_BYTES {
            return Err(SourceIconError::Unavailable);
        }
        let bytes = fs::read(path).map_err(|_| SourceIconError::Unavailable)?;
        if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err(SourceIconError::Unavailable);
        }
        let expected_hash = reference
            .strip_prefix("files/source-icons/")
            .and_then(|name| name.strip_suffix(".png"))
            .ok_or(SourceIconError::InvalidReference)?;
        if format!("{:x}", Sha256::digest(&bytes)) != expected_hash {
            return Err(SourceIconError::Unavailable);
        }
        if !cached_png_is_compatible(&bytes) {
            return Err(SourceIconError::Unavailable);
        }
        Ok(bytes)
    }

    pub fn reconcile(&self, references: &[String]) -> SourceIconReconcileResult {
        let mut result = SourceIconReconcileResult::default();
        let referenced = references
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>();
        for reference in references {
            if self.read(reference).is_err() && !result.invalid_references.contains(reference) {
                result.invalid_references.push(reference.clone());
            }
        }
        let Ok(entries) = fs::read_dir(&self.directory) else {
            return result;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let reference = format!("{ICON_DIRECTORY}/{name}");
            if name.ends_with(".tmp") || !referenced.contains(reference.as_str()) {
                let _ = fs::remove_file(entry.path());
            }
        }
        result
    }

    pub fn remove_references(&self, references: &[String]) {
        for reference in references {
            if validate_reference(reference).is_err() {
                continue;
            }
            let Ok(path) = self.storage.resolve_relative_path(reference) else {
                continue;
            };
            let _ = fs::remove_file(path);
        }
    }
}

fn cached_png_is_compatible(bytes: &[u8]) -> bool {
    let Ok(reader) = png::Decoder::new(std::io::Cursor::new(bytes)).read_info() else {
        return false;
    };
    let info = reader.info();
    info.width == ICON_SIZE
        && info.height == ICON_SIZE
        && info.color_type == png::ColorType::Rgba
        && info.bit_depth == png::BitDepth::Eight
}

fn validate_reference(reference: &str) -> Result<(), SourceIconError> {
    let Some(file_name) = reference.strip_prefix("files/source-icons/") else {
        return Err(SourceIconError::InvalidReference);
    };
    let Some(hash) = file_name.strip_suffix(".png") else {
        return Err(SourceIconError::InvalidReference);
    };
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(SourceIconError::InvalidReference);
    }
    Ok(())
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, SourceIconError> {
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
fn extract_windows_icon(icon_path: &Path, icon_index: i32) -> Option<Vec<u8>> {
    use std::mem;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::null_mut;
    use windows_sys::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{DrawIconEx, DI_NORMAL};

    let path = icon_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let icon = extract_high_resolution_icon(&path, icon_index)?;
    let dc = unsafe { CreateCompatibleDC(null_mut()) };
    let mut pixels = null_mut();
    let mut info: BITMAPINFO = unsafe { mem::zeroed() };
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: ICON_SIZE as i32,
        biHeight: -(ICON_SIZE as i32),
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..unsafe { mem::zeroed() }
    };
    let bitmap = unsafe { CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut pixels, null_mut(), 0) };
    let mut result = None;
    if !dc.is_null() && !bitmap.is_null() && !pixels.is_null() {
        let previous = unsafe { SelectObject(dc, bitmap) };
        let drawn = unsafe {
            DrawIconEx(
                dc,
                0,
                0,
                icon.0,
                ICON_SIZE as i32,
                ICON_SIZE as i32,
                0,
                null_mut(),
                DI_NORMAL,
            )
        } != 0;
        if drawn {
            let bgra = unsafe {
                std::slice::from_raw_parts(
                    pixels.cast::<u8>(),
                    (ICON_SIZE * ICON_SIZE * 4) as usize,
                )
            };
            let mut rgba = Vec::with_capacity(bgra.len());
            for pixel in bgra.chunks_exact(4) {
                rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
            }
            result = Some(rgba);
        }
        unsafe { SelectObject(dc, previous) };
    }
    if !bitmap.is_null() {
        unsafe { DeleteObject(bitmap) };
    }
    if !dc.is_null() {
        unsafe { DeleteDC(dc) };
    }
    result
}

#[cfg(windows)]
struct OwnedIcon(windows_sys::Win32::UI::WindowsAndMessaging::HICON);

#[cfg(windows)]
impl OwnedIcon {
    fn new(icon: windows_sys::Win32::UI::WindowsAndMessaging::HICON) -> Option<Self> {
        (!icon.is_null()).then_some(Self(icon))
    }
}

#[cfg(windows)]
impl Drop for OwnedIcon {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::DestroyIcon(self.0);
        }
    }
}

#[cfg(windows)]
fn extract_high_resolution_icon(path: &[u16], icon_index: i32) -> Option<OwnedIcon> {
    use std::{mem, ptr::null_mut};
    use windows_sys::Win32::UI::Shell::{
        ExtractIconExW, SHDefExtractIconW, SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON,
        SHGFI_LARGEICON,
    };

    let mut requested = null_mut();
    let result = unsafe {
        SHDefExtractIconW(
            path.as_ptr(),
            icon_index,
            0,
            &mut requested,
            null_mut(),
            icon_size_request(ICON_EXTRACTION_SIZE, 0),
        )
    };
    let requested = OwnedIcon::new(requested);
    if result >= 0 && requested.is_some() {
        return requested;
    }
    drop(requested);

    // Some executables do not cooperate with the shell's sized extraction.
    // Keep compatibility through ExtractIconExW, but request only its large
    // icon so a system-small bitmap can never be selected and enlarged.
    let mut large = null_mut();
    if unsafe { ExtractIconExW(path.as_ptr(), icon_index, &mut large, null_mut(), 1) } != 0 {
        if let Some(icon) = OwnedIcon::new(large) {
            return Some(icon);
        }
    }

    // Some signed / packaged executables reject both resource extraction APIs.
    // Ask the Shell for its actual large file icon as a final availability
    // fallback. The normal route above remains the high-resolution path.
    let mut info: SHFILEINFOW = unsafe { mem::zeroed() };
    let result = unsafe {
        SHGetFileInfoW(
            path.as_ptr(),
            0,
            &mut info,
            mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        )
    };
    (result != 0).then(|| OwnedIcon::new(info.hIcon)).flatten()
}

#[cfg(windows)]
const fn icon_size_request(large: u16, small: u16) -> u32 {
    large as u32 | ((small as u32) << 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_content_addressed_icon_references_are_accepted() {
        let hash = "a".repeat(64);
        assert!(validate_reference(&format!("files/source-icons/{hash}.png")).is_ok());
        for invalid in [
            "../icon.png",
            "files/source-icons/a.png",
            "files/source-icons/ABC.png",
        ] {
            assert!(validate_reference(invalid).is_err());
        }
    }

    #[cfg(windows)]
    #[test]
    fn system_icon_request_uses_a_large_source_without_requesting_a_small_icon() {
        assert_eq!(
            icon_size_request(ICON_EXTRACTION_SIZE, 0),
            u32::from(ICON_EXTRACTION_SIZE)
        );
        assert_eq!(icon_size_request(0, 16), 16_u32 << 16);
    }

    #[test]
    fn cached_png_preserves_high_resolution_dimensions_and_alpha() {
        use std::io::Cursor;

        let mut rgba = vec![0_u8; (ICON_SIZE * ICON_SIZE * 4) as usize];
        rgba[..8].copy_from_slice(&[10, 20, 30, 17, 40, 50, 60, 231]);
        let encoded = encode_png(ICON_SIZE, ICON_SIZE, &rgba).unwrap();
        let decoder = png::Decoder::new(Cursor::new(encoded));
        let mut reader = decoder.read_info().unwrap();
        let mut decoded = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut decoded).unwrap();

        assert_eq!((info.width, info.height), (ICON_SIZE, ICON_SIZE));
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(info.bit_depth, png::BitDepth::Eight);
        assert_eq!(&decoded[..8], &[10, 20, 30, 17, 40, 50, 60, 231]);
        assert!(cached_png_is_compatible(
            &encode_png(ICON_SIZE, ICON_SIZE, &rgba).unwrap()
        ));
        assert!(!cached_png_is_compatible(
            &encode_png(32, 32, &vec![0; 32 * 32 * 4]).unwrap()
        ));
    }

    #[test]
    fn reconcile_reports_and_deletes_legacy_low_resolution_icons() {
        let temp = tempfile::tempdir().unwrap();
        let storage = std::sync::Arc::new(StorageService::initialize(temp.path()).unwrap());
        let service = SourceIconService::initialize(storage).unwrap();
        let legacy = encode_png(32, 32, &vec![0; 32 * 32 * 4]).unwrap();
        let hash = format!("{:x}", Sha256::digest(&legacy));
        let reference = format!("{ICON_DIRECTORY}/{hash}.png");
        let path = service.directory.join(format!("{hash}.png"));
        fs::write(&path, legacy).unwrap();

        assert!(matches!(
            service.read(&reference),
            Err(SourceIconError::Unavailable)
        ));
        let result = service.reconcile(std::slice::from_ref(&reference));
        assert_eq!(
            result,
            SourceIconReconcileResult {
                invalid_references: vec![reference.clone()],
            }
        );
        assert!(path.exists());
        service.remove_references(&result.invalid_references);
        assert!(!path.exists());
    }
}
