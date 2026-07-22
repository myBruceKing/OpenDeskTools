#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{params, OptionalExtension, Row};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::clipboard_settings::{
    self, ClipboardHistoryReuseStrategy, ClipboardSettings, ClipboardSettingsError,
};
use super::image::{ImageError, ImageService};
use super::source_icon::{SourceIconError, SourceIconService};
use super::storage::{StorageError, StorageService};

pub const CLIPBOARD_HISTORY_CAPACITY: u32 = 100;
pub const JS_MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SOURCE_APPLICATION_CHARS: usize = 256;
pub const MAX_SOURCE_PROCESS_CHARS: usize = 512;
pub const MAX_SEARCH_CHARS: usize = 256;
pub const MAX_CLIPBOARD_FILES: usize = 128;
pub const MAX_CLIPBOARD_FILE_PATH_UNITS: usize = 32_767;
pub const MAX_CLIPBOARD_FILE_PATHS_JSON_BYTES: usize = 1024 * 1024;

#[cfg(test)]
type AfterImageStoreHook = Arc<dyn Fn(&ClipboardCaptureMetadata) + Send + Sync + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardContentKind {
    Text,
    Image,
    Files,
}

impl ClipboardContentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Files => "files",
        }
    }

    fn parse(value: &str) -> Result<Self, ClipboardError> {
        match value {
            "text" => Ok(Self::Text),
            "image" => Ok(Self::Image),
            "files" => Ok(Self::Files),
            _ => Err(ClipboardError::CorruptRecord),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardHistoryItem {
    pub id: i64,
    pub kind: ClipboardContentKind,
    pub text_content: Option<String>,
    pub(crate) file_path: Option<String>,
    pub(crate) file_paths: Option<Vec<Vec<u16>>>,
    pub source_application: Option<String>,
    pub source_process: Option<String>,
    pub captured_at_ms: u64,
    pub byte_size: u64,
    pub is_favorite: bool,
    pub revision: u64,
    pub(crate) source_icon_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // The Win32 listener will construct this in the next clipboard slice.
pub struct ClipboardCaptureMetadata {
    pub captured_at_ms: u64,
    pub source_application: Option<String>,
    pub source_process: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardHistoryQuery {
    pub favorites_only: bool,
    pub search: Option<String>,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardHistoryPage {
    pub items: Vec<ClipboardHistoryItem>,
    pub total_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardWriteContent {
    Text(String),
    Image {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    Files {
        paths: Vec<Vec<u16>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // Kept as the truthful listener-facing outcome before listener wiring exists.
pub struct ClipboardRecordResult {
    pub retained: bool,
    pub item: Option<ClipboardHistoryItem>,
}

#[derive(Debug, Error)]
pub enum ClipboardError {
    #[error("clipboard storage operation failed")]
    Storage(#[from] StorageError),
    #[allow(dead_code)] // Produced by record_text once the listener is connected.
    #[error("clipboard text must not be empty")]
    EmptyText,
    #[error("clipboard text exceeds the supported byte limit")]
    TextTooLarge,
    #[error("clipboard source application exceeds the supported character limit")]
    SourceApplicationTooLong,
    #[error("clipboard source process exceeds the supported character limit")]
    SourceProcessTooLong,
    #[error("clipboard history search exceeds the supported character limit")]
    SearchTooLong,
    #[error("clipboard numeric value is outside the supported range")]
    NumericRange,
    #[error("clipboard history query limit must be between 1 and 100")]
    InvalidLimit,
    #[error("clipboard history item was not found")]
    NotFound,
    #[error("clipboard history has no usable latest item")]
    NoLatestItem,
    #[error("clipboard text duplicates another history item")]
    DuplicateText,
    #[error("clipboard item revision conflict")]
    RevisionConflict,
    #[error("clipboard file list is invalid")]
    InvalidFiles,
    #[error("one or more clipboard files are unavailable")]
    FilesUnavailable,
    #[error("clipboard history contains an invalid record")]
    CorruptRecord,
    #[error("clipboard image lifecycle lock is poisoned")]
    LifecycleLockPoisoned,
    #[error("clipboard image operation failed")]
    Image(#[from] ImageError),
    #[error("clipboard source icon is unavailable")]
    SourceIcon(#[from] SourceIconError),
    #[error("clipboard settings are invalid")]
    Settings(#[from] ClipboardSettingsError),
    #[error("clipboard settings state lock is poisoned")]
    SettingsLockPoisoned,
}

pub struct ClipboardService {
    storage: Arc<StorageService>,
    settings: Mutex<ClipboardSettings>,
    images: ImageService,
    source_icons: SourceIconService,
    image_lifecycle: Mutex<()>,
    corrupt_history_row_observations: AtomicU64,
    #[cfg(test)]
    after_image_store_hook: Mutex<Option<AfterImageStoreHook>>,
}

impl std::fmt::Debug for ClipboardService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClipboardService")
            .field("storage", &self.storage)
            .field("settings", &self.settings)
            .field("images", &self.images)
            .field("source_icons", &self.source_icons)
            .field(
                "corrupt_history_row_observations",
                &self
                    .corrupt_history_row_observations
                    .load(Ordering::Relaxed),
            )
            .finish_non_exhaustive()
    }
}

impl ClipboardService {
    #[cfg(test)]
    pub fn initialize(storage: Arc<StorageService>) -> Self {
        Self::try_initialize(storage).expect("clipboard image storage should initialize")
    }

    pub fn try_initialize(storage: Arc<StorageService>) -> Result<Self, ClipboardError> {
        let images = ImageService::initialize(&storage)?;
        let source_icons = SourceIconService::initialize(Arc::clone(&storage))?;
        let settings = clipboard_settings::load(&storage).unwrap_or_default();
        let service = Self {
            storage,
            settings: Mutex::new(settings),
            images,
            source_icons,
            image_lifecycle: Mutex::new(()),
            corrupt_history_row_observations: AtomicU64::new(0),
            #[cfg(test)]
            after_image_store_hook: Mutex::new(None),
        };
        service.reconcile_images()?;
        service.reconcile_source_icons()?;
        Ok(service)
    }

    pub fn settings(&self) -> Result<ClipboardSettings, ClipboardError> {
        self.settings
            .lock()
            .map(|settings| settings.clone())
            .map_err(|_| ClipboardError::SettingsLockPoisoned)
    }

    pub fn update_settings(&self, settings: ClipboardSettings) -> Result<u64, ClipboardError> {
        clipboard_settings::validate(&settings)?;
        clipboard_settings::save(&self.storage, &settings)?;
        *self
            .settings
            .lock()
            .map_err(|_| ClipboardError::SettingsLockPoisoned)? = settings;
        self.apply_settings_retention_and_capacity()
    }

    /// Runs the persisted retention and capacity policy for an existing data
    /// store.  The application calls this during startup so an elapsed
    /// retention period is enforced even when the user has not revisited the
    /// settings page or copied a new item.
    pub fn reconcile_retention_and_capacity(&self) -> Result<u64, ClipboardError> {
        self.apply_settings_retention_and_capacity()
    }

    #[allow(dead_code)] // Intentionally not exposed as a frontend command.
    pub fn record_text(
        &self,
        text: String,
        metadata: ClipboardCaptureMetadata,
    ) -> Result<ClipboardRecordResult, ClipboardError> {
        let settings = self.settings()?;
        if self.should_ignore_source(&metadata, &settings)
            || self.matches_sensitive_rule(&text, &settings)
        {
            return Ok(ClipboardRecordResult {
                retained: false,
                item: None,
            });
        }
        if text.is_empty() {
            return Err(ClipboardError::EmptyText);
        }
        if text.len() > MAX_TEXT_BYTES {
            return Err(ClipboardError::TextTooLarge);
        }
        validate_optional_chars(
            metadata.source_application.as_deref(),
            MAX_SOURCE_APPLICATION_CHARS,
            ClipboardError::SourceApplicationTooLong,
        )?;
        validate_optional_chars(
            metadata.source_process.as_deref(),
            MAX_SOURCE_PROCESS_CHARS,
            ClipboardError::SourceProcessTooLong,
        )?;
        validate_safe_integer(metadata.captured_at_ms)?;
        let byte_size = u64::try_from(text.len()).map_err(|_| ClipboardError::NumericRange)?;
        validate_safe_integer(byte_size)?;
        let captured_at_ms =
            i64::try_from(metadata.captured_at_ms).map_err(|_| ClipboardError::NumericRange)?;
        let byte_size = i64::try_from(byte_size).map_err(|_| ClipboardError::NumericRange)?;
        let content_hash = format!("{:x}", Sha256::digest(text.as_bytes()));
        let _lifecycle = self.lock_image_lifecycle()?;

        let (id, removed_images) = self.storage.transaction(|transaction| {
            let id = transaction.query_row(
                "INSERT INTO clipboard_history (
                    content_type,
                    text_content,
                    file_path,
                    content_hash,
                    source_application,
                    source_process,
                    captured_at_ms,
                    byte_size
                 ) VALUES ('text', ?1, NULL, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(content_type, content_hash) DO UPDATE SET
                    text_content = excluded.text_content,
                    source_application = excluded.source_application,
                    source_process = excluded.source_process,
                    captured_at_ms = excluded.captured_at_ms,
                    byte_size = excluded.byte_size,
                    source_icon_path = NULL,
                    content_revision = clipboard_history.content_revision + 1
                 RETURNING id",
                params![
                    text,
                    content_hash,
                    metadata.source_application,
                    metadata.source_process,
                    captured_at_ms,
                    byte_size
                ],
                |row| row.get::<_, i64>(0),
            )?;

            let removed_images = evict_excess(transaction, settings.max_items)?;
            Ok((id, removed_images))
        })?;
        self.remove_image_references(removed_images);

        let item = self.item_by_id(id)?;
        Ok(ClipboardRecordResult {
            retained: item.is_some(),
            item,
        })
    }

    pub fn record_image(
        &self,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        metadata: ClipboardCaptureMetadata,
    ) -> Result<ClipboardRecordResult, ClipboardError> {
        let settings = self.settings()?;
        if self.should_ignore_source(&metadata, &settings) {
            return Ok(ClipboardRecordResult {
                retained: false,
                item: None,
            });
        }
        validate_metadata(&metadata)?;
        let _lifecycle = self.lock_image_lifecycle()?;
        let stored = self.images.store_rgba(width, height, &rgba)?;
        #[cfg(test)]
        if let Some(hook) = self.after_image_store_hook.lock().unwrap().clone() {
            hook(&metadata);
        }
        validate_safe_integer(stored.byte_size)?;
        let captured_at_ms =
            i64::try_from(metadata.captured_at_ms).map_err(|_| ClipboardError::NumericRange)?;
        let byte_size =
            i64::try_from(stored.byte_size).map_err(|_| ClipboardError::NumericRange)?;
        let transaction_result = self.storage.transaction(|transaction| {
            let id = transaction.query_row(
                "INSERT INTO clipboard_history (
                    content_type, text_content, file_path, content_hash,
                    source_application, source_process, captured_at_ms, byte_size
                 ) VALUES ('image', NULL, ?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(content_type, content_hash) DO UPDATE SET
                    file_path = excluded.file_path,
                    source_application = excluded.source_application,
                    source_process = excluded.source_process,
                    captured_at_ms = excluded.captured_at_ms,
                    byte_size = excluded.byte_size,
                    source_icon_path = NULL,
                    content_revision = clipboard_history.content_revision + 1
                 RETURNING id",
                params![
                    stored.reference,
                    stored.hash,
                    metadata.source_application,
                    metadata.source_process,
                    captured_at_ms,
                    byte_size
                ],
                |row| row.get::<_, i64>(0),
            )?;
            Ok((id, evict_excess(transaction, settings.max_items)?))
        });
        let (id, removed_images) = match transaction_result {
            Ok(value) => value,
            Err(error) => {
                if stored.newly_created {
                    self.cleanup_if_unreferenced(&stored.reference);
                }
                return Err(error.into());
            }
        };
        self.remove_image_references(removed_images);
        let item = self.item_by_id(id)?;
        Ok(ClipboardRecordResult {
            retained: item.is_some(),
            item,
        })
    }

    pub fn record_files(
        &self,
        paths: Vec<Vec<u16>>,
        metadata: ClipboardCaptureMetadata,
    ) -> Result<ClipboardRecordResult, ClipboardError> {
        let settings = self.settings()?;
        if self.should_ignore_source(&metadata, &settings) {
            return Ok(ClipboardRecordResult {
                retained: false,
                item: None,
            });
        }
        validate_metadata(&metadata)?;
        let validated = validate_file_paths(paths, false)?;
        let file_paths_json =
            serde_json::to_string(&validated.paths).map_err(|_| ClipboardError::InvalidFiles)?;
        if file_paths_json.len() > MAX_CLIPBOARD_FILE_PATHS_JSON_BYTES {
            return Err(ClipboardError::InvalidFiles);
        }
        let content_hash = hash_file_paths(&validated.paths);
        let captured_at_ms =
            i64::try_from(metadata.captured_at_ms).map_err(|_| ClipboardError::NumericRange)?;
        let byte_size =
            i64::try_from(validated.byte_size).map_err(|_| ClipboardError::NumericRange)?;
        let _lifecycle = self.lock_image_lifecycle()?;
        let (id, removed_images) = self.storage.transaction(|transaction| {
            let id = transaction.query_row(
                "INSERT INTO clipboard_history (
                    content_type, text_content, file_path, file_paths_json, content_hash,
                    source_application, source_process, captured_at_ms, byte_size
                 ) VALUES ('files', NULL, NULL, ?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(content_type, content_hash) DO UPDATE SET
                    file_paths_json = excluded.file_paths_json,
                    source_application = excluded.source_application,
                    source_process = excluded.source_process,
                    captured_at_ms = excluded.captured_at_ms,
                    byte_size = excluded.byte_size,
                    source_icon_path = NULL,
                    content_revision = clipboard_history.content_revision + 1
                 RETURNING id",
                params![
                    file_paths_json,
                    content_hash,
                    metadata.source_application,
                    metadata.source_process,
                    captured_at_ms,
                    byte_size
                ],
                |row| row.get::<_, i64>(0),
            )?;
            Ok((id, evict_excess(transaction, settings.max_items)?))
        })?;
        self.remove_image_references(removed_images);
        let item = self.item_by_id(id)?;
        Ok(ClipboardRecordResult {
            retained: item.is_some(),
            item,
        })
    }

    pub fn history(
        &self,
        query: ClipboardHistoryQuery,
    ) -> Result<ClipboardHistoryPage, ClipboardError> {
        // A long-running tray process may not restart for weeks. Reconcile on
        // the user-visible history read as well as startup, so an elapsed
        // retention period is not merely a preference that waits for restart.
        // Unit tests intentionally use synthetic historical timestamps; their
        // policy behavior is covered through explicit reconciliation tests.
        #[cfg(not(test))]
        self.apply_settings_retention_and_capacity()?;
        if !(1..=CLIPBOARD_HISTORY_CAPACITY).contains(&query.limit) {
            return Err(ClipboardError::InvalidLimit);
        }
        if query
            .search
            .as_deref()
            .is_some_and(|search| search.chars().count() > MAX_SEARCH_CHARS)
        {
            return Err(ClipboardError::SearchTooLong);
        }
        let search = query.search.filter(|value| !value.is_empty());
        let favorite_filter = i64::from(query.favorites_only);
        let raw_items = self.storage.read(|connection| {
            let mut statement = connection.prepare(
                "SELECT
                    id,
                    content_type,
                    text_content,
                    file_path,
                    source_application,
                    source_process,
                    captured_at_ms,
                    byte_size,
                    is_favorite,
                    content_revision,
                    source_icon_path,
                    file_paths_json
                 FROM clipboard_history
                 WHERE (?1 = 0 OR is_favorite = 1)
                   AND (
                     ?2 IS NULL
                     OR instr(
                        lower(
                          coalesce(text_content, '') || ' ' ||
                          coalesce(source_application, '') || ' ' ||
                          coalesce(source_process, '')
                        ),
                        lower(?2)
                     ) > 0
                   )
                 ORDER BY captured_at_ms DESC, id DESC",
            )?;
            let items = statement
                .query_map(params![favorite_filter, search.as_deref()], raw_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(items)
        })?;

        let mut items = Vec::with_capacity(raw_items.len());
        let mut skipped = 0_u64;
        for raw in raw_items {
            match ClipboardHistoryItem::try_from(raw) {
                Ok(item) => items.push(item),
                Err(ClipboardError::CorruptRecord) => skipped = skipped.saturating_add(1),
                Err(error) => return Err(error),
            }
        }
        if skipped > 0 {
            let cumulative = self
                .corrupt_history_row_observations
                .fetch_add(skipped, Ordering::Relaxed)
                .saturating_add(skipped);
            eprintln!(
                "clipboard history isolated {skipped} corrupt record(s); cumulative observations: {cumulative}"
            );
        }
        let total_count = u64::try_from(items.len()).map_err(|_| ClipboardError::CorruptRecord)?;
        validate_safe_integer(total_count)?;
        items.truncate(query.limit as usize);
        Ok(ClipboardHistoryPage { items, total_count })
    }

    /// Records that an existing history item was deliberately reused.  The
    /// clipboard writer suppresses its own Windows notification, so this is the
    /// single path that preserves "reused item moves to the front" semantics.
    pub fn promote_item(&self, id: i64) -> Result<(), ClipboardError> {
        let captured_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| {
                u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
            });
        validate_safe_integer(captured_at_ms)?;
        let captured_at_ms =
            i64::try_from(captured_at_ms).map_err(|_| ClipboardError::NumericRange)?;
        let changed = self.storage.transaction(|transaction| {
            Ok(transaction.execute(
                "UPDATE clipboard_history SET captured_at_ms = ?1 WHERE id = ?2",
                params![captured_at_ms, id],
            )?)
        })?;
        if changed == 1 {
            Ok(())
        } else {
            Err(ClipboardError::NotFound)
        }
    }

    /// Applies the user's explicit history-reuse preference after a successful
    /// copy or input. External duplicate captures always merge and promote;
    /// this setting concerns only deliberate reuse of an existing record.
    pub fn apply_history_reuse(&self, id: i64) -> Result<(), ClipboardError> {
        if self.settings()?.history_reuse_strategy == ClipboardHistoryReuseStrategy::Promote {
            self.promote_item(id)?;
        }
        Ok(())
    }

    fn should_ignore_source(
        &self,
        metadata: &ClipboardCaptureMetadata,
        settings: &ClipboardSettings,
    ) -> bool {
        metadata.source_process.as_deref().is_some_and(|process| {
            let process = process.trim().to_ascii_lowercase();
            settings
                .ignored_apps
                .iter()
                .any(|ignored| ignored == &process)
        })
    }

    fn matches_sensitive_rule(&self, text: &str, settings: &ClipboardSettings) -> bool {
        settings.sensitive_rules.iter().any(|rule| {
            regex::Regex::new(rule)
                .map(|expression| expression.is_match(text))
                .unwrap_or(false)
        })
    }

    fn apply_settings_retention_and_capacity(&self) -> Result<u64, ClipboardError> {
        let settings = self.settings()?;
        let cutoff = settings.retention_days.map(|days| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |duration| {
                    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
                });
            now.saturating_sub(u64::from(days) * 24 * 60 * 60 * 1_000)
        });
        let _lifecycle = self.lock_image_lifecycle()?;
        let (removed, image_references) = self.storage.transaction(|transaction| {
            let mut removed = 0_u64;
            let mut image_references = Vec::new();
            if let Some(cutoff) = cutoff {
                let cutoff = i64::try_from(cutoff).map_err(|_| StorageError::LockPoisoned)?;
                let mut statement = transaction.prepare(
                    "SELECT id, file_path FROM clipboard_history
                     WHERE is_favorite = 0 AND captured_at_ms < ?1",
                )?;
                let victims = statement
                    .query_map([cutoff], |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                drop(statement);
                for (id, reference) in victims {
                    removed = removed.saturating_add(u64::from(
                        transaction.execute("DELETE FROM clipboard_history WHERE id = ?1", [id])?
                            > 0,
                    ));
                    if let Some(reference) = reference {
                        image_references.push(reference);
                    }
                }
            }
            let capacity_removed = evict_excess(transaction, settings.max_items)?;
            removed =
                removed.saturating_add(u64::try_from(capacity_removed.len()).unwrap_or(u64::MAX));
            image_references.extend(capacity_removed);
            Ok((removed, image_references))
        })?;
        self.remove_image_references(image_references);
        Ok(removed)
    }

    #[cfg(test)]
    pub fn corrupt_history_row_observations(&self) -> u64 {
        self.corrupt_history_row_observations
            .load(Ordering::Relaxed)
    }

    pub fn set_favorite(
        &self,
        id: i64,
        is_favorite: bool,
    ) -> Result<ClipboardHistoryItem, ClipboardError> {
        validate_id(id)?;
        let changed = self.storage.transaction(|transaction| {
            Ok(transaction.execute(
                "UPDATE clipboard_history SET is_favorite = ?1 WHERE id = ?2",
                params![is_favorite, id],
            )?)
        })?;
        if changed == 0 {
            return Err(ClipboardError::NotFound);
        }
        self.item_by_id(id)?.ok_or(ClipboardError::NotFound)
    }

    pub fn update_text(
        &self,
        id: i64,
        text: String,
        expected_revision: u64,
    ) -> Result<ClipboardHistoryItem, ClipboardError> {
        validate_id(id)?;
        if text.is_empty() {
            return Err(ClipboardError::EmptyText);
        }
        if text.len() > MAX_TEXT_BYTES {
            return Err(ClipboardError::TextTooLarge);
        }
        validate_safe_integer(expected_revision)?;
        let byte_size = i64::try_from(text.len()).map_err(|_| ClipboardError::NumericRange)?;
        let content_hash = format!("{:x}", Sha256::digest(text.as_bytes()));

        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        enum EditOutcome {
            Updated,
            NotFound,
            Duplicate,
            RevisionConflict,
        }
        let outcome = self.storage.transaction(|transaction| {
            let existing = transaction
                .query_row(
                    "SELECT content_type, content_revision FROM clipboard_history WHERE id = ?1",
                    [id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
                )
                .optional()?;
            let Some((kind, revision)) = existing else {
                return Ok(EditOutcome::NotFound);
            };
            if kind != "text" {
                return Ok(EditOutcome::NotFound);
            }
            if revision != expected_revision {
                return Ok(EditOutcome::RevisionConflict);
            }
            let duplicate = transaction.query_row(
                "SELECT EXISTS(
                        SELECT 1 FROM clipboard_history
                        WHERE content_type = 'text' AND content_hash = ?1 AND id <> ?2
                     )",
                params![content_hash, id],
                |row| row.get::<_, bool>(0),
            )?;
            if duplicate {
                return Ok(EditOutcome::Duplicate);
            }
            let changed = transaction.execute(
                "UPDATE clipboard_history SET
                    text_content = ?1,
                    content_hash = ?2,
                    byte_size = ?3,
                    content_revision = content_revision + 1
                 WHERE id = ?4 AND content_revision = ?5",
                params![text, content_hash, byte_size, id, expected_revision],
            )?;
            Ok(if changed == 1 {
                EditOutcome::Updated
            } else {
                EditOutcome::RevisionConflict
            })
        })?;
        match outcome {
            EditOutcome::Updated => self.item_by_id(id)?.ok_or(ClipboardError::NotFound),
            EditOutcome::NotFound => Err(ClipboardError::NotFound),
            EditOutcome::Duplicate => Err(ClipboardError::DuplicateText),
            EditOutcome::RevisionConflict => Err(ClipboardError::RevisionConflict),
        }
    }

    pub fn delete(&self, id: i64) -> Result<bool, ClipboardError> {
        validate_id(id)?;
        let _lifecycle = self.lock_image_lifecycle()?;
        let (deleted, image) = self.storage.transaction(|transaction| {
            let image = transaction
                .query_row(
                    "SELECT file_path FROM clipboard_history WHERE id = ?1",
                    [id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten();
            let deleted =
                transaction.execute("DELETE FROM clipboard_history WHERE id = ?1", [id])?;
            Ok((deleted, image))
        })?;
        if let Some(reference) = image {
            self.cleanup_if_unreferenced(&reference);
        }
        Ok(deleted > 0)
    }

    pub fn clear_unfavorite(&self) -> Result<u64, ClipboardError> {
        let _lifecycle = self.lock_image_lifecycle()?;
        let (deleted, images) = self.storage.transaction(|transaction| {
            let mut statement = transaction.prepare(
                "SELECT file_path FROM clipboard_history WHERE is_favorite = 0 AND file_path IS NOT NULL",
            )?;
            let images = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            drop(statement);
            let deleted = transaction.execute("DELETE FROM clipboard_history WHERE is_favorite = 0", [])?;
            Ok((deleted, images))
        })?;
        self.remove_image_references(images);
        u64::try_from(deleted).map_err(|_| ClipboardError::NumericRange)
    }

    pub fn image_bytes(&self, id: i64) -> Result<Vec<u8>, ClipboardError> {
        validate_id(id)?;
        let _lifecycle = self.lock_image_lifecycle()?;
        let item = self.item_by_id(id)?.ok_or(ClipboardError::NotFound)?;
        if item.kind != ClipboardContentKind::Image {
            return Err(ClipboardError::NotFound);
        }
        self.images
            .read(
                item.file_path
                    .as_deref()
                    .ok_or(ClipboardError::CorruptRecord)?,
            )
            .map_err(Into::into)
    }

    pub fn source_icon_bytes(&self, id: i64) -> Result<Vec<u8>, ClipboardError> {
        validate_id(id)?;
        let item = self.item_by_id(id)?.ok_or(ClipboardError::NotFound)?;
        let reference = item
            .source_icon_path
            .as_deref()
            .ok_or(SourceIconError::Unavailable)?;
        self.source_icons.read(reference).map_err(Into::into)
    }

    pub(crate) fn cache_source_icon(&self, executable: &std::path::Path) -> Option<String> {
        self.source_icons
            .cache_executable(executable)
            .ok()
            .flatten()
    }

    pub(crate) fn attach_source_icon(
        &self,
        id: i64,
        reference: Option<&str>,
    ) -> Result<(), ClipboardError> {
        let Some(reference) = reference else {
            return Ok(());
        };
        // Validation is performed by a read through the controlled cache before a
        // reference can be persisted. No executable path reaches SQLite.
        let _ = self.source_icons.read(reference)?;
        self.storage.transaction(|transaction| {
            transaction.execute(
                "UPDATE clipboard_history SET source_icon_path = ?1 WHERE id = ?2",
                params![reference, id],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    pub fn content_for_write(&self, id: i64) -> Result<ClipboardWriteContent, ClipboardError> {
        validate_id(id)?;
        let _lifecycle = self.lock_image_lifecycle()?;
        let item = self.item_by_id(id)?.ok_or(ClipboardError::NotFound)?;
        match item.kind {
            ClipboardContentKind::Text => Ok(ClipboardWriteContent::Text(
                item.text_content.ok_or(ClipboardError::CorruptRecord)?,
            )),
            ClipboardContentKind::Image => {
                let image = self.images.decode_rgba(
                    item.file_path
                        .as_deref()
                        .ok_or(ClipboardError::CorruptRecord)?,
                )?;
                Ok(ClipboardWriteContent::Image {
                    width: image.width,
                    height: image.height,
                    rgba: image.rgba,
                })
            }
            ClipboardContentKind::Files => {
                let paths = item.file_paths.ok_or(ClipboardError::CorruptRecord)?;
                let validated = validate_file_paths(paths, true)?;
                Ok(ClipboardWriteContent::Files {
                    paths: validated.paths,
                })
            }
        }
    }

    /// Returns the newest persisted internal history item. F4 deliberately
    /// uses this instead of re-reading Windows' current clipboard so its input
    /// stays consistent with the toolbox history the user can see and manage.
    pub fn latest_content_for_write(&self) -> Result<ClipboardWriteContent, ClipboardError> {
        let page = self.history(ClipboardHistoryQuery {
            favorites_only: false,
            search: None,
            limit: 1,
        })?;
        let item = page
            .items
            .into_iter()
            .next()
            .ok_or(ClipboardError::NoLatestItem)?;
        self.content_for_write(item.id)
    }

    fn reconcile_images(&self) -> Result<(), ClipboardError> {
        let _lifecycle = self.lock_image_lifecycle()?;
        let references = self.storage.read(|connection| {
            let mut statement = connection.prepare(
                "SELECT file_path FROM clipboard_history WHERE content_type = 'image' AND file_path IS NOT NULL",
            )?;
            let references = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(references)
        })?;
        let result = self.images.reconcile(&references);
        if !result.missing_references.is_empty() {
            self.storage.transaction(|transaction| {
                let mut statement = transaction.prepare(
                    "DELETE FROM clipboard_history WHERE content_type = 'image' AND file_path = ?1",
                )?;
                for reference in &result.missing_references {
                    statement.execute([reference])?;
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    fn reconcile_source_icons(&self) -> Result<(), ClipboardError> {
        let references = self.storage.read(|connection| {
            let mut statement = connection.prepare(
                "SELECT DISTINCT source_icon_path FROM clipboard_history
                 WHERE source_icon_path IS NOT NULL",
            )?;
            let references = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(references)
        })?;
        let result = self.source_icons.reconcile(&references);
        if !result.invalid_references.is_empty() {
            self.storage.transaction(|transaction| {
                let mut statement = transaction.prepare(
                    "UPDATE clipboard_history SET source_icon_path = NULL
                     WHERE source_icon_path = ?1",
                )?;
                for reference in &result.invalid_references {
                    statement.execute([reference])?;
                }
                Ok(())
            })?;
            self.source_icons
                .remove_references(&result.invalid_references);
        }
        Ok(())
    }

    fn remove_image_references(&self, references: Vec<String>) {
        for reference in references {
            self.cleanup_if_unreferenced(&reference);
        }
    }

    fn cleanup_if_unreferenced(&self, reference: &str) {
        let referenced = self.storage.read(|connection| {
            Ok(connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM clipboard_history WHERE file_path = ?1)",
                [reference],
                |row| row.get::<_, bool>(0),
            )?)
        });
        if matches!(referenced, Ok(false)) {
            let _ = self.images.remove(reference);
        }
    }

    fn lock_image_lifecycle(&self) -> Result<MutexGuard<'_, ()>, ClipboardError> {
        self.image_lifecycle
            .lock()
            .map_err(|_| ClipboardError::LifecycleLockPoisoned)
    }

    fn item_by_id(&self, id: i64) -> Result<Option<ClipboardHistoryItem>, ClipboardError> {
        let raw = self.storage.read(|connection| {
            Ok(connection
                .query_row(
                    "SELECT
                        id,
                        content_type,
                        text_content,
                        file_path,
                        source_application,
                        source_process,
                        captured_at_ms,
                        byte_size,
                        is_favorite,
                        content_revision,
                        source_icon_path,
                        file_paths_json
                     FROM clipboard_history
                     WHERE id = ?1",
                    [id],
                    raw_row,
                )
                .optional()?)
        })?;
        raw.map(ClipboardHistoryItem::try_from).transpose()
    }
}

fn validate_metadata(metadata: &ClipboardCaptureMetadata) -> Result<(), ClipboardError> {
    validate_optional_chars(
        metadata.source_application.as_deref(),
        MAX_SOURCE_APPLICATION_CHARS,
        ClipboardError::SourceApplicationTooLong,
    )?;
    validate_optional_chars(
        metadata.source_process.as_deref(),
        MAX_SOURCE_PROCESS_CHARS,
        ClipboardError::SourceProcessTooLong,
    )?;
    validate_safe_integer(metadata.captured_at_ms)
}

fn evict_excess(
    transaction: &rusqlite::Transaction<'_>,
    capacity: u32,
) -> Result<Vec<String>, StorageError> {
    let count: i64 =
        transaction.query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
            row.get(0)
        })?;
    let excess = count.saturating_sub(i64::from(capacity));
    if excess <= 0 {
        return Ok(Vec::new());
    }
    let mut statement = transaction.prepare(
        "SELECT id, file_path FROM clipboard_history
         WHERE is_favorite = 0 ORDER BY captured_at_ms ASC, id ASC LIMIT ?1",
    )?;
    let victims = statement
        .query_map([excess], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for (id, _) in &victims {
        transaction.execute("DELETE FROM clipboard_history WHERE id = ?1", [id])?;
    }
    Ok(victims
        .into_iter()
        .filter_map(|(_, reference)| reference)
        .collect())
}

#[derive(Debug)]
struct RawClipboardHistoryItem {
    id: i64,
    content_type: String,
    text_content: Option<String>,
    file_path: Option<String>,
    source_application: Option<String>,
    source_process: Option<String>,
    captured_at_ms: i64,
    byte_size: i64,
    is_favorite: i64,
    content_revision: i64,
    source_icon_path: Option<String>,
    file_paths_json: Option<String>,
}

fn raw_row(row: &Row<'_>) -> rusqlite::Result<RawClipboardHistoryItem> {
    Ok(RawClipboardHistoryItem {
        id: row.get(0)?,
        content_type: row.get(1)?,
        text_content: row.get(2)?,
        file_path: row.get(3)?,
        source_application: row.get(4)?,
        source_process: row.get(5)?,
        captured_at_ms: row.get(6)?,
        byte_size: row.get(7)?,
        is_favorite: row.get(8)?,
        content_revision: row.get(9)?,
        source_icon_path: row.get(10)?,
        file_paths_json: row.get(11)?,
    })
}

impl TryFrom<RawClipboardHistoryItem> for ClipboardHistoryItem {
    type Error = ClipboardError;

    fn try_from(raw: RawClipboardHistoryItem) -> Result<Self, Self::Error> {
        validate_id(raw.id).map_err(|_| ClipboardError::CorruptRecord)?;
        let kind = ClipboardContentKind::parse(&raw.content_type)?;
        let file_paths = raw
            .file_paths_json
            .as_deref()
            .map(|json| {
                if json.len() > MAX_CLIPBOARD_FILE_PATHS_JSON_BYTES {
                    return Err(ClipboardError::CorruptRecord);
                }
                serde_json::from_str::<Vec<Vec<u16>>>(json)
                    .map_err(|_| ClipboardError::CorruptRecord)
            })
            .transpose()?;
        match kind {
            ClipboardContentKind::Text
                if raw.text_content.is_some()
                    && raw.file_path.is_none()
                    && file_paths.is_none() => {}
            ClipboardContentKind::Image
                if raw.text_content.is_none()
                    && raw.file_path.is_some()
                    && file_paths.is_none() => {}
            ClipboardContentKind::Files
                if raw.text_content.is_none()
                    && raw.file_path.is_none()
                    && file_paths.as_ref().is_some_and(|paths| !paths.is_empty()) => {}
            _ => return Err(ClipboardError::CorruptRecord),
        }
        let captured_at_ms =
            u64::try_from(raw.captured_at_ms).map_err(|_| ClipboardError::CorruptRecord)?;
        let byte_size = u64::try_from(raw.byte_size).map_err(|_| ClipboardError::CorruptRecord)?;
        validate_safe_integer(captured_at_ms).map_err(|_| ClipboardError::CorruptRecord)?;
        validate_safe_integer(byte_size).map_err(|_| ClipboardError::CorruptRecord)?;
        let is_favorite = match raw.is_favorite {
            0 => false,
            1 => true,
            _ => return Err(ClipboardError::CorruptRecord),
        };
        let revision =
            u64::try_from(raw.content_revision).map_err(|_| ClipboardError::CorruptRecord)?;
        if revision == 0 {
            return Err(ClipboardError::CorruptRecord);
        }
        Ok(Self {
            id: raw.id,
            kind,
            text_content: raw.text_content,
            file_path: raw.file_path,
            file_paths,
            source_application: raw.source_application,
            source_process: raw.source_process,
            captured_at_ms,
            byte_size,
            is_favorite,
            revision,
            source_icon_path: raw.source_icon_path,
        })
    }
}

struct ValidatedFilePaths {
    paths: Vec<Vec<u16>>,
    byte_size: u64,
}

fn validate_file_paths(
    paths: Vec<Vec<u16>>,
    unavailable: bool,
) -> Result<ValidatedFilePaths, ClipboardError> {
    let file_error = || {
        if unavailable {
            ClipboardError::FilesUnavailable
        } else {
            ClipboardError::InvalidFiles
        }
    };
    if paths.is_empty() || paths.len() > MAX_CLIPBOARD_FILES {
        return Err(file_error());
    }
    let mut byte_size = 0_u64;
    for units in &paths {
        if units.is_empty() || units.len() > MAX_CLIPBOARD_FILE_PATH_UNITS || units.contains(&0) {
            return Err(file_error());
        }
        let path = path_from_utf16(units).ok_or_else(&file_error)?;
        if !path.is_absolute() || !path.is_file() {
            return Err(file_error());
        }
        let size = path.metadata().map_err(|_| file_error())?.len();
        byte_size = byte_size.checked_add(size).ok_or_else(&file_error)?;
        validate_safe_integer(byte_size).map_err(|_| file_error())?;
    }
    Ok(ValidatedFilePaths { paths, byte_size })
}

#[cfg(windows)]
fn path_from_utf16(units: &[u16]) -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    Some(PathBuf::from(OsString::from_wide(units)))
}

#[cfg(not(windows))]
fn path_from_utf16(units: &[u16]) -> Option<PathBuf> {
    String::from_utf16(units).ok().map(PathBuf::from)
}

#[cfg(all(windows, test))]
pub(crate) fn path_to_utf16(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().collect()
}

#[cfg(all(not(windows), test))]
pub(crate) fn path_to_utf16(path: &Path) -> Vec<u16> {
    path.to_string_lossy().encode_utf16().collect()
}

pub(crate) fn file_names(paths: &[Vec<u16>]) -> Result<Vec<String>, ClipboardError> {
    if paths.is_empty() || paths.len() > MAX_CLIPBOARD_FILES {
        return Err(ClipboardError::CorruptRecord);
    }
    paths
        .iter()
        .map(|path| {
            let start = path
                .iter()
                .rposition(|unit| matches!(*unit, 47 | 92))
                .map_or(0, |index| index + 1);
            if start >= path.len() {
                return Err(ClipboardError::CorruptRecord);
            }
            let name = String::from_utf16_lossy(&path[start..]);
            (!name.is_empty())
                .then_some(name)
                .ok_or(ClipboardError::CorruptRecord)
        })
        .collect()
}

fn hash_file_paths(paths: &[Vec<u16>]) -> String {
    let mut digest = Sha256::new();
    for path in paths {
        digest.update((path.len() as u64).to_le_bytes());
        for unit in path {
            digest.update(unit.to_le_bytes());
        }
    }
    format!("{:x}", digest.finalize())
}

fn validate_safe_integer(value: u64) -> Result<(), ClipboardError> {
    if value <= JS_MAX_SAFE_INTEGER {
        Ok(())
    } else {
        Err(ClipboardError::NumericRange)
    }
}

fn validate_optional_chars(
    value: Option<&str>,
    maximum: usize,
    error: ClipboardError,
) -> Result<(), ClipboardError> {
    if value.is_some_and(|value| value.chars().count() > maximum) {
        Err(error)
    } else {
        Ok(())
    }
}

fn validate_id(id: i64) -> Result<(), ClipboardError> {
    if id > 0 {
        Ok(())
    } else {
        Err(ClipboardError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::params;
    use std::fs;
    use std::sync::{mpsc, Barrier};
    use std::time::Duration;
    use tempfile::tempdir;

    use super::*;

    fn service() -> (tempfile::TempDir, Arc<StorageService>, ClipboardService) {
        let temp = tempdir().unwrap();
        let storage = Arc::new(StorageService::initialize(temp.path()).unwrap());
        let service = ClipboardService::initialize(Arc::clone(&storage));
        (temp, storage, service)
    }

    fn metadata(timestamp: u64) -> ClipboardCaptureMetadata {
        ClipboardCaptureMetadata {
            captured_at_ms: timestamp,
            source_application: None,
            source_process: None,
        }
    }

    fn all(limit: u32) -> ClipboardHistoryQuery {
        ClipboardHistoryQuery {
            favorites_only: false,
            search: None,
            limit,
        }
    }

    struct RawTextInsert<'a> {
        id: i64,
        text: &'a str,
        hash: &'a str,
        source_application: Option<&'a str>,
        source_process: Option<&'a str>,
        captured_at_ms: i64,
        byte_size: i64,
    }

    fn valid_raw_text<'a>(text: &'a str, hash: &'a str) -> RawTextInsert<'a> {
        RawTextInsert {
            id: 1,
            text,
            hash,
            source_application: None,
            source_process: None,
            captured_at_ms: 1,
            byte_size: text.len() as i64,
        }
    }

    fn insert_raw_text(
        storage: &StorageService,
        input: RawTextInsert<'_>,
    ) -> Result<(), StorageError> {
        storage.transaction(|transaction| {
            transaction.execute(
                "INSERT INTO clipboard_history (
                    id,
                    content_type,
                    text_content,
                    file_path,
                    content_hash,
                    source_application,
                    source_process,
                    captured_at_ms,
                    byte_size,
                    is_favorite
                 ) VALUES (?1, 'text', ?2, NULL, ?3, ?4, ?5, ?6, ?7, 0)",
                params![
                    input.id,
                    input.text,
                    input.hash,
                    input.source_application,
                    input.source_process,
                    input.captured_at_ms,
                    input.byte_size
                ],
            )?;
            Ok(())
        })
    }

    #[test]
    fn text_is_persisted_with_real_metadata_and_restored_after_restart() {
        let temp = tempdir().unwrap();
        let data_root = temp.path().join("app-data");
        let storage = Arc::new(StorageService::initialize(&data_root).unwrap());
        let service = ClipboardService::initialize(storage);

        let recorded = service
            .record_text(
                "真实文本".to_owned(),
                ClipboardCaptureMetadata {
                    captured_at_ms: 1_720_000_000_123,
                    source_application: Some("Editor".to_owned()),
                    source_process: Some("editor.exe".to_owned()),
                },
            )
            .unwrap();
        assert!(recorded.retained);
        drop(service);

        let reopened_storage = Arc::new(StorageService::initialize(&data_root).unwrap());
        let reopened = ClipboardService::initialize(reopened_storage);
        let page = reopened.history(all(100)).unwrap();

        assert_eq!(page.total_count, 1);
        let item = &page.items[0];
        assert_eq!(item.kind, ClipboardContentKind::Text);
        assert_eq!(item.text_content.as_deref(), Some("真实文本"));
        assert_eq!(item.source_application.as_deref(), Some("Editor"));
        assert_eq!(item.source_process.as_deref(), Some("editor.exe"));
        assert_eq!(item.captured_at_ms, 1_720_000_000_123);
        assert_eq!(item.byte_size, "真实文本".len() as u64);
    }

    #[test]
    fn duplicate_text_moves_to_latest_metadata_and_preserves_favorite() {
        let (_temp, _storage, service) = service();
        let first = service
            .record_text("same".to_owned(), metadata(10))
            .unwrap()
            .item
            .unwrap();
        service.set_favorite(first.id, true).unwrap();

        let duplicate = service
            .record_text(
                "same".to_owned(),
                ClipboardCaptureMetadata {
                    captured_at_ms: 30,
                    source_application: Some("Latest".to_owned()),
                    source_process: None,
                },
            )
            .unwrap()
            .item
            .unwrap();

        assert_eq!(duplicate.id, first.id);
        assert!(duplicate.is_favorite);
        assert_eq!(duplicate.captured_at_ms, 30);
        assert_eq!(duplicate.source_application.as_deref(), Some("Latest"));
        assert_eq!(service.history(all(100)).unwrap().total_count, 1);
    }

    #[test]
    fn persisted_settings_apply_capacity_ignored_apps_reuse_and_sensitive_rules() {
        let (_temp, _storage, service) = service();
        let settings = ClipboardSettings {
            retention_days: Some(7),
            max_items: 10,
            ignored_apps: vec!["notepad.exe".to_owned()],
            history_reuse_strategy: ClipboardHistoryReuseStrategy::Keep,
            sensitive_rules: vec!["(?i)password".to_owned()],
        };
        assert_eq!(service.update_settings(settings.clone()).unwrap(), 0);
        assert_eq!(service.settings().unwrap(), settings);

        let ignored = service
            .record_text(
                "from ignored app".to_owned(),
                ClipboardCaptureMetadata {
                    captured_at_ms: 1,
                    source_application: Some("Notepad".to_owned()),
                    source_process: Some("NOTEPAD.EXE".to_owned()),
                },
            )
            .unwrap();
        assert!(!ignored.retained);
        assert!(service
            .record_text("my password".to_owned(), metadata(2))
            .unwrap()
            .item
            .is_none());

        let first = service.record_text("once".to_owned(), metadata(3)).unwrap();
        assert!(first.retained);
        let repeated = service.record_text("once".to_owned(), metadata(4)).unwrap();
        assert!(repeated.retained);
        assert_eq!(repeated.item.unwrap().id, first.item.unwrap().id);
        for index in 0_u64..10 {
            service
                .record_text(format!("item-{index}"), metadata(index + 10))
                .unwrap();
        }
        assert_eq!(service.history(all(100)).unwrap().items.len(), 10);
    }

    #[test]
    fn reusing_an_existing_item_promotes_it_to_the_history_front() {
        let (_temp, _storage, service) = service();
        let old = service
            .record_text("old".to_owned(), metadata(1))
            .unwrap()
            .item
            .unwrap();
        let newest = service
            .record_text("newest".to_owned(), metadata(2))
            .unwrap()
            .item
            .unwrap();

        service.apply_history_reuse(old.id).unwrap();

        let history = service.history(all(100)).unwrap();
        assert_eq!(history.items[0].id, old.id);
        assert_eq!(history.items[1].id, newest.id);
        assert!(history.items[0].captured_at_ms > newest.captured_at_ms);
    }

    #[test]
    fn keeping_reused_history_items_preserves_their_existing_order() {
        let (_temp, _storage, service) = service();
        service
            .update_settings(ClipboardSettings {
                history_reuse_strategy: ClipboardHistoryReuseStrategy::Keep,
                ..ClipboardSettings::default()
            })
            .unwrap();
        let old = service
            .record_text("old".to_owned(), metadata(1))
            .unwrap()
            .item
            .unwrap();
        let newest = service
            .record_text("newest".to_owned(), metadata(2))
            .unwrap()
            .item
            .unwrap();

        service.apply_history_reuse(old.id).unwrap();

        let history = service.history(all(100)).unwrap();
        assert_eq!(history.items[0].id, newest.id);
        assert_eq!(history.items[1].id, old.id);
    }

    #[test]
    fn text_edit_is_revision_guarded_and_preserves_metadata_favorite_and_timestamp() {
        let (_temp, _storage, service) = service();
        let id = service
            .record_text("before".to_owned(), metadata(77))
            .unwrap()
            .item
            .unwrap()
            .id;
        service.set_favorite(id, true).unwrap();

        let updated = service.update_text(id, "after".to_owned(), 1).unwrap();

        assert_eq!(updated.text_content.as_deref(), Some("after"));
        assert_eq!(updated.byte_size, 5);
        assert_eq!(updated.captured_at_ms, 77);
        assert!(updated.is_favorite);
        assert_eq!(updated.revision, 2);
        assert!(matches!(
            service.update_text(id, "stale".to_owned(), 1),
            Err(ClipboardError::RevisionConflict)
        ));
        assert_eq!(
            service.history(all(100)).unwrap().items[0]
                .text_content
                .as_deref(),
            Some("after")
        );
    }

    #[test]
    fn text_edit_rejects_empty_oversized_image_and_duplicate_without_mutation() {
        let (_temp, _storage, service) = service();
        let first = service
            .record_text("first".to_owned(), metadata(1))
            .unwrap()
            .item
            .unwrap();
        let second = service
            .record_text("second".to_owned(), metadata(2))
            .unwrap()
            .item
            .unwrap();
        let image = service
            .record_image(1, 1, vec![1, 2, 3, 255], metadata(3))
            .unwrap()
            .item
            .unwrap();

        assert!(matches!(
            service.update_text(first.id, String::new(), 1),
            Err(ClipboardError::EmptyText)
        ));
        assert!(matches!(
            service.update_text(first.id, "x".repeat(MAX_TEXT_BYTES + 1), 1),
            Err(ClipboardError::TextTooLarge)
        ));
        assert!(matches!(
            service.update_text(first.id, "second".to_owned(), 1),
            Err(ClipboardError::DuplicateText)
        ));
        assert!(matches!(
            service.update_text(image.id, "text".to_owned(), 1),
            Err(ClipboardError::NotFound)
        ));
        let items = service.history(all(100)).unwrap().items;
        assert_eq!(
            items
                .iter()
                .find(|item| item.id == first.id)
                .unwrap()
                .text_content
                .as_deref(),
            Some("first")
        );
        assert_eq!(
            items
                .iter()
                .find(|item| item.id == second.id)
                .unwrap()
                .revision,
            1
        );
    }

    #[test]
    fn capacity_evicts_oldest_nonfavorite_and_never_evicts_favorites() {
        let (_temp, _storage, service) = service();
        for index in 0..CLIPBOARD_HISTORY_CAPACITY {
            let item = service
                .record_text(format!("item-{index}"), metadata(u64::from(index)))
                .unwrap()
                .item
                .unwrap();
            if index == 0 {
                service.set_favorite(item.id, true).unwrap();
            }
        }

        let newest = service
            .record_text("newest".to_owned(), metadata(500))
            .unwrap();
        assert!(newest.retained);
        let page = service.history(all(100)).unwrap();
        assert_eq!(page.total_count, 100);
        assert!(page
            .items
            .iter()
            .any(|item| item.text_content.as_deref() == Some("item-0") && item.is_favorite));
        assert!(!page
            .items
            .iter()
            .any(|item| item.text_content.as_deref() == Some("item-1")));
        assert_eq!(page.items[0].text_content.as_deref(), Some("newest"));
    }

    #[test]
    fn a_new_nonfavorite_reports_not_retained_when_favorites_fill_capacity() {
        let (_temp, _storage, service) = service();
        for index in 0..CLIPBOARD_HISTORY_CAPACITY {
            let item = service
                .record_text(format!("favorite-{index}"), metadata(u64::from(index)))
                .unwrap()
                .item
                .unwrap();
            service.set_favorite(item.id, true).unwrap();
        }

        let result = service
            .record_text("cannot-fit".to_owned(), metadata(1_000))
            .unwrap();

        assert!(!result.retained);
        assert_eq!(result.item, None);
        let page = service.history(all(100)).unwrap();
        assert_eq!(page.total_count, 100);
        assert!(page.items.iter().all(|item| item.is_favorite));
    }

    #[test]
    fn query_search_scope_limit_and_total_count_report_filtered_truth() {
        let (_temp, _storage, service) = service();
        let alpha = service
            .record_text(
                "Alpha note".to_owned(),
                ClipboardCaptureMetadata {
                    captured_at_ms: 10,
                    source_application: Some("Notes".to_owned()),
                    source_process: None,
                },
            )
            .unwrap()
            .item
            .unwrap();
        service.set_favorite(alpha.id, true).unwrap();
        service
            .record_text("Beta note".to_owned(), metadata(20))
            .unwrap();
        service
            .record_text(
                "Gamma".to_owned(),
                ClipboardCaptureMetadata {
                    captured_at_ms: 30,
                    source_application: None,
                    source_process: Some("NOTES.EXE".to_owned()),
                },
            )
            .unwrap();

        let searched = service
            .history(ClipboardHistoryQuery {
                favorites_only: false,
                search: Some("note".to_owned()),
                limit: 1,
            })
            .unwrap();
        assert_eq!(searched.items.len(), 1);
        assert_eq!(searched.total_count, 3);

        let favorites = service
            .history(ClipboardHistoryQuery {
                favorites_only: true,
                search: None,
                limit: 100,
            })
            .unwrap();
        assert_eq!(favorites.total_count, 1);
        assert_eq!(favorites.items[0].id, alpha.id);
    }

    #[test]
    fn history_isolates_corrupt_file_json_but_propagates_database_failures() {
        let (temp, storage, service) = service();
        let valid = service
            .record_text("still visible".to_owned(), metadata(10))
            .unwrap()
            .item
            .unwrap();
        let source = temp.path().join("isolated.txt");
        fs::write(&source, b"isolated").unwrap();
        let corrupt = service
            .record_files(vec![path_to_utf16(&source)], metadata(20))
            .unwrap()
            .item
            .unwrap();
        storage
            .transaction(|transaction| {
                transaction.execute(
                    "UPDATE clipboard_history SET file_paths_json = ?1 WHERE id = ?2",
                    params!["{malformed", corrupt.id],
                )?;
                Ok(())
            })
            .unwrap();

        let page = service.history(all(100)).unwrap();
        assert_eq!(page.total_count, 1);
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, valid.id);
        assert_eq!(service.corrupt_history_row_observations(), 1);

        storage
            .transaction(|transaction| {
                transaction.execute_batch(
                    "ALTER TABLE clipboard_history RENAME TO clipboard_history_unavailable;",
                )?;
                Ok(())
            })
            .unwrap();
        assert!(matches!(
            service.history(all(100)),
            Err(ClipboardError::Storage(StorageError::Sql(_)))
        ));
        assert_eq!(service.corrupt_history_row_observations(), 1);
    }

    #[test]
    fn favorite_delete_and_clear_mutations_return_persisted_results() {
        let (_temp, _storage, service) = service();
        let kept = service
            .record_text("kept".to_owned(), metadata(10))
            .unwrap()
            .item
            .unwrap();
        let deleted = service
            .record_text("deleted".to_owned(), metadata(20))
            .unwrap()
            .item
            .unwrap();
        service.set_favorite(kept.id, true).unwrap();

        assert!(service.delete(deleted.id).unwrap());
        assert!(!service.delete(deleted.id).unwrap());
        service
            .record_text("cleared".to_owned(), metadata(30))
            .unwrap();
        assert_eq!(service.clear_unfavorite().unwrap(), 1);

        let page = service.history(all(100)).unwrap();
        assert_eq!(page.total_count, 1);
        assert_eq!(page.items[0].id, kept.id);
        assert!(page.items[0].is_favorite);
    }

    fn image_path(temp: &tempfile::TempDir, item: &ClipboardHistoryItem) -> std::path::PathBuf {
        temp.path()
            .join(item.file_path.as_deref().unwrap().replace('/', "\\"))
    }

    #[test]
    fn image_persists_as_png_across_restart_and_duplicate_preserves_favorite() {
        let temp = tempdir().unwrap();
        let data_root = temp.path().join("app-data");
        let storage = Arc::new(StorageService::initialize(&data_root).unwrap());
        let service = ClipboardService::initialize(storage);
        let first = service
            .record_image(1, 1, vec![1, 2, 3, 255], metadata(10))
            .unwrap()
            .item
            .unwrap();
        assert_eq!(first.kind, ClipboardContentKind::Image);
        assert_eq!(first.text_content, None);
        assert!(service
            .image_bytes(first.id)
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n"));
        service.set_favorite(first.id, true).unwrap();
        let duplicate = service
            .record_image(1, 1, vec![1, 2, 3, 255], metadata(20))
            .unwrap()
            .item
            .unwrap();
        assert_eq!(duplicate.id, first.id);
        assert!(duplicate.is_favorite);
        assert_eq!(duplicate.captured_at_ms, 20);
        drop(service);

        let reopened =
            ClipboardService::initialize(Arc::new(StorageService::initialize(&data_root).unwrap()));
        let item = reopened.history(all(100)).unwrap().items.pop().unwrap();
        assert_eq!(item.kind, ClipboardContentKind::Image);
        assert!(reopened
            .image_bytes(item.id)
            .unwrap()
            .starts_with(b"\x89PNG"));
    }

    #[test]
    fn image_files_are_removed_by_delete_clear_and_capacity_eviction() {
        let (temp, _storage, service) = service();
        let deleted = service
            .record_image(1, 1, vec![1, 0, 0, 255], metadata(1))
            .unwrap()
            .item
            .unwrap();
        let deleted_path = image_path(&temp, &deleted);
        assert!(deleted_path.is_file());
        assert!(service.delete(deleted.id).unwrap());
        assert!(!deleted_path.exists());

        let cleared = service
            .record_image(1, 1, vec![2, 0, 0, 255], metadata(2))
            .unwrap()
            .item
            .unwrap();
        let cleared_path = image_path(&temp, &cleared);
        assert_eq!(service.clear_unfavorite().unwrap(), 1);
        assert!(!cleared_path.exists());

        let evicted = service
            .record_image(1, 1, vec![3, 0, 0, 255], metadata(0))
            .unwrap()
            .item
            .unwrap();
        let evicted_path = image_path(&temp, &evicted);
        for index in 0..CLIPBOARD_HISTORY_CAPACITY {
            service
                .record_text(format!("capacity-{index}"), metadata(u64::from(index) + 1))
                .unwrap();
        }
        assert!(!evicted_path.exists());
        assert_eq!(service.history(all(100)).unwrap().total_count, 100);
    }

    #[test]
    fn startup_reconcile_removes_orphans_temp_files_and_missing_image_rows() {
        let temp = tempdir().unwrap();
        let data_root = temp.path().join("app-data");
        let storage = Arc::new(StorageService::initialize(&data_root).unwrap());
        let service = ClipboardService::initialize(storage);
        let item = service
            .record_image(1, 1, vec![9, 8, 7, 255], metadata(1))
            .unwrap()
            .item
            .unwrap();
        let corrupt_item = service
            .record_image(1, 1, vec![6, 5, 4, 255], metadata(2))
            .unwrap()
            .item
            .unwrap();
        let image_path = data_root.join(item.file_path.as_deref().unwrap().replace('/', "\\"));
        let corrupt_path = data_root.join(
            corrupt_item
                .file_path
                .as_deref()
                .unwrap()
                .replace('/', "\\"),
        );
        let directory = image_path.parent().unwrap().to_path_buf();
        drop(service);
        fs::remove_file(&image_path).unwrap();
        fs::write(&corrupt_path, b"corrupt png").unwrap();
        let orphan = directory.join(format!("{}.png", "a".repeat(64)));
        let temporary = directory.join("stale.tmp");
        let unremovable_orphan = directory.join("unexpected-directory");
        fs::write(&orphan, b"orphan").unwrap();
        fs::write(&temporary, b"temporary").unwrap();
        fs::create_dir(&unremovable_orphan).unwrap();

        let reopened =
            ClipboardService::initialize(Arc::new(StorageService::initialize(&data_root).unwrap()));
        assert_eq!(reopened.history(all(100)).unwrap().total_count, 0);
        assert!(!orphan.exists());
        assert!(!temporary.exists());
        assert!(!corrupt_path.exists());
        assert!(unremovable_orphan.is_dir());
    }

    #[test]
    fn startup_reconcile_clears_legacy_low_resolution_source_icon_references() {
        let temp = tempdir().unwrap();
        let data_root = temp.path().join("app-data");
        let storage = Arc::new(StorageService::initialize(&data_root).unwrap());
        let service = ClipboardService::initialize(Arc::clone(&storage));
        let item = service
            .record_text("legacy icon".to_owned(), metadata(1))
            .unwrap()
            .item
            .unwrap();

        let mut legacy_png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut legacy_png, 32, 32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&vec![0; 32 * 32 * 4]).unwrap();
        }
        let hash = format!("{:x}", Sha256::digest(&legacy_png));
        let reference = format!("files/source-icons/{hash}.png");
        let icon_path = data_root
            .join("files")
            .join("source-icons")
            .join(format!("{hash}.png"));
        fs::write(&icon_path, legacy_png).unwrap();
        storage
            .transaction(|transaction| {
                transaction.execute(
                    "UPDATE clipboard_history SET source_icon_path = ?1 WHERE id = ?2",
                    params![reference, item.id],
                )?;
                Ok(())
            })
            .unwrap();
        assert!(matches!(
            service.source_icon_bytes(item.id),
            Err(ClipboardError::SourceIcon(SourceIconError::Unavailable))
        ));
        drop(service);
        drop(storage);

        let reopened =
            ClipboardService::initialize(Arc::new(StorageService::initialize(&data_root).unwrap()));
        let reopened_item = reopened.history(all(100)).unwrap().items.pop().unwrap();
        assert_eq!(reopened_item.id, item.id);
        assert_eq!(reopened_item.source_icon_path, None);
        assert!(!icon_path.exists());
    }

    #[test]
    fn image_write_and_database_failures_leave_no_new_managed_file() {
        let (temp, storage, service) = service();
        let directory = temp.path().join("files\\clipboard\\images");
        fs::remove_dir_all(&directory).unwrap();
        fs::write(&directory, b"blocks directory").unwrap();
        assert!(matches!(
            service.record_image(1, 1, vec![1, 2, 3, 255], metadata(1)),
            Err(ClipboardError::Image(_))
        ));
        fs::remove_file(&directory).unwrap();
        fs::create_dir_all(&directory).unwrap();
        storage.transaction(|transaction| {
            transaction.execute_batch("CREATE TRIGGER reject_images BEFORE INSERT ON clipboard_history WHEN NEW.content_type = 'image' BEGIN SELECT RAISE(FAIL, 'reject'); END;")?;
            Ok(())
        }).unwrap();
        assert!(matches!(
            service.record_image(1, 1, vec![4, 5, 6, 255], metadata(2)),
            Err(ClipboardError::Storage(_))
        ));
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
    }

    #[test]
    fn committed_delete_keeps_database_success_truth_when_file_cleanup_fails() {
        let (temp, _storage, service) = service();
        let item = service
            .record_image(1, 1, vec![8, 8, 8, 255], metadata(1))
            .unwrap()
            .item
            .unwrap();
        let path = image_path(&temp, &item);
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(service.delete(item.id).unwrap());
        assert_eq!(service.history(all(100)).unwrap().total_count, 0);
        assert!(path.is_dir());
    }

    #[test]
    fn image_lifecycle_lock_serializes_same_hash_failure_reuse_and_mixed_mutations() {
        let temp = tempdir().unwrap();
        let storage = Arc::new(StorageService::initialize(temp.path()).unwrap());
        storage
            .transaction(|transaction| {
                transaction.execute_batch(
                    "CREATE TRIGGER reject_selected_images
                     BEFORE INSERT ON clipboard_history
                     WHEN NEW.content_type = 'image' AND NEW.source_application = 'Fail'
                     BEGIN SELECT RAISE(FAIL, 'selected failure'); END;",
                )?;
                Ok(())
            })
            .unwrap();
        let service = Arc::new(ClipboardService::initialize(storage));
        let (entered_sender, entered_receiver) = mpsc::sync_channel(1);
        let (proceed_sender, proceed_receiver) = mpsc::sync_channel(1);
        let proceed_receiver = Arc::new(Mutex::new(proceed_receiver));
        *service.after_image_store_hook.lock().unwrap() = Some(Arc::new({
            let proceed_receiver = Arc::clone(&proceed_receiver);
            move |metadata| {
                if metadata.source_application.as_deref() == Some("Fail") {
                    entered_sender.send(()).unwrap();
                    proceed_receiver.lock().unwrap().recv().unwrap();
                }
            }
        }));

        let failing_service = Arc::clone(&service);
        let failing = std::thread::spawn(move || {
            failing_service.record_image(
                1,
                1,
                vec![1, 2, 3, 255],
                ClipboardCaptureMetadata {
                    captured_at_ms: 1,
                    source_application: Some("Fail".to_owned()),
                    source_process: None,
                },
            )
        });
        entered_receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap();

        let (success_done_sender, success_done_receiver) = mpsc::sync_channel(1);
        let (success_started_sender, success_started_receiver) = mpsc::sync_channel(1);
        let successful_service = Arc::clone(&service);
        let successful = std::thread::spawn(move || {
            success_started_sender.send(()).unwrap();
            let result = successful_service.record_image(
                1,
                1,
                vec![1, 2, 3, 255],
                ClipboardCaptureMetadata {
                    captured_at_ms: 2,
                    source_application: Some("Success".to_owned()),
                    source_process: None,
                },
            );
            success_done_sender.send(()).unwrap();
            result
        });
        success_started_receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert!(success_done_receiver
            .recv_timeout(Duration::from_millis(100))
            .is_err());
        proceed_sender.send(()).unwrap();
        assert!(matches!(
            failing.join().unwrap(),
            Err(ClipboardError::Storage(_))
        ));
        assert!(successful.join().unwrap().unwrap().retained);
        *service.after_image_store_hook.lock().unwrap() = None;

        let existing = service
            .record_image(1, 1, vec![8, 8, 8, 255], metadata(3))
            .unwrap()
            .item
            .unwrap();
        let barrier = Arc::new(Barrier::new(4));
        let success_service = Arc::clone(&service);
        let success_barrier = Arc::clone(&barrier);
        let record = std::thread::spawn(move || {
            success_barrier.wait();
            success_service.record_image(1, 1, vec![4, 5, 6, 255], metadata(4))
        });
        let fail_service = Arc::clone(&service);
        let fail_barrier = Arc::clone(&barrier);
        let fail = std::thread::spawn(move || {
            fail_barrier.wait();
            fail_service.record_image(
                1,
                1,
                vec![4, 5, 6, 255],
                ClipboardCaptureMetadata {
                    captured_at_ms: 5,
                    source_application: Some("Fail".to_owned()),
                    source_process: None,
                },
            )
        });
        let delete_service = Arc::clone(&service);
        let delete_barrier = Arc::clone(&barrier);
        let delete = std::thread::spawn(move || {
            delete_barrier.wait();
            delete_service.delete(existing.id)
        });
        barrier.wait();
        assert!(record.join().unwrap().unwrap().retained);
        assert!(matches!(
            fail.join().unwrap(),
            Err(ClipboardError::Storage(_))
        ));
        assert!(delete.join().unwrap().unwrap());

        let page = service.history(all(100)).unwrap();
        for item in page
            .items
            .iter()
            .filter(|item| item.kind == ClipboardContentKind::Image)
        {
            assert!(service
                .image_bytes(item.id)
                .unwrap()
                .starts_with(b"\x89PNG"));
        }
    }

    #[test]
    fn file_drop_persists_multiple_paths_and_rebuilds_only_while_files_exist() {
        let (temp, storage, clipboard) = service();
        let text_file = temp.path().join("notes.txt");
        let image_file = temp.path().join("preview.png");
        fs::write(&text_file, b"hello").unwrap();
        fs::write(&image_file, [1_u8, 2, 3]).unwrap();
        let paths = vec![path_to_utf16(&text_file), path_to_utf16(&image_file)];

        let first = clipboard
            .record_files(paths.clone(), metadata(100))
            .unwrap()
            .item
            .unwrap();
        assert_eq!(first.kind, ClipboardContentKind::Files);
        assert_eq!(first.text_content, None);
        assert_eq!(first.file_path, None);
        assert_eq!(first.file_paths.as_ref(), Some(&paths));
        assert_eq!(first.byte_size, 8);
        assert_eq!(
            file_names(first.file_paths.as_deref().unwrap()).unwrap(),
            vec!["notes.txt", "preview.png"]
        );

        let duplicate = clipboard
            .record_files(paths.clone(), metadata(200))
            .unwrap()
            .item
            .unwrap();
        assert_eq!(duplicate.id, first.id);
        assert_eq!(duplicate.revision, first.revision + 1);
        drop(clipboard);
        let reopened = ClipboardService::initialize(storage);
        assert_eq!(
            reopened.history(all(100)).unwrap().items[0]
                .file_paths
                .as_ref(),
            Some(&paths)
        );
        assert!(matches!(
            reopened.content_for_write(first.id).unwrap(),
            ClipboardWriteContent::Files { paths: rebuilt } if rebuilt == paths
        ));

        fs::remove_file(&text_file).unwrap();
        assert!(matches!(
            reopened.content_for_write(first.id),
            Err(ClipboardError::FilesUnavailable)
        ));
        assert!(reopened.delete(first.id).unwrap());
        assert!(reopened.history(all(100)).unwrap().items.is_empty());
    }

    #[test]
    fn file_drop_rejects_relative_missing_empty_and_excessive_lists_without_persistence() {
        let (temp, _storage, clipboard) = service();
        let existing = temp.path().join("one.txt");
        fs::write(&existing, b"one").unwrap();
        for paths in [
            Vec::<Vec<u16>>::new(),
            vec!["relative.txt".encode_utf16().collect()],
            vec![path_to_utf16(&temp.path().join("missing.txt"))],
            vec![Vec::new()],
        ] {
            assert!(matches!(
                clipboard.record_files(paths, metadata(1)),
                Err(ClipboardError::InvalidFiles)
            ));
        }
        let excessive = vec![path_to_utf16(&existing); MAX_CLIPBOARD_FILES + 1];
        assert!(matches!(
            clipboard.record_files(excessive, metadata(1)),
            Err(ClipboardError::InvalidFiles)
        ));
        assert_eq!(clipboard.history(all(100)).unwrap().total_count, 0);
    }

    #[test]
    fn invalid_inputs_are_rejected_before_storage_changes() {
        let (_temp, _storage, service) = service();

        assert!(matches!(
            service.record_text(String::new(), metadata(1)),
            Err(ClipboardError::EmptyText)
        ));
        assert!(matches!(
            service.record_text("value".to_owned(), metadata(JS_MAX_SAFE_INTEGER + 1)),
            Err(ClipboardError::NumericRange)
        ));
        for limit in [0, 101] {
            assert!(matches!(
                service.history(all(limit)),
                Err(ClipboardError::InvalidLimit)
            ));
        }
        assert_eq!(service.history(all(100)).unwrap().total_count, 0);
    }

    #[test]
    fn exact_text_source_and_search_boundaries_use_utf8_bytes_and_unicode_scalar_chars() {
        let (_temp, _storage, service) = service();
        let text = "a".repeat(MAX_TEXT_BYTES);
        let source_application = "界".repeat(MAX_SOURCE_APPLICATION_CHARS);
        let source_process = "🙂".repeat(MAX_SOURCE_PROCESS_CHARS);

        let recorded = service
            .record_text(
                text,
                ClipboardCaptureMetadata {
                    captured_at_ms: JS_MAX_SAFE_INTEGER,
                    source_application: Some(source_application.clone()),
                    source_process: Some(source_process),
                },
            )
            .unwrap()
            .item
            .unwrap();

        assert_eq!(recorded.byte_size, MAX_TEXT_BYTES as u64);
        assert_eq!(
            recorded
                .source_application
                .as_deref()
                .unwrap()
                .chars()
                .count(),
            MAX_SOURCE_APPLICATION_CHARS
        );
        let result = service
            .history(ClipboardHistoryQuery {
                favorites_only: false,
                search: Some("界".repeat(MAX_SEARCH_CHARS)),
                limit: 100,
            })
            .unwrap();
        assert_eq!(result.total_count, 1);
    }

    #[test]
    fn oversized_text_sources_and_search_are_rejected_before_sql() {
        let (_temp, _storage, service) = service();

        assert!(matches!(
            service.record_text("a".repeat(MAX_TEXT_BYTES + 1), metadata(1)),
            Err(ClipboardError::TextTooLarge)
        ));
        assert!(matches!(
            service.record_text(
                "value".to_owned(),
                ClipboardCaptureMetadata {
                    captured_at_ms: 1,
                    source_application: Some("界".repeat(MAX_SOURCE_APPLICATION_CHARS + 1)),
                    source_process: None,
                }
            ),
            Err(ClipboardError::SourceApplicationTooLong)
        ));
        assert!(matches!(
            service.record_text(
                "value".to_owned(),
                ClipboardCaptureMetadata {
                    captured_at_ms: 1,
                    source_application: None,
                    source_process: Some("🙂".repeat(MAX_SOURCE_PROCESS_CHARS + 1)),
                }
            ),
            Err(ClipboardError::SourceProcessTooLong)
        ));
        assert!(matches!(
            service.history(ClipboardHistoryQuery {
                favorites_only: false,
                search: Some("界".repeat(MAX_SEARCH_CHARS + 1)),
                limit: 100,
            }),
            Err(ClipboardError::SearchTooLong)
        ));
        assert_eq!(service.history(all(100)).unwrap().total_count, 0);
    }

    #[test]
    fn schema_checks_reject_invalid_direct_inserts() {
        let (_temp, storage, _service) = service();
        let valid_hash = "a".repeat(64);
        let invalid_hash = "A".repeat(64);
        let oversized_text = "a".repeat(MAX_TEXT_BYTES + 1);
        let oversized_application = "界".repeat(MAX_SOURCE_APPLICATION_CHARS + 1);
        let oversized_process = "🙂".repeat(MAX_SOURCE_PROCESS_CHARS + 1);
        let invalid_attempts = [
            insert_raw_text(
                &storage,
                RawTextInsert {
                    hash: &invalid_hash,
                    ..valid_raw_text("value", &invalid_hash)
                },
            ),
            insert_raw_text(
                &storage,
                RawTextInsert {
                    byte_size: 4,
                    ..valid_raw_text("value", &valid_hash)
                },
            ),
            insert_raw_text(
                &storage,
                RawTextInsert {
                    source_application: Some(&oversized_application),
                    ..valid_raw_text("value", &valid_hash)
                },
            ),
            insert_raw_text(
                &storage,
                RawTextInsert {
                    source_process: Some(&oversized_process),
                    ..valid_raw_text("value", &valid_hash)
                },
            ),
            insert_raw_text(
                &storage,
                RawTextInsert {
                    captured_at_ms: (JS_MAX_SAFE_INTEGER + 1) as i64,
                    ..valid_raw_text("value", &valid_hash)
                },
            ),
            insert_raw_text(&storage, valid_raw_text(&oversized_text, &valid_hash)),
            insert_raw_text(
                &storage,
                RawTextInsert {
                    id: -1,
                    ..valid_raw_text("value", &valid_hash)
                },
            ),
        ];

        for result in invalid_attempts {
            assert!(matches!(result, Err(StorageError::Sql(_))));
        }
        assert_eq!(
            storage
                .query_i64("SELECT COUNT(*) FROM clipboard_history", &[])
                .unwrap(),
            0
        );
    }
}
