use std::sync::Mutex;
use std::time::Duration;

use thiserror::Error;

use super::clipboard::ClipboardWriteContent;

const CF_UNICODETEXT_FORMAT: u32 = 13;
const CF_HDROP_FORMAT: u32 = 15;
const CF_DIBV5_FORMAT: u32 = 17;
const OPEN_ATTEMPTS: usize = 5;
const OPEN_RETRY_DELAY: Duration = Duration::from_millis(12);

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ClipboardWriterError {
    #[cfg(not(windows))]
    #[error("clipboard writing is unavailable on this platform")]
    UnsupportedPlatform,
    #[error("clipboard writer lock is poisoned")]
    LockPoisoned,
    #[error("clipboard is temporarily busy")]
    Busy,
    #[error("Windows clipboard operation failed: {0}")]
    WindowsApi(&'static str),
    #[error("clipboard content exceeds the supported size")]
    TooLarge,
    #[error("clipboard content is invalid")]
    InvalidContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClipboardFormatBytes {
    format: u32,
    bytes: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct ClipboardWriter {
    operation_lock: Mutex<()>,
}

impl ClipboardWriter {
    /// Replaces the system clipboard with already validated OpenDeskTools
    /// content. This is used by transformations such as F4 after their result
    /// has been persisted to the internal history; it intentionally shares the
    /// same sequence guard and listener-suppression contract as history copy.
    #[cfg_attr(test, allow(dead_code))]
    pub fn replace_current<F>(
        &self,
        owner_window: usize,
        content: &ClipboardWriteContent,
        mut suppress: F,
    ) -> Result<u32, ClipboardWriterError>
    where
        F: FnMut(u32),
    {
        self.transaction(owner_window, |transaction| {
            transaction
                .replace_current(content, &mut suppress)?
                .ok_or(ClipboardWriterError::Busy)
        })
    }

    pub fn transaction<T, E, F>(&self, owner_window: usize, operation: F) -> Result<T, E>
    where
        E: From<ClipboardWriterError>,
        F: FnOnce(&mut ClipboardWriterTransaction<'_>) -> Result<T, E>,
    {
        let guard = self
            .operation_lock
            .lock()
            .map_err(|_| E::from(ClipboardWriterError::LockPoisoned))?;
        #[cfg(windows)]
        {
            let mut transaction = ClipboardWriterTransaction {
                _guard: guard,
                ops: SystemClipboardOps { owner_window },
            };
            operation(&mut transaction)
        }
        #[cfg(not(windows))]
        {
            let _ = (guard, owner_window, operation);
            Err(E::from(ClipboardWriterError::UnsupportedPlatform))
        }
    }
}

pub struct ClipboardWriterTransaction<'a> {
    _guard: std::sync::MutexGuard<'a, ()>,
    #[cfg(windows)]
    ops: SystemClipboardOps,
}

impl ClipboardWriterTransaction<'_> {
    /// Replaces the current clipboard using Windows clipboard-history semantics.
    ///
    /// The old clipboard formats are intentionally neither enumerated nor restored.
    /// The selected payload is fully validated and allocated before EmptyClipboard,
    /// and a sequence guard prevents overwriting a concurrent external copy.
    pub fn replace_current<F>(
        &mut self,
        content: &ClipboardWriteContent,
        suppress: &mut F,
    ) -> Result<Option<u32>, ClipboardWriterError>
    where
        F: FnMut(u32),
    {
        let formats = formats_for_content(content)?;
        #[cfg(windows)]
        {
            replace_current_formats_with_ops(&mut self.ops, &formats, suppress)
        }
        #[cfg(not(windows))]
        {
            let _ = (formats, suppress);
            Err(ClipboardWriterError::UnsupportedPlatform)
        }
    }
}

fn formats_for_content(
    content: &ClipboardWriteContent,
) -> Result<Vec<ClipboardFormatBytes>, ClipboardWriterError> {
    match content {
        ClipboardWriteContent::Text(text) => {
            let mut bytes = Vec::with_capacity((text.encode_utf16().count() + 1).saturating_mul(2));
            for unit in text.encode_utf16().chain(std::iter::once(0)) {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
            Ok(vec![ClipboardFormatBytes {
                format: CF_UNICODETEXT_FORMAT,
                bytes,
            }])
        }
        ClipboardWriteContent::Image {
            width,
            height,
            rgba,
        } => Ok(vec![ClipboardFormatBytes {
            format: CF_DIBV5_FORMAT,
            bytes: rgba_to_dib_v5(*width, *height, rgba)?,
        }]),
        ClipboardWriteContent::Files { paths } => Ok(vec![ClipboardFormatBytes {
            format: CF_HDROP_FORMAT,
            bytes: paths_to_hdrop(paths)?,
        }]),
    }
}

fn paths_to_hdrop(paths: &[Vec<u16>]) -> Result<Vec<u8>, ClipboardWriterError> {
    use super::clipboard::{
        MAX_CLIPBOARD_FILES, MAX_CLIPBOARD_FILE_PATHS_JSON_BYTES, MAX_CLIPBOARD_FILE_PATH_UNITS,
    };

    if paths.is_empty() || paths.len() > MAX_CLIPBOARD_FILES {
        return Err(ClipboardWriterError::InvalidContent);
    }
    let units = paths.iter().try_fold(1_usize, |total, path| {
        if path.is_empty() || path.len() > MAX_CLIPBOARD_FILE_PATH_UNITS || path.contains(&0) {
            return Err(ClipboardWriterError::InvalidContent);
        }
        total
            .checked_add(path.len())
            .and_then(|value| value.checked_add(1))
            .ok_or(ClipboardWriterError::TooLarge)
    })?;
    let total = 20_usize
        .checked_add(units.checked_mul(2).ok_or(ClipboardWriterError::TooLarge)?)
        .ok_or(ClipboardWriterError::TooLarge)?;
    if total > MAX_CLIPBOARD_FILE_PATHS_JSON_BYTES {
        return Err(ClipboardWriterError::TooLarge);
    }
    let mut bytes = Vec::with_capacity(total);
    bytes.extend_from_slice(&20_u32.to_le_bytes());
    bytes.extend_from_slice(&0_i32.to_le_bytes());
    bytes.extend_from_slice(&0_i32.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    for path in paths {
        for unit in path.iter().copied().chain(std::iter::once(0)) {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    Ok(bytes)
}

fn rgba_to_dib_v5(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, ClipboardWriterError> {
    if width == 0 || height == 0 || width > i32::MAX as u32 || height > i32::MAX as u32 {
        return Err(ClipboardWriterError::InvalidContent);
    }
    let pixels = usize::try_from(
        u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(ClipboardWriterError::TooLarge)?,
    )
    .map_err(|_| ClipboardWriterError::TooLarge)?;
    let pixel_bytes = pixels
        .checked_mul(4)
        .ok_or(ClipboardWriterError::TooLarge)?;
    if rgba.len() != pixel_bytes {
        return Err(ClipboardWriterError::InvalidContent);
    }
    let total = 124_usize
        .checked_add(pixel_bytes)
        .ok_or(ClipboardWriterError::TooLarge)?;
    let mut dib = vec![0_u8; total];
    dib[0..4].copy_from_slice(&124_u32.to_le_bytes());
    dib[4..8].copy_from_slice(&(width as i32).to_le_bytes());
    dib[8..12].copy_from_slice(&(-(height as i32)).to_le_bytes());
    dib[12..14].copy_from_slice(&1_u16.to_le_bytes());
    dib[14..16].copy_from_slice(&32_u16.to_le_bytes());
    dib[16..20].copy_from_slice(&3_u32.to_le_bytes());
    dib[20..24].copy_from_slice(&(pixel_bytes as u32).to_le_bytes());
    dib[40..44].copy_from_slice(&0x00ff_0000_u32.to_le_bytes());
    dib[44..48].copy_from_slice(&0x0000_ff00_u32.to_le_bytes());
    dib[48..52].copy_from_slice(&0x0000_00ff_u32.to_le_bytes());
    dib[52..56].copy_from_slice(&0xff00_0000_u32.to_le_bytes());
    dib[56..60].copy_from_slice(&0x7352_4742_u32.to_le_bytes());
    for (source, target) in rgba.chunks_exact(4).zip(dib[124..].chunks_exact_mut(4)) {
        target.copy_from_slice(&[source[2], source[1], source[0], source[3]]);
    }
    Ok(dib)
}

trait ClipboardOps {
    type Prepared;
    fn open(&mut self) -> bool;
    fn close(&mut self);
    fn sequence(&mut self) -> u32;
    fn empty(&mut self) -> bool;
    fn prepare_bytes(
        &mut self,
        format: u32,
        bytes: &[u8],
    ) -> Result<Self::Prepared, ClipboardWriterError>;
    fn set_prepared(&mut self, prepared: Self::Prepared) -> bool;
}

struct OpenGuard<'a, O: ClipboardOps>(&'a mut O);
impl<O: ClipboardOps> Drop for OpenGuard<'_, O> {
    fn drop(&mut self) {
        self.0.close();
    }
}

fn open_with_retries<O: ClipboardOps>(
    ops: &mut O,
) -> Result<OpenGuard<'_, O>, ClipboardWriterError> {
    for attempt in 0..OPEN_ATTEMPTS {
        if ops.open() {
            return Ok(OpenGuard(ops));
        }
        if attempt + 1 < OPEN_ATTEMPTS {
            std::thread::sleep(OPEN_RETRY_DELAY);
        }
    }
    Err(ClipboardWriterError::Busy)
}

#[derive(Debug)]
struct WriteFailure {
    error: ClipboardWriterError,
}

fn prepare_formats<O: ClipboardOps>(
    ops: &mut O,
    formats: &[ClipboardFormatBytes],
) -> Result<Vec<O::Prepared>, ClipboardWriterError> {
    formats
        .iter()
        .map(|format| ops.prepare_bytes(format.format, &format.bytes))
        .collect()
}

fn write_prepared<O: ClipboardOps>(
    ops: &mut O,
    prepared: Vec<O::Prepared>,
    expected_sequence: Option<u32>,
    suppress: &mut impl FnMut(u32),
) -> Result<Option<u32>, WriteFailure> {
    let clipboard = open_with_retries(ops).map_err(|error| WriteFailure { error })?;
    if expected_sequence.is_some_and(|expected| clipboard.0.sequence() != expected) {
        return Ok(None);
    }
    if !clipboard.0.empty() {
        return Err(WriteFailure {
            error: ClipboardWriterError::WindowsApi("EmptyClipboard"),
        });
    }
    for prepared in prepared {
        if !clipboard.0.set_prepared(prepared) {
            let sequence = clipboard.0.sequence();
            if sequence != 0 {
                suppress(sequence);
            }
            return Err(WriteFailure {
                error: ClipboardWriterError::WindowsApi("SetClipboardData"),
            });
        }
    }
    let sequence = clipboard.0.sequence();
    if sequence == 0 {
        return Err(WriteFailure {
            error: ClipboardWriterError::Busy,
        });
    }
    suppress(sequence);
    Ok(Some(sequence))
}

fn replace_current_formats_with_ops<O: ClipboardOps>(
    ops: &mut O,
    formats: &[ClipboardFormatBytes],
    suppress: &mut impl FnMut(u32),
) -> Result<Option<u32>, ClipboardWriterError> {
    let expected_sequence = ops.sequence();
    if expected_sequence == 0 {
        return Err(ClipboardWriterError::Busy);
    }
    // Allocate every HGLOBAL before Open/EmptyClipboard. If another process
    // changes the clipboard during allocation, write_prepared returns None.
    let prepared = prepare_formats(ops, formats)?;
    write_prepared(ops, prepared, Some(expected_sequence), suppress)
        .map_err(|failure| failure.error)
}

#[cfg(windows)]
struct SystemClipboardOps {
    owner_window: usize,
}

#[cfg(windows)]
struct SystemPreparedClipboardData {
    format: u32,
    handle: usize,
}

#[cfg(windows)]
impl Drop for SystemPreparedClipboardData {
    fn drop(&mut self) {
        if self.handle != 0 {
            unsafe {
                windows_sys::Win32::Foundation::GlobalFree(self.handle as _);
            }
        }
    }
}

#[cfg(windows)]
impl ClipboardOps for SystemClipboardOps {
    type Prepared = SystemPreparedClipboardData;
    fn open(&mut self) -> bool {
        unsafe {
            windows_sys::Win32::System::DataExchange::OpenClipboard(self.owner_window as _) != 0
        }
    }
    fn close(&mut self) {
        unsafe {
            windows_sys::Win32::System::DataExchange::CloseClipboard();
        }
    }
    fn sequence(&mut self) -> u32 {
        unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() }
    }
    fn empty(&mut self) -> bool {
        unsafe { windows_sys::Win32::System::DataExchange::EmptyClipboard() != 0 }
    }
    fn prepare_bytes(
        &mut self,
        format: u32,
        bytes: &[u8],
    ) -> Result<Self::Prepared, ClipboardWriterError> {
        let handle = prepare_global_bytes(&mut SystemGlobalMemory, bytes)
            .ok_or(ClipboardWriterError::WindowsApi("GlobalAlloc"))?;
        Ok(SystemPreparedClipboardData { format, handle })
    }
    fn set_prepared(&mut self, mut prepared: Self::Prepared) -> bool {
        let succeeded = !unsafe {
            windows_sys::Win32::System::DataExchange::SetClipboardData(
                prepared.format,
                prepared.handle as _,
            )
        }
        .is_null();
        if succeeded {
            prepared.handle = 0;
        }
        succeeded
    }
}

trait GlobalMemoryApi {
    fn allocate(&mut self, size: usize) -> Option<usize>;
    fn lock(&mut self, handle: usize) -> Option<*mut u8>;
    fn unlock(&mut self, handle: usize);
    fn free(&mut self, handle: usize);
    #[cfg(test)]
    fn set_clipboard_data(&mut self, format: u32, handle: usize) -> bool;
}

fn prepare_global_bytes<M: GlobalMemoryApi>(memory: &mut M, bytes: &[u8]) -> Option<usize> {
    let handle = memory.allocate(bytes.len())?;
    let Some(pointer) = memory.lock(handle) else {
        memory.free(handle);
        return None;
    };
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer, bytes.len());
    }
    memory.unlock(handle);
    Some(handle)
}

#[cfg(test)]
fn transfer_global_bytes<M: GlobalMemoryApi>(memory: &mut M, format: u32, bytes: &[u8]) -> bool {
    let Some(handle) = prepare_global_bytes(memory, bytes) else {
        return false;
    };
    if memory.set_clipboard_data(format, handle) {
        true
    } else {
        memory.free(handle);
        false
    }
}

#[cfg(windows)]
struct SystemGlobalMemory;

#[cfg(windows)]
impl GlobalMemoryApi for SystemGlobalMemory {
    fn allocate(&mut self, size: usize) -> Option<usize> {
        let handle = unsafe { windows_sys::Win32::System::Memory::GlobalAlloc(0x0002, size) };
        (!handle.is_null()).then_some(handle as usize)
    }
    fn lock(&mut self, handle: usize) -> Option<*mut u8> {
        let pointer =
            unsafe { windows_sys::Win32::System::Memory::GlobalLock(handle as _) } as *mut u8;
        (!pointer.is_null()).then_some(pointer)
    }
    fn unlock(&mut self, handle: usize) {
        unsafe {
            windows_sys::Win32::System::Memory::GlobalUnlock(handle as _);
        }
    }
    fn free(&mut self, handle: usize) {
        unsafe {
            windows_sys::Win32::Foundation::GlobalFree(handle as _);
        }
    }
    #[cfg(test)]
    fn set_clipboard_data(&mut self, format: u32, handle: usize) -> bool {
        !unsafe { windows_sys::Win32::System::DataExchange::SetClipboardData(format, handle as _) }
            .is_null()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[derive(Default)]
    struct FakeMemory {
        bytes: Vec<u8>,
        allocs: usize,
        unlocks: usize,
        frees: usize,
        set_result: bool,
        lock_result: bool,
    }
    impl GlobalMemoryApi for FakeMemory {
        fn allocate(&mut self, size: usize) -> Option<usize> {
            self.allocs += 1;
            self.bytes.resize(size, 0);
            Some(1)
        }
        fn lock(&mut self, _handle: usize) -> Option<*mut u8> {
            self.lock_result.then_some(self.bytes.as_mut_ptr())
        }
        fn unlock(&mut self, _handle: usize) {
            self.unlocks += 1;
        }
        fn free(&mut self, _handle: usize) {
            self.frees += 1;
        }
        fn set_clipboard_data(&mut self, _format: u32, _handle: usize) -> bool {
            self.set_result
        }
    }

    #[test]
    fn global_memory_ownership_transfers_only_after_successful_set() {
        let mut success = FakeMemory {
            set_result: true,
            lock_result: true,
            ..Default::default()
        };
        assert!(transfer_global_bytes(&mut success, 13, &[1, 2, 3]));
        assert_eq!((success.allocs, success.unlocks, success.frees), (1, 1, 0));
        assert_eq!(success.bytes, vec![1, 2, 3]);
        let mut failure = FakeMemory {
            set_result: false,
            lock_result: true,
            ..Default::default()
        };
        assert!(!transfer_global_bytes(&mut failure, 13, &[1]));
        assert_eq!((failure.unlocks, failure.frees), (1, 1));
        let mut lock_failure = FakeMemory {
            set_result: true,
            lock_result: false,
            ..Default::default()
        };
        assert!(!transfer_global_bytes(&mut lock_failure, 13, &[1]));
        assert_eq!((lock_failure.unlocks, lock_failure.frees), (0, 1));
    }

    #[derive(Default)]
    struct FakeOps {
        opens: VecDeque<bool>,
        closes: usize,
        sequence: u32,
        formats: Vec<ClipboardFormatBytes>,
        unsupported_original_formats: Vec<u32>,
        emptied: usize,
        prepares_at_first_empty: Option<usize>,
        prepares: usize,
        sets: usize,
        prepare_fail_at: Option<usize>,
        set_fail_at: Vec<usize>,
        change_sequence_on_open: bool,
    }
    impl ClipboardOps for FakeOps {
        type Prepared = ClipboardFormatBytes;
        fn open(&mut self) -> bool {
            if self.change_sequence_on_open {
                self.change_sequence_on_open = false;
                self.sequence += 1;
            }
            self.opens.pop_front().unwrap_or(true)
        }
        fn close(&mut self) {
            self.closes += 1;
        }
        fn sequence(&mut self) -> u32 {
            self.sequence
        }
        fn empty(&mut self) -> bool {
            self.emptied += 1;
            self.prepares_at_first_empty.get_or_insert(self.prepares);
            self.formats.clear();
            self.unsupported_original_formats.clear();
            true
        }
        fn prepare_bytes(
            &mut self,
            format: u32,
            bytes: &[u8],
        ) -> Result<Self::Prepared, ClipboardWriterError> {
            self.prepares += 1;
            if self.prepare_fail_at == Some(self.prepares) {
                return Err(ClipboardWriterError::WindowsApi("GlobalAlloc"));
            }
            Ok(ClipboardFormatBytes {
                format,
                bytes: bytes.to_vec(),
            })
        }
        fn set_prepared(&mut self, prepared: Self::Prepared) -> bool {
            self.sets += 1;
            if self.set_fail_at.contains(&self.sets) {
                self.sequence += 1;
                return false;
            }
            self.formats.push(prepared);
            self.sequence += 1;
            true
        }
    }

    #[test]
    fn replacement_ignores_unsupported_original_formats_and_is_fully_prepared_before_empty() {
        let replacement = vec![format(CF_UNICODETEXT_FORMAT, 65)];
        let mut ops = FakeOps {
            sequence: 10,
            formats: vec![format(0xc001, 9)],
            // Simulates HTML/RTF/private formats that the old snapshot path rejected.
            unsupported_original_formats: vec![2, 0xc002],
            ..Default::default()
        };
        let mut suppressed = Vec::new();

        assert_eq!(
            replace_current_formats_with_ops(&mut ops, &replacement, &mut |sequence| {
                suppressed.push(sequence)
            })
            .unwrap(),
            Some(11)
        );
        assert_eq!(ops.emptied, 1);
        assert_eq!(ops.prepares_at_first_empty, Some(replacement.len()));
        assert_eq!(ops.formats, replacement);
        assert!(ops.unsupported_original_formats.is_empty());
        assert_eq!(suppressed, vec![11]);
    }

    #[test]
    fn text_and_rgba_generate_unicode_and_top_down_dibv5() {
        let text = formats_for_content(&ClipboardWriteContent::Text("A".to_owned())).unwrap();
        assert_eq!(
            text[0],
            ClipboardFormatBytes {
                format: CF_UNICODETEXT_FORMAT,
                bytes: vec![65, 0, 0, 0]
            }
        );
        let image = formats_for_content(&ClipboardWriteContent::Image {
            width: 1,
            height: 1,
            rgba: vec![1, 2, 3, 4],
        })
        .unwrap();
        assert_eq!(image[0].format, CF_DIBV5_FORMAT);
        assert_eq!(&image[0].bytes[8..12], &(-1_i32).to_le_bytes());
        assert_eq!(&image[0].bytes[124..], &[3, 2, 1, 4]);
    }

    #[test]
    fn file_paths_generate_one_wide_dropfiles_payload_with_double_terminator() {
        let paths = vec![
            r"C:\tmp\notes.txt".encode_utf16().collect::<Vec<_>>(),
            r"D:\images\图.png".encode_utf16().collect::<Vec<_>>(),
        ];
        let formats = formats_for_content(&ClipboardWriteContent::Files {
            paths: paths.clone(),
        })
        .unwrap();
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].format, CF_HDROP_FORMAT);
        let bytes = &formats[0].bytes;
        assert_eq!(u32::from_le_bytes(bytes[0..4].try_into().unwrap()), 20);
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 1);
        let units = bytes[20..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        assert_eq!(&units[units.len() - 2..], &[0, 0]);
        let rebuilt = units[..units.len() - 1]
            .split(|unit| *unit == 0)
            .filter(|path| !path.is_empty())
            .map(<[u16]>::to_vec)
            .collect::<Vec<_>>();
        assert_eq!(rebuilt, paths);
    }

    #[test]
    fn file_drop_writer_rejects_empty_embedded_nul_and_path_count_bombs() {
        assert_eq!(
            paths_to_hdrop(&[]),
            Err(ClipboardWriterError::InvalidContent)
        );
        assert_eq!(
            paths_to_hdrop(&[vec![b'C' as u16, 0, b'x' as u16]]),
            Err(ClipboardWriterError::InvalidContent)
        );
        let bomb = vec![vec![b'x' as u16]; super::super::clipboard::MAX_CLIPBOARD_FILES + 1];
        assert_eq!(
            paths_to_hdrop(&bomb),
            Err(ClipboardWriterError::InvalidContent)
        );
    }

    fn format(format: u32, value: u8) -> ClipboardFormatBytes {
        ClipboardFormatBytes {
            format,
            bytes: vec![value],
        }
    }

    #[test]
    fn allocation_failure_and_concurrent_change_do_not_mutate_clipboard() {
        let replacement = vec![format(CF_UNICODETEXT_FORMAT, 2)];
        let mut allocation_failure = FakeOps {
            sequence: 20,
            formats: vec![format(0xc001, 1)],
            prepare_fail_at: Some(1),
            ..Default::default()
        };
        assert!(replace_current_formats_with_ops(
            &mut allocation_failure,
            &replacement,
            &mut |_| {}
        )
        .is_err());
        assert_eq!(allocation_failure.emptied, 0);
        assert_eq!(allocation_failure.formats, vec![format(0xc001, 1)]);

        let mut changed = FakeOps {
            sequence: 20,
            formats: vec![format(0xc001, 1)],
            change_sequence_on_open: true,
            ..Default::default()
        };
        assert_eq!(
            replace_current_formats_with_ops(&mut changed, &replacement, &mut |_| panic!(
                "a rejected guarded write must not be suppressed"
            ))
            .unwrap(),
            None
        );
        assert_eq!(changed.emptied, 0);
        assert_eq!(changed.formats, vec![format(0xc001, 1)]);
        assert_eq!(changed.prepares, replacement.len());
    }

    #[test]
    fn set_clipboard_data_failure_is_explicit_and_suppresses_the_mutated_sequence() {
        let replacement = vec![format(CF_UNICODETEXT_FORMAT, 2)];
        let mut ops = FakeOps {
            sequence: 30,
            formats: vec![format(0xc001, 1)],
            set_fail_at: vec![1],
            ..Default::default()
        };
        let mut suppressed = Vec::new();
        assert_eq!(
            replace_current_formats_with_ops(&mut ops, &replacement, &mut |sequence| suppressed
                .push(sequence)),
            Err(ClipboardWriterError::WindowsApi("SetClipboardData"))
        );
        assert_eq!(ops.emptied, 1);
        assert_eq!(suppressed, vec![31]);
    }
}
