use std::sync::Arc;

use rusqlite::{params, OptionalExtension, Row};
use sha2::{Digest, Sha256};
use thiserror::Error;

use super::storage::{StorageError, StorageService};

pub const CLIPBOARD_HISTORY_CAPACITY: u32 = 100;
pub const JS_MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SOURCE_APPLICATION_CHARS: usize = 256;
pub const MAX_SOURCE_PROCESS_CHARS: usize = 512;
pub const MAX_SEARCH_CHARS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardContentKind {
    Text,
    Image,
}

impl ClipboardContentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
        }
    }

    fn parse(value: &str) -> Result<Self, ClipboardError> {
        match value {
            "text" => Ok(Self::Text),
            "image" => Ok(Self::Image),
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
    pub source_application: Option<String>,
    pub source_process: Option<String>,
    pub captured_at_ms: u64,
    pub byte_size: u64,
    pub is_favorite: bool,
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
    #[error("clipboard history contains an invalid record")]
    CorruptRecord,
}

#[derive(Debug)]
pub struct ClipboardService {
    storage: Arc<StorageService>,
}

impl ClipboardService {
    pub fn initialize(storage: Arc<StorageService>) -> Self {
        Self { storage }
    }

    #[allow(dead_code)] // Intentionally not exposed as a frontend command.
    pub fn record_text(
        &self,
        text: String,
        metadata: ClipboardCaptureMetadata,
    ) -> Result<ClipboardRecordResult, ClipboardError> {
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

        let id = self.storage.transaction(|transaction| {
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
                    byte_size = excluded.byte_size
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

            let count: i64 =
                transaction.query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                    row.get(0)
                })?;
            let excess = count.saturating_sub(i64::from(CLIPBOARD_HISTORY_CAPACITY));
            if excess > 0 {
                transaction.execute(
                    "DELETE FROM clipboard_history
                     WHERE id IN (
                        SELECT id FROM clipboard_history
                        WHERE is_favorite = 0
                        ORDER BY captured_at_ms ASC, id ASC
                        LIMIT ?1
                     )",
                    [excess],
                )?;
            }
            Ok(id)
        })?;

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
        let limit = i64::from(query.limit);

        let (raw_items, total_count) = self.storage.read(|connection| {
            let total_count = connection.query_row(
                "SELECT COUNT(*) FROM clipboard_history
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
                   )",
                params![favorite_filter, search.as_deref()],
                |row| row.get::<_, i64>(0),
            )?;

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
                    is_favorite
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
                 ORDER BY captured_at_ms DESC, id DESC
                 LIMIT ?3",
            )?;
            let items = statement
                .query_map(params![favorite_filter, search.as_deref(), limit], raw_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok((items, total_count))
        })?;

        let items = raw_items
            .into_iter()
            .map(ClipboardHistoryItem::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        let total_count = u64::try_from(total_count).map_err(|_| ClipboardError::CorruptRecord)?;
        validate_safe_integer(total_count)?;
        Ok(ClipboardHistoryPage { items, total_count })
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

    pub fn delete(&self, id: i64) -> Result<bool, ClipboardError> {
        validate_id(id)?;
        let deleted = self.storage.transaction(|transaction| {
            Ok(transaction.execute("DELETE FROM clipboard_history WHERE id = ?1", [id])?)
        })?;
        Ok(deleted > 0)
    }

    pub fn clear_unfavorite(&self) -> Result<u64, ClipboardError> {
        let deleted = self.storage.transaction(|transaction| {
            Ok(transaction.execute("DELETE FROM clipboard_history WHERE is_favorite = 0", [])?)
        })?;
        u64::try_from(deleted).map_err(|_| ClipboardError::NumericRange)
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
                        is_favorite
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
    })
}

impl TryFrom<RawClipboardHistoryItem> for ClipboardHistoryItem {
    type Error = ClipboardError;

    fn try_from(raw: RawClipboardHistoryItem) -> Result<Self, Self::Error> {
        validate_id(raw.id)?;
        let kind = ClipboardContentKind::parse(&raw.content_type)?;
        match kind {
            ClipboardContentKind::Text if raw.text_content.is_some() && raw.file_path.is_none() => {
            }
            ClipboardContentKind::Image
                if raw.text_content.is_none() && raw.file_path.is_some() => {}
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
        Ok(Self {
            id: raw.id,
            kind,
            text_content: raw.text_content,
            file_path: raw.file_path,
            source_application: raw.source_application,
            source_process: raw.source_process,
            captured_at_ms,
            byte_size,
            is_favorite,
        })
    }
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
