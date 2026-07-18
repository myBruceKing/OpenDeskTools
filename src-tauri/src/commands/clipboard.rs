use serde::{Deserialize, Serialize};
use tauri::State;

use crate::infrastructure::application::ApplicationRuntime;
use crate::infrastructure::clipboard::{
    ClipboardContentKind, ClipboardError, ClipboardHistoryItem, ClipboardHistoryQuery,
};
use crate::infrastructure::clipboard_listener::ClipboardListenerStatus;

const DEFAULT_HISTORY_LIMIT: u32 = 100;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipboardHistoryScopeInput {
    All,
    Favorites,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClipboardHistoryQueryInput {
    scope: ClipboardHistoryScopeInput,
    search: Option<String>,
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClipboardFavoriteInput {
    id: String,
    is_favorite: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClipboardItemIdInput {
    id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardHistoryItemDto {
    id: String,
    kind: &'static str,
    text_content: Option<String>,
    source_application: Option<String>,
    source_process: Option<String>,
    captured_at_ms: u64,
    byte_size: u64,
    is_favorite: bool,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardHistoryPageDto {
    items: Vec<ClipboardHistoryItemDto>,
    total_count: u64,
    monitoring: ClipboardMonitoringDto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipboardMonitoringDto {
    Running,
    Unavailable,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardDeleteResultDto {
    deleted: bool,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardClearResultDto {
    deleted_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardCommandErrorDto {
    code: &'static str,
    message: &'static str,
    retryable: bool,
}

#[tauri::command]
pub fn get_clipboard_history(
    runtime: State<'_, ApplicationRuntime>,
    query: ClipboardHistoryQueryInput,
) -> Result<ClipboardHistoryPageDto, ClipboardCommandErrorDto> {
    get_history(&runtime, query)
}

#[tauri::command]
pub fn set_clipboard_history_favorite(
    runtime: State<'_, ApplicationRuntime>,
    input: ClipboardFavoriteInput,
) -> Result<ClipboardHistoryItemDto, ClipboardCommandErrorDto> {
    set_favorite(&runtime, input)
}

#[tauri::command]
pub fn delete_clipboard_history_item(
    runtime: State<'_, ApplicationRuntime>,
    input: ClipboardItemIdInput,
) -> Result<ClipboardDeleteResultDto, ClipboardCommandErrorDto> {
    delete_item(&runtime, input)
}

#[tauri::command]
pub fn clear_unfavorite_clipboard_history(
    runtime: State<'_, ApplicationRuntime>,
) -> Result<ClipboardClearResultDto, ClipboardCommandErrorDto> {
    clear_unfavorite(&runtime)
}

fn get_history(
    runtime: &ApplicationRuntime,
    input: ClipboardHistoryQueryInput,
) -> Result<ClipboardHistoryPageDto, ClipboardCommandErrorDto> {
    let page = runtime
        .clipboard()
        .history(ClipboardHistoryQuery {
            favorites_only: matches!(input.scope, ClipboardHistoryScopeInput::Favorites),
            search: input.search,
            limit: input.limit.unwrap_or(DEFAULT_HISTORY_LIMIT),
        })
        .map_err(map_error)?;
    let items = page
        .items
        .into_iter()
        .map(item_dto)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ClipboardHistoryPageDto {
        items,
        total_count: page.total_count,
        monitoring: monitoring_dto(runtime.clipboard_listener().status()),
    })
}

fn monitoring_dto(status: ClipboardListenerStatus) -> ClipboardMonitoringDto {
    match status {
        ClipboardListenerStatus::Running => ClipboardMonitoringDto::Running,
        ClipboardListenerStatus::Unavailable | ClipboardListenerStatus::Stopped => {
            ClipboardMonitoringDto::Unavailable
        }
    }
}

fn set_favorite(
    runtime: &ApplicationRuntime,
    input: ClipboardFavoriteInput,
) -> Result<ClipboardHistoryItemDto, ClipboardCommandErrorDto> {
    let id = parse_id(&input.id)?;
    runtime
        .clipboard()
        .set_favorite(id, input.is_favorite)
        .map_err(map_error)
        .and_then(item_dto)
}

fn delete_item(
    runtime: &ApplicationRuntime,
    input: ClipboardItemIdInput,
) -> Result<ClipboardDeleteResultDto, ClipboardCommandErrorDto> {
    let id = parse_id(&input.id)?;
    runtime
        .clipboard()
        .delete(id)
        .map(|deleted| ClipboardDeleteResultDto { deleted })
        .map_err(map_error)
}

fn clear_unfavorite(
    runtime: &ApplicationRuntime,
) -> Result<ClipboardClearResultDto, ClipboardCommandErrorDto> {
    runtime
        .clipboard()
        .clear_unfavorite()
        .map(|deleted_count| ClipboardClearResultDto { deleted_count })
        .map_err(map_error)
}

fn parse_id(value: &str) -> Result<i64, ClipboardCommandErrorDto> {
    let canonical_decimal = value.as_bytes().split_first().is_some_and(|(first, rest)| {
        (b'1'..=b'9').contains(first) && rest.iter().all(u8::is_ascii_digit)
    });
    if canonical_decimal {
        if let Ok(id) = value.parse::<i64>() {
            return Ok(id);
        }
    }
    Err(ClipboardCommandErrorDto {
        code: "invalid_clipboard_item_id",
        message: "Clipboard history item id is invalid.",
        retryable: false,
    })
}

fn item_dto(
    item: ClipboardHistoryItem,
) -> Result<ClipboardHistoryItemDto, ClipboardCommandErrorDto> {
    if item.kind != ClipboardContentKind::Text
        || item.text_content.is_none()
        || item.file_path.is_some()
    {
        return Err(ClipboardCommandErrorDto {
            code: "clipboard_content_unavailable",
            message: "This clipboard content type is not available yet.",
            retryable: false,
        });
    }
    Ok(ClipboardHistoryItemDto {
        id: item.id.to_string(),
        kind: item.kind.as_str(),
        text_content: item.text_content,
        source_application: item.source_application,
        source_process: item.source_process,
        captured_at_ms: item.captured_at_ms,
        byte_size: item.byte_size,
        is_favorite: item.is_favorite,
    })
}

fn map_error(error: ClipboardError) -> ClipboardCommandErrorDto {
    match error {
        ClipboardError::InvalidLimit | ClipboardError::SearchTooLong => ClipboardCommandErrorDto {
            code: "invalid_clipboard_history_query",
            message: "Clipboard history query is invalid.",
            retryable: false,
        },
        ClipboardError::NotFound => ClipboardCommandErrorDto {
            code: "clipboard_item_not_found",
            message: "Clipboard history item was not found.",
            retryable: false,
        },
        ClipboardError::EmptyText
        | ClipboardError::TextTooLarge
        | ClipboardError::SourceApplicationTooLong
        | ClipboardError::SourceProcessTooLong
        | ClipboardError::NumericRange => ClipboardCommandErrorDto {
            code: "invalid_clipboard_content",
            message: "Clipboard content is invalid.",
            retryable: false,
        },
        ClipboardError::Storage(_) | ClipboardError::CorruptRecord => ClipboardCommandErrorDto {
            code: "clipboard_history_unavailable",
            message: "Clipboard history is temporarily unavailable.",
            retryable: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use tauri::ipc::{InvokeResponseBody, IpcResponse};
    use tempfile::tempdir;

    use crate::infrastructure::clipboard::ClipboardCaptureMetadata;
    use crate::infrastructure::storage::StorageError;

    use super::*;

    fn runtime() -> (tempfile::TempDir, ApplicationRuntime) {
        let temp = tempdir().unwrap();
        let runtime = ApplicationRuntime::from_app_data_dir(temp.path().join("app-data")).unwrap();
        (temp, runtime)
    }

    fn record(runtime: &ApplicationRuntime, text: &str, timestamp: u64) -> i64 {
        runtime
            .clipboard()
            .record_text(
                text.to_owned(),
                ClipboardCaptureMetadata {
                    captured_at_ms: timestamp,
                    source_application: Some("TestEditor".to_owned()),
                    source_process: None,
                },
            )
            .unwrap()
            .item
            .unwrap()
            .id
    }

    #[test]
    fn history_command_has_stable_camel_case_json_without_file_paths() {
        let (_temp, runtime) = runtime();
        let id = record(&runtime, "hello", 1_720_000_000_123);

        let response = get_history(
            &runtime,
            ClipboardHistoryQueryInput {
                scope: ClipboardHistoryScopeInput::All,
                search: None,
                limit: Some(10),
            },
        )
        .unwrap()
        .body()
        .unwrap();
        let InvokeResponseBody::Json(json) = response else {
            panic!("clipboard page must serialize as JSON");
        };

        assert_eq!(
            json,
            format!(
                "{{\"items\":[{{\"id\":\"{id}\",\"kind\":\"text\",\"textContent\":\"hello\",\"sourceApplication\":\"TestEditor\",\"sourceProcess\":null,\"capturedAtMs\":1720000000123,\"byteSize\":5,\"isFavorite\":false}}],\"totalCount\":1,\"monitoring\":\"unavailable\"}}"
            )
        );
        assert!(!json.contains("filePath"));
    }

    #[test]
    fn stopped_listener_maps_to_unavailable_monitoring_without_blocking_history() {
        let (_temp, runtime) = runtime();

        let page = get_history(
            &runtime,
            ClipboardHistoryQueryInput {
                scope: ClipboardHistoryScopeInput::All,
                search: None,
                limit: Some(100),
            },
        )
        .unwrap();

        assert_eq!(
            runtime.clipboard_listener().status(),
            ClipboardListenerStatus::Stopped
        );
        assert_eq!(page.monitoring, ClipboardMonitoringDto::Unavailable);
        assert_eq!(page.total_count, 0);
        assert_eq!(
            monitoring_dto(ClipboardListenerStatus::Running),
            ClipboardMonitoringDto::Running
        );
        assert_eq!(
            monitoring_dto(ClipboardListenerStatus::Unavailable),
            ClipboardMonitoringDto::Unavailable
        );
    }

    #[test]
    fn mutation_commands_return_persisted_truth_and_decimal_string_ids() {
        let (_temp, runtime) = runtime();
        let id = record(&runtime, "favorite", 10);

        let favorite = set_favorite(
            &runtime,
            ClipboardFavoriteInput {
                id: id.to_string(),
                is_favorite: true,
            },
        )
        .unwrap();
        assert_eq!(favorite.id, id.to_string());
        assert!(favorite.is_favorite);

        assert_eq!(clear_unfavorite(&runtime).unwrap().deleted_count, 0);
        assert!(
            delete_item(&runtime, ClipboardItemIdInput { id: id.to_string() })
                .unwrap()
                .deleted
        );
        assert!(
            !delete_item(&runtime, ClipboardItemIdInput { id: id.to_string() })
                .unwrap()
                .deleted
        );
    }

    #[test]
    fn invalid_query_and_ids_return_non_retryable_safe_errors() {
        let (_temp, runtime) = runtime();
        let query_error = get_history(
            &runtime,
            ClipboardHistoryQueryInput {
                scope: ClipboardHistoryScopeInput::All,
                search: None,
                limit: Some(101),
            },
        )
        .unwrap_err();
        assert_eq!(query_error.code, "invalid_clipboard_history_query");
        assert!(!query_error.retryable);

        let search_error = get_history(
            &runtime,
            ClipboardHistoryQueryInput {
                scope: ClipboardHistoryScopeInput::All,
                search: Some("界".repeat(crate::infrastructure::clipboard::MAX_SEARCH_CHARS + 1)),
                limit: Some(100),
            },
        )
        .unwrap_err();
        assert_eq!(search_error.code, "invalid_clipboard_history_query");
        assert_eq!(search_error.message, "Clipboard history query is invalid.");
        assert!(!search_error.retryable);

        for invalid_id in ["not-an-id", "01", "+1", "-1", "0", "9223372036854775808"] {
            let id_error = delete_item(
                &runtime,
                ClipboardItemIdInput {
                    id: invalid_id.to_owned(),
                },
            )
            .unwrap_err();
            assert_eq!(id_error.code, "invalid_clipboard_item_id");
            assert!(!id_error.retryable);
        }
    }

    #[test]
    fn internal_storage_errors_serialize_without_sql_or_paths() {
        let error = map_error(ClipboardError::Storage(StorageError::Sql(
            rusqlite::Error::InvalidQuery,
        )));
        let response = error.body().unwrap();
        let InvokeResponseBody::Json(json) = response else {
            panic!("clipboard error must serialize as JSON");
        };

        assert_eq!(
            json,
            r#"{"code":"clipboard_history_unavailable","message":"Clipboard history is temporarily unavailable.","retryable":true}"#
        );
        assert!(!json.contains("SQLite"));
        assert!(!json.contains(".sqlite3"));
        assert!(!json.contains("InvalidQuery"));
    }
}
