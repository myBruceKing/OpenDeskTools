use serde::{Deserialize, Serialize};
use tauri::{ipc::Response, AppHandle, State, WebviewWindow};

use crate::infrastructure::application::ApplicationRuntime;
use crate::infrastructure::clipboard::{
    file_names, ClipboardContentKind, ClipboardError, ClipboardHistoryItem, ClipboardHistoryQuery,
};
use crate::infrastructure::clipboard_input::{
    ClipboardActionKind, ClipboardActionOutcome, ClipboardInputError,
};
use crate::infrastructure::clipboard_listener::ClipboardListenerStatus;
use crate::infrastructure::clipboard_settings::{
    parse_ignored_apps, ClipboardHistoryReuseStrategy, ClipboardSettings,
};
use crate::infrastructure::clipboard_surface_window::{
    self, ClipboardPreviewCloseReason, ClipboardSurfaceCloseReason,
};
use crate::infrastructure::clipboard_writer::ClipboardWriterError;
use crate::infrastructure::debug_qa;
use crate::infrastructure::image::ImageError;
use crate::infrastructure::surface::SurfaceError;

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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClipboardMonitoringInput {
    enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClipboardSettingsInput {
    retention_days: Option<u16>,
    max_items: u32,
    ignored_apps: Vec<String>,
    history_reuse_strategy: ClipboardHistoryReuseStrategyInput,
    sensitive_rules: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipboardHistoryReuseStrategyInput {
    Promote,
    Keep,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardPreviewDebugEventInput {
    OpenRequested,
    OpenResolved,
    OpenFailed,
    CloseScheduled,
    CloseCanceled,
    CloseFired,
    CloseRequested,
    CloseResolved,
    CloseFailed,
    HoverInside,
    HoverOutside,
    WindowBlurIgnored,
    SurfaceMetrics,
}

impl ClipboardPreviewDebugEventInput {
    const fn as_str(self) -> &'static str {
        match self {
            Self::OpenRequested => "open_requested",
            Self::OpenResolved => "open_resolved",
            Self::OpenFailed => "open_failed",
            Self::CloseScheduled => "close_scheduled",
            Self::CloseCanceled => "close_canceled",
            Self::CloseFired => "close_fired",
            Self::CloseRequested => "close_requested",
            Self::CloseResolved => "close_resolved",
            Self::CloseFailed => "close_failed",
            Self::HoverInside => "hover_inside",
            Self::HoverOutside => "hover_outside",
            Self::WindowBlurIgnored => "window_blur_ignored",
            Self::SurfaceMetrics => "surface_metrics",
        }
    }
}

const MAX_CLIPBOARD_PREVIEW_DEBUG_DETAIL_BYTES: usize = 4_096;

fn sanitize_clipboard_preview_debug_detail(detail: Option<String>) -> Option<String> {
    detail.map(|detail| {
        let mut sanitized =
            String::with_capacity(detail.len().min(MAX_CLIPBOARD_PREVIEW_DEBUG_DETAIL_BYTES));
        for character in detail.chars() {
            let character = if character.is_control() {
                ' '
            } else {
                character
            };
            if sanitized.len() + character.len_utf8() > MAX_CLIPBOARD_PREVIEW_DEBUG_DETAIL_BYTES {
                break;
            }
            sanitized.push(character);
        }
        sanitized
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClipboardTextUpdateInput {
    id: String,
    text_content: String,
    expected_revision: u64,
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
    revision: u64,
    source_icon_available: bool,
    file_count: Option<u32>,
    file_names: Option<Vec<String>>,
    display_category: &'static str,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardHistoryPageDto {
    items: Vec<ClipboardHistoryItemDto>,
    total_count: u64,
    monitoring: ClipboardMonitoringDto,
    input_available: bool,
    surface_active: bool,
    settings: ClipboardSettingsDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardSettingsDto {
    retention_days: Option<u16>,
    max_items: u32,
    ignored_apps: Vec<String>,
    history_reuse_strategy: &'static str,
    sensitive_rules: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardSettingsUpdateDto {
    settings: ClipboardSettingsDto,
    removed_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipboardMonitoringDto {
    Running,
    Paused,
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

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardActionResultDto {
    action: &'static str,
    clipboard_updated: bool,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardSurfaceCloseResultDto {
    closed: bool,
    input_available: bool,
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
pub fn set_clipboard_monitoring(
    app: AppHandle,
    runtime: State<'_, ApplicationRuntime>,
    input: ClipboardMonitoringInput,
) -> Result<ClipboardMonitoringDto, ClipboardCommandErrorDto> {
    runtime
        .set_clipboard_monitoring_enabled(input.enabled, crate::clipboard_history_event_sink(&app))
        .map(monitoring_dto)
        .map_err(|_| ClipboardCommandErrorDto {
            code: "clipboard_monitoring_unavailable",
            message: "剪贴板监控状态未能更新，请重试。",
            retryable: true,
        })
}

#[tauri::command]
pub fn update_clipboard_settings(
    runtime: State<'_, ApplicationRuntime>,
    input: ClipboardSettingsInput,
) -> Result<ClipboardSettingsUpdateDto, ClipboardCommandErrorDto> {
    let ignored_apps = parse_ignored_apps(&input.ignored_apps.join("\n"))
        .map_err(|error| map_error(ClipboardError::Settings(error)))?;
    let settings = ClipboardSettings {
        retention_days: input.retention_days,
        max_items: input.max_items,
        ignored_apps,
        history_reuse_strategy: match input.history_reuse_strategy {
            ClipboardHistoryReuseStrategyInput::Promote => ClipboardHistoryReuseStrategy::Promote,
            ClipboardHistoryReuseStrategyInput::Keep => ClipboardHistoryReuseStrategy::Keep,
        },
        sensitive_rules: input.sensitive_rules,
    };
    let removed_count = runtime
        .clipboard()
        .update_settings(settings.clone())
        .map_err(map_error)?;
    Ok(ClipboardSettingsUpdateDto {
        settings: settings_dto(settings),
        removed_count,
    })
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

#[tauri::command]
pub fn get_clipboard_history_image(
    runtime: State<'_, ApplicationRuntime>,
    input: ClipboardItemIdInput,
) -> Result<Response, ClipboardCommandErrorDto> {
    get_image(&runtime, input)
}

#[tauri::command]
pub fn update_clipboard_history_text(
    runtime: State<'_, ApplicationRuntime>,
    input: ClipboardTextUpdateInput,
) -> Result<ClipboardHistoryItemDto, ClipboardCommandErrorDto> {
    update_text(&runtime, input)
}

#[tauri::command]
pub fn get_clipboard_history_source_icon(
    runtime: State<'_, ApplicationRuntime>,
    input: ClipboardItemIdInput,
) -> Result<Response, ClipboardCommandErrorDto> {
    get_source_icon(&runtime, input)
}

fn get_source_icon(
    runtime: &ApplicationRuntime,
    input: ClipboardItemIdInput,
) -> Result<Response, ClipboardCommandErrorDto> {
    let id = parse_id(&input.id)?;
    runtime
        .clipboard()
        .source_icon_bytes(id)
        .map(Response::new)
        .map_err(|_| ClipboardCommandErrorDto {
            code: "clipboard_source_icon_unavailable",
            message: "Source application icon is unavailable.",
            retryable: false,
        })
}

#[tauri::command]
pub fn copy_clipboard_history_item(
    runtime: State<'_, ApplicationRuntime>,
    window: WebviewWindow,
    input: ClipboardItemIdInput,
) -> Result<ClipboardActionResultDto, ClipboardCommandErrorDto> {
    let id = parse_id(&input.id)?;
    let owner_window = native_window_handle(&window)?;
    let result = runtime
        .clipboard_input()
        .copy(id, owner_window, |sequence| {
            runtime.clipboard_listener().suppress_sequence(sequence);
        })
        .map_err(map_input_error)?;
    runtime
        .clipboard()
        .apply_history_reuse(id)
        .map_err(map_error)?;
    Ok(action_dto(result))
}

#[tauri::command]
pub fn input_clipboard_history_item(
    runtime: State<'_, ApplicationRuntime>,
    app: AppHandle,
    window: WebviewWindow,
    input: ClipboardItemIdInput,
) -> Result<ClipboardActionResultDto, ClipboardCommandErrorDto> {
    let id = parse_id(&input.id)?;
    let owner_window = native_window_handle(&window)?;
    let result = runtime
        .clipboard_input()
        .input(id, owner_window, |sequence| {
            runtime.clipboard_listener().suppress_sequence(sequence);
        });
    match result {
        Ok(outcome) => {
            runtime
                .clipboard()
                .apply_history_reuse(id)
                .map_err(map_error)?;
            // A newer generation may have opened while paste consumption was
            // settling. Only hide when this input actually ended the surface.
            if !runtime.surface().surface_active() {
                if let Err(error) = clipboard_surface_window::close(
                    &app,
                    runtime.surface(),
                    ClipboardSurfaceCloseReason::InputSucceeded,
                ) {
                    eprintln!(
                        "clipboard input succeeded but the surface remained visible: {error}"
                    );
                }
            }
            Ok(action_dto(outcome))
        }
        // A no-activate surface remains visible while the target is restored;
        // return the error in place instead of stealing focus to "reshow" it.
        Err(error) => Err(map_input_error(error)),
    }
}

#[tauri::command]
pub fn close_clipboard_surface(
    runtime: State<'_, ApplicationRuntime>,
    app: AppHandle,
) -> Result<ClipboardSurfaceCloseResultDto, ClipboardCommandErrorDto> {
    clipboard_surface_window::close(
        &app,
        runtime.surface(),
        ClipboardSurfaceCloseReason::Command,
    )
    .map_err(|_| window_unavailable_error())?;
    Ok(ClipboardSurfaceCloseResultDto {
        closed: true,
        input_available: false,
    })
}

#[tauri::command]
pub fn open_clipboard_preview_surface(
    app: AppHandle,
    record_id: String,
) -> Result<(), ClipboardCommandErrorDto> {
    debug_qa::trace(format!(
        "preview command=open request raw_record_id={record_id:?}"
    ));
    let id = parse_id(&record_id)?;
    clipboard_surface_window::open_preview(&app, id.to_string()).map_err(|error| {
        debug_qa::trace(format!(
            "preview command=open result=error record_id={id} error={error}"
        ));
        window_unavailable_error()
    })?;
    debug_qa::trace(format!(
        "preview command=open result=success record_id={id}"
    ));
    Ok(())
}

#[tauri::command]
pub fn close_clipboard_preview_surface(app: AppHandle) -> Result<(), ClipboardCommandErrorDto> {
    debug_qa::trace("preview command=close request");
    clipboard_surface_window::close_preview(&app, ClipboardPreviewCloseReason::Command).map_err(
        |error| {
            debug_qa::trace(format!("preview command=close result=error error={error}"));
            window_unavailable_error()
        },
    )?;
    debug_qa::trace("preview command=close result=success");
    Ok(())
}

#[tauri::command]
pub fn trace_clipboard_preview_debug(
    event: ClipboardPreviewDebugEventInput,
    record_id: Option<String>,
    detail: Option<String>,
) {
    let record_id = record_id.and_then(|value| parse_id(&value).ok());
    let detail = sanitize_clipboard_preview_debug_detail(detail);
    debug_qa::trace(format!(
        "preview frontend event={} record_id={record_id:?} detail={detail:?}",
        event.as_str(),
    ));
}

#[tauri::command]
pub fn set_clipboard_surface_underlay_color(
    app: AppHandle,
    color: String,
) -> Result<(), ClipboardCommandErrorDto> {
    let color = clipboard_surface_window::ClipboardSurfaceUnderlayColor::parse_hex(&color)
        .ok_or_else(invalid_surface_underlay_color_error)?;
    clipboard_surface_window::set_group_underlay_color(&app, color)
        .map_err(|_| surface_underlay_unavailable_error())
}

#[tauri::command]
pub fn get_clipboard_preview_surface_state(
    app: AppHandle,
) -> Result<clipboard_surface_window::ClipboardPreviewSurfaceState, ClipboardCommandErrorDto> {
    clipboard_surface_window::preview_state(&app).map_err(|_| window_unavailable_error())
}

fn get_image(
    runtime: &ApplicationRuntime,
    input: ClipboardItemIdInput,
) -> Result<Response, ClipboardCommandErrorDto> {
    let id = parse_id(&input.id)?;
    runtime
        .clipboard()
        .image_bytes(id)
        .map(Response::new)
        .map_err(map_error)
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
    let settings = runtime.clipboard().settings().map_err(map_error)?;
    let items = page
        .items
        .into_iter()
        .map(item_dto)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ClipboardHistoryPageDto {
        items,
        total_count: page.total_count,
        monitoring: monitoring_dto(runtime.clipboard_listener().status()),
        input_available: runtime.surface().input_available(),
        surface_active: runtime.surface().surface_active(),
        settings: settings_dto(settings),
    })
}

fn settings_dto(settings: ClipboardSettings) -> ClipboardSettingsDto {
    ClipboardSettingsDto {
        retention_days: settings.retention_days,
        max_items: settings.max_items,
        ignored_apps: settings.ignored_apps,
        history_reuse_strategy: settings.history_reuse_strategy.as_str(),
        sensitive_rules: settings.sensitive_rules,
    }
}

#[cfg(windows)]
fn native_window_handle(window: &WebviewWindow) -> Result<usize, ClipboardCommandErrorDto> {
    window
        .hwnd()
        .map(|handle| handle.0 as usize)
        .map_err(|_| window_unavailable_error())
}

#[cfg(not(windows))]
fn native_window_handle(_window: &WebviewWindow) -> Result<usize, ClipboardCommandErrorDto> {
    Err(window_unavailable_error())
}

fn window_unavailable_error() -> ClipboardCommandErrorDto {
    ClipboardCommandErrorDto {
        code: "clipboard_write_unavailable",
        message: "Clipboard writing is temporarily unavailable.",
        retryable: true,
    }
}

fn invalid_surface_underlay_color_error() -> ClipboardCommandErrorDto {
    ClipboardCommandErrorDto {
        code: "invalid_clipboard_surface_underlay_color",
        message: "Clipboard surface underlay color must use #RRGGBB format.",
        retryable: false,
    }
}

fn surface_underlay_unavailable_error() -> ClipboardCommandErrorDto {
    ClipboardCommandErrorDto {
        code: "clipboard_surface_underlay_unavailable",
        message: "Clipboard surface background is temporarily unavailable.",
        retryable: true,
    }
}

fn action_dto(outcome: ClipboardActionOutcome) -> ClipboardActionResultDto {
    ClipboardActionResultDto {
        action: match outcome.action {
            ClipboardActionKind::Copied => "copied",
            ClipboardActionKind::Input => "input",
        },
        clipboard_updated: outcome.clipboard_updated,
    }
}

fn map_input_error(error: ClipboardInputError) -> ClipboardCommandErrorDto {
    match error {
        ClipboardInputError::Clipboard(error) => map_error(error),
        ClipboardInputError::Surface(error) => map_surface_error(error),
        ClipboardInputError::Writer(ClipboardWriterError::WindowsApi("SetClipboardData")) => {
            ClipboardCommandErrorDto {
                code: "clipboard_write_failed",
                message: "Windows rejected the selected clipboard data after replacement began.",
                retryable: true,
            }
        }
        ClipboardInputError::InputCleanupDenied => ClipboardCommandErrorDto {
            code: "clipboard_input_cleanup_failed",
            message: "Windows did not release the synthetic input keys safely.",
            retryable: false,
        },
        ClipboardInputError::ModifierPressed | ClipboardInputError::InputDenied => {
            ClipboardCommandErrorDto {
                code: "clipboard_input_denied",
                message: "Windows did not allow input to the target application.",
                retryable: false,
            }
        }
        ClipboardInputError::ClipboardChanged => ClipboardCommandErrorDto {
            code: "clipboard_operation_not_applied",
            message: "The clipboard changed before the selected history item could be written.",
            retryable: true,
        },
        ClipboardInputError::Writer(_) => window_unavailable_error(),
    }
}

fn map_surface_error(error: SurfaceError) -> ClipboardCommandErrorDto {
    match error {
        SurfaceError::FocusDenied => ClipboardCommandErrorDto {
            code: "clipboard_target_focus_denied",
            message: "Windows did not allow the target application to receive focus.",
            retryable: true,
        },
        SurfaceError::InputAttachmentDenied => ClipboardCommandErrorDto {
            code: "clipboard_input_denied",
            message: "Windows did not allow access to the target input thread.",
            retryable: false,
        },
        SurfaceError::TargetUnavailable => ClipboardCommandErrorDto {
            code: "clipboard_target_unavailable",
            message: "The original target application is no longer available.",
            retryable: false,
        },
        SurfaceError::LockPoisoned => window_unavailable_error(),
        #[cfg(not(windows))]
        SurfaceError::UnsupportedPlatform => window_unavailable_error(),
    }
}

fn monitoring_dto(status: ClipboardListenerStatus) -> ClipboardMonitoringDto {
    match status {
        ClipboardListenerStatus::Running => ClipboardMonitoringDto::Running,
        ClipboardListenerStatus::Stopped => ClipboardMonitoringDto::Paused,
        ClipboardListenerStatus::Unavailable => ClipboardMonitoringDto::Unavailable,
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

fn update_text(
    runtime: &ApplicationRuntime,
    input: ClipboardTextUpdateInput,
) -> Result<ClipboardHistoryItemDto, ClipboardCommandErrorDto> {
    let id = parse_id(&input.id)?;
    runtime
        .clipboard()
        .update_text(id, input.text_content, input.expected_revision)
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
    let shape_is_valid = match item.kind {
        ClipboardContentKind::Text => {
            item.text_content.is_some() && item.file_path.is_none() && item.file_paths.is_none()
        }
        ClipboardContentKind::Image => {
            item.text_content.is_none() && item.file_path.is_some() && item.file_paths.is_none()
        }
        ClipboardContentKind::Files => {
            item.text_content.is_none()
                && item.file_path.is_none()
                && item
                    .file_paths
                    .as_ref()
                    .is_some_and(|paths| !paths.is_empty())
        }
    };
    if !shape_is_valid {
        return Err(ClipboardCommandErrorDto {
            code: "clipboard_content_unavailable",
            message: "This clipboard content type is not available yet.",
            retryable: false,
        });
    }
    let (file_count, file_names, display_category) = if item.kind == ClipboardContentKind::Files {
        let names = file_names(
            item.file_paths
                .as_deref()
                .ok_or_else(clipboard_content_unavailable_error)?,
        )
        .map_err(|_| clipboard_content_unavailable_error())?;
        let count =
            u32::try_from(names.len()).map_err(|_| clipboard_content_unavailable_error())?;
        let category = file_display_category(&names);
        (Some(count), Some(names), category)
    } else {
        (
            None,
            None,
            match item.kind {
                ClipboardContentKind::Text => "text",
                ClipboardContentKind::Image => "image",
                ClipboardContentKind::Files => unreachable!(),
            },
        )
    };
    Ok(ClipboardHistoryItemDto {
        id: item.id.to_string(),
        kind: item.kind.as_str(),
        text_content: item.text_content,
        source_application: item.source_application,
        source_process: item.source_process,
        captured_at_ms: item.captured_at_ms,
        byte_size: item.byte_size,
        is_favorite: item.is_favorite,
        revision: item.revision,
        source_icon_available: item.source_icon_path.is_some(),
        file_count,
        file_names,
        display_category,
    })
}

fn clipboard_content_unavailable_error() -> ClipboardCommandErrorDto {
    ClipboardCommandErrorDto {
        code: "clipboard_content_unavailable",
        message: "This clipboard content is unavailable.",
        retryable: false,
    }
}

fn file_display_category(names: &[String]) -> &'static str {
    if names.len() != 1 {
        return "files";
    }
    let extension = names[0]
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase());
    match extension.as_deref() {
        Some("txt" | "md" | "log" | "csv" | "json" | "xml" | "yaml" | "yml") => "text",
        Some("png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tif" | "tiff" | "ico") => "image",
        _ => "files",
    }
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
        ClipboardError::EmptyText => ClipboardCommandErrorDto {
            code: "clipboard_edit_empty",
            message: "Clipboard text must not be empty.",
            retryable: false,
        },
        ClipboardError::DuplicateText => ClipboardCommandErrorDto {
            code: "clipboard_edit_duplicate",
            message: "The same text already exists in clipboard history.",
            retryable: false,
        },
        ClipboardError::RevisionConflict => ClipboardCommandErrorDto {
            code: "clipboard_revision_conflict",
            message: "Clipboard history changed; reload and retry.",
            retryable: true,
        },
        ClipboardError::TextTooLarge
        | ClipboardError::InvalidFiles
        | ClipboardError::SourceApplicationTooLong
        | ClipboardError::SourceProcessTooLong
        | ClipboardError::NumericRange => ClipboardCommandErrorDto {
            code: "invalid_clipboard_content",
            message: "Clipboard content is invalid.",
            retryable: false,
        },
        ClipboardError::FilesUnavailable => ClipboardCommandErrorDto {
            code: "clipboard_files_unavailable",
            message: "One or more clipboard files are no longer available.",
            retryable: false,
        },
        ClipboardError::Settings(_) => ClipboardCommandErrorDto {
            code: "invalid_clipboard_settings",
            message: "Clipboard settings are invalid.",
            retryable: false,
        },
        ClipboardError::Image(ImageError::TooLarge) => ClipboardCommandErrorDto {
            code: "clipboard_image_too_large",
            message: "Clipboard image exceeds the supported size.",
            retryable: false,
        },
        ClipboardError::Image(
            ImageError::Missing
            | ImageError::Corrupt
            | ImageError::InvalidReference
            | ImageError::InvalidImage,
        ) => ClipboardCommandErrorDto {
            code: "clipboard_image_unavailable",
            message: "Clipboard image is unavailable.",
            retryable: false,
        },
        ClipboardError::SourceIcon(_) => ClipboardCommandErrorDto {
            code: "clipboard_source_icon_unavailable",
            message: "Source application icon is unavailable.",
            retryable: false,
        },
        ClipboardError::Image(
            ImageError::Io(_) | ImageError::Storage(_) | ImageError::Encode(_),
        ) => ClipboardCommandErrorDto {
            code: "clipboard_history_unavailable",
            message: "Clipboard history is temporarily unavailable.",
            retryable: true,
        },
        ClipboardError::Storage(_)
        | ClipboardError::CorruptRecord
        | ClipboardError::LifecycleLockPoisoned
        | ClipboardError::SettingsLockPoisoned => ClipboardCommandErrorDto {
            code: "clipboard_history_unavailable",
            message: "Clipboard history is temporarily unavailable.",
            retryable: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};
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
    fn preview_debug_surface_metrics_detail_is_bounded_and_single_line() {
        let event: ClipboardPreviewDebugEventInput =
            serde_json::from_str(r#""surface_metrics""#).unwrap();
        assert!(matches!(
            event,
            ClipboardPreviewDebugEventInput::SurfaceMetrics
        ));

        let detail = format!("line1\r\nline2\t\0{}", "界".repeat(2_000));
        let sanitized = sanitize_clipboard_preview_debug_detail(Some(detail)).unwrap();

        assert!(sanitized.starts_with("line1  line2  "));
        assert!(
            sanitized.len() <= MAX_CLIPBOARD_PREVIEW_DEBUG_DETAIL_BYTES,
            "detail exceeded the UTF-8 byte limit"
        );
        assert!(sanitized.chars().all(|character| !character.is_control()));
        assert_eq!(sanitize_clipboard_preview_debug_detail(None), None);
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
                "{{\"items\":[{{\"id\":\"{id}\",\"kind\":\"text\",\"textContent\":\"hello\",\"sourceApplication\":\"TestEditor\",\"sourceProcess\":null,\"capturedAtMs\":1720000000123,\"byteSize\":5,\"isFavorite\":false,\"revision\":1,\"sourceIconAvailable\":false,\"fileCount\":null,\"fileNames\":null,\"displayCategory\":\"text\"}}],\"totalCount\":1,\"monitoring\":\"paused\",\"inputAvailable\":false,\"surfaceActive\":false,\"settings\":{{\"retentionDays\":30,\"maxItems\":100,\"ignoredApps\":[],\"historyReuseStrategy\":\"promote\",\"sensitiveRules\":[]}}}}"
            )
        );
        assert!(!json.contains("filePath"));
    }

    #[test]
    fn file_history_dto_exposes_only_names_count_and_safe_display_category() {
        let (temp, runtime) = runtime();
        let text_path = temp.path().join("private-notes.txt");
        std::fs::write(&text_path, b"secret").unwrap();
        let item = runtime
            .clipboard()
            .record_files(
                vec![crate::infrastructure::clipboard::path_to_utf16(&text_path)],
                crate::infrastructure::clipboard::ClipboardCaptureMetadata {
                    captured_at_ms: 1,
                    source_application: Some("Explorer".to_owned()),
                    source_process: Some("explorer.exe".to_owned()),
                },
            )
            .unwrap()
            .item
            .unwrap();
        let json = serde_json::to_value(item_dto(item).unwrap()).unwrap();
        assert_eq!(json["kind"], "files");
        assert_eq!(json["displayCategory"], "text");
        assert_eq!(json["fileCount"], 1);
        assert_eq!(json["fileNames"], serde_json::json!(["private-notes.txt"]));
        assert_eq!(json["textContent"], serde_json::Value::Null);
        let serialized = json.to_string();
        assert!(!serialized.contains(&temp.path().to_string_lossy().to_string()));
        assert!(!serialized.contains("filePaths"));

        assert_eq!(file_display_category(&["photo.PNG".to_owned()]), "image");
        assert_eq!(file_display_category(&["archive.zip".to_owned()]), "files");
        assert_eq!(
            file_display_category(&["one.txt".to_owned(), "two.png".to_owned()]),
            "files"
        );
    }

    #[test]
    fn stopped_listener_maps_to_paused_monitoring_without_blocking_history() {
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
        assert_eq!(page.monitoring, ClipboardMonitoringDto::Paused);
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
    fn browse_only_surface_contract_stays_active_until_backend_close_clears_it() {
        let (_temp, runtime) = runtime();
        runtime.surface().activate_without_target().unwrap();

        let active = get_history(
            &runtime,
            ClipboardHistoryQueryInput {
                scope: ClipboardHistoryScopeInput::All,
                search: None,
                limit: Some(100),
            },
        )
        .unwrap();
        assert!(active.surface_active);
        assert!(!active.input_available);

        runtime.surface().clear().unwrap();
        let closed = get_history(
            &runtime,
            ClipboardHistoryQueryInput {
                scope: ClipboardHistoryScopeInput::All,
                search: None,
                limit: Some(100),
            },
        )
        .unwrap();
        assert!(!closed.surface_active);
        assert!(!closed.input_available);
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
    fn text_edit_command_returns_persisted_revision_and_stable_conflict_codes() {
        let (_temp, runtime) = runtime();
        let first = record(&runtime, "first", 10);
        let second = record(&runtime, "second", 20);

        let updated = update_text(
            &runtime,
            ClipboardTextUpdateInput {
                id: first.to_string(),
                text_content: "edited".to_owned(),
                expected_revision: 1,
            },
        )
        .unwrap();
        assert_eq!(updated.text_content.as_deref(), Some("edited"));
        assert_eq!(updated.revision, 2);

        let stale = update_text(
            &runtime,
            ClipboardTextUpdateInput {
                id: first.to_string(),
                text_content: "stale".to_owned(),
                expected_revision: 1,
            },
        )
        .unwrap_err();
        assert_eq!(stale.code, "clipboard_revision_conflict");
        assert!(stale.retryable);

        let duplicate = update_text(
            &runtime,
            ClipboardTextUpdateInput {
                id: first.to_string(),
                text_content: "second".to_owned(),
                expected_revision: 2,
            },
        )
        .unwrap_err();
        assert_eq!(duplicate.code, "clipboard_edit_duplicate");
        assert!(!duplicate.retryable);

        let empty = update_text(
            &runtime,
            ClipboardTextUpdateInput {
                id: second.to_string(),
                text_content: String::new(),
                expected_revision: 1,
            },
        )
        .unwrap_err();
        assert_eq!(empty.code, "clipboard_edit_empty");
        assert!(!empty.retryable);
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

    #[test]
    fn clipboard_write_and_input_safety_failures_have_distinct_error_contracts() {
        let write = map_input_error(ClipboardInputError::Writer(
            ClipboardWriterError::WindowsApi("SetClipboardData"),
        ));
        assert_eq!(write.code, "clipboard_write_failed");
        assert!(write.retryable);
        assert!(write.message.contains("replacement began"));

        let cleanup = map_input_error(ClipboardInputError::InputCleanupDenied);
        assert_eq!(cleanup.code, "clipboard_input_cleanup_failed");
        assert!(!cleanup.retryable);
        assert!(cleanup.message.contains("synthetic input keys"));

        let attachment = map_input_error(ClipboardInputError::Surface(
            SurfaceError::InputAttachmentDenied,
        ));
        assert_eq!(attachment.code, "clipboard_input_denied");
        assert!(!attachment.retryable);
        assert!(attachment.message.contains("input thread"));

        let changed = map_input_error(ClipboardInputError::ClipboardChanged);
        assert_eq!(changed.code, "clipboard_operation_not_applied");
        assert!(changed.retryable);
        assert!(changed.message.contains("changed before"));
    }

    #[test]
    fn action_outcome_reports_permanent_clipboard_update_without_restore_contract() {
        let copied = action_dto(ClipboardActionOutcome {
            action: ClipboardActionKind::Copied,
            clipboard_updated: true,
        });
        let input = action_dto(ClipboardActionOutcome {
            action: ClipboardActionKind::Input,
            clipboard_updated: true,
        });

        assert_eq!(
            serde_json::to_string(&copied).unwrap(),
            r#"{"action":"copied","clipboardUpdated":true}"#
        );
        assert_eq!(
            serde_json::to_string(&input).unwrap(),
            r#"{"action":"input","clipboardUpdated":true}"#
        );
    }

    #[test]
    fn image_history_hides_file_reference_and_raw_command_returns_png_bytes() {
        let (temp, runtime) = runtime();
        let item = runtime
            .clipboard()
            .record_image(
                1,
                1,
                vec![4, 5, 6, 255],
                ClipboardCaptureMetadata {
                    captured_at_ms: 42,
                    source_application: None,
                    source_process: None,
                },
            )
            .unwrap()
            .item
            .unwrap();
        let page = get_history(
            &runtime,
            ClipboardHistoryQueryInput {
                scope: ClipboardHistoryScopeInput::All,
                search: None,
                limit: Some(100),
            },
        )
        .unwrap();
        let body = page.body().unwrap();
        let InvokeResponseBody::Json(json) = body else {
            panic!("history should be JSON");
        };
        assert!(json.contains("\"kind\":\"image\",\"textContent\":null"));
        assert!(!json.contains("filePath"));
        assert!(!json.contains("files/clipboard"));

        let response = get_image(
            &runtime,
            ClipboardItemIdInput {
                id: item.id.to_string(),
            },
        )
        .unwrap();
        let body = response.body().unwrap();
        let InvokeResponseBody::Raw(bytes) = body else {
            panic!("image should use raw IPC bytes");
        };
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        let managed_path = temp
            .path()
            .join("app-data")
            .join(item.file_path.as_deref().unwrap().replace('/', "\\"));
        std::fs::write(&managed_path, b"corrupt").unwrap();
        let corrupt = get_image(
            &runtime,
            ClipboardItemIdInput {
                id: item.id.to_string(),
            },
        )
        .err()
        .unwrap();
        assert_eq!(corrupt.code, "clipboard_image_unavailable");
        assert!(!corrupt.retryable);
        assert!(!corrupt.message.contains("files"));
        for invalid in ["../1", "01", "-1"] {
            let error = get_image(
                &runtime,
                ClipboardItemIdInput {
                    id: invalid.to_owned(),
                },
            )
            .err()
            .unwrap();
            assert_eq!(error.code, "invalid_clipboard_item_id");
            assert!(!error.message.contains("files"));
        }
        let text_id = record(&runtime, "not-image", 43);
        let error = get_image(
            &runtime,
            ClipboardItemIdInput {
                id: text_id.to_string(),
            },
        )
        .err()
        .unwrap();
        assert_eq!(error.code, "clipboard_item_not_found");

        let oversized = map_error(ClipboardError::Image(ImageError::TooLarge));
        assert_eq!(oversized.code, "clipboard_image_too_large");
        assert!(!oversized.retryable);

        for unavailable in [
            ImageError::Missing,
            ImageError::Corrupt,
            ImageError::InvalidReference,
            ImageError::InvalidImage,
        ] {
            let dto = map_error(ClipboardError::Image(unavailable));
            assert_eq!(dto.code, "clipboard_image_unavailable");
            assert!(!dto.retryable);
        }
        let transient = map_error(ClipboardError::Image(ImageError::Io(
            std::io::Error::other("test-only"),
        )));
        assert_eq!(transient.code, "clipboard_history_unavailable");
        assert!(transient.retryable);
    }

    #[test]
    fn source_icon_contract_exposes_only_availability_and_id_scoped_png_bytes() {
        let (_temp, runtime) = runtime();
        let id = record(&runtime, "with-icon", 44);
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 96, 96);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&vec![255; 96 * 96 * 4]).unwrap();
        }
        let hash = format!("{:x}", Sha256::digest(&png));
        let reference = format!("files/source-icons/{hash}.png");
        let path = runtime.storage().resolve_relative_path(&reference).unwrap();
        std::fs::write(path, &png).unwrap();
        runtime
            .storage()
            .transaction(|transaction| {
                transaction.execute(
                    "UPDATE clipboard_history SET source_icon_path = ?1 WHERE id = ?2",
                    rusqlite::params![reference, id],
                )?;
                Ok(())
            })
            .unwrap();

        let page = get_history(
            &runtime,
            ClipboardHistoryQueryInput {
                scope: ClipboardHistoryScopeInput::All,
                search: Some("with-icon".to_owned()),
                limit: Some(10),
            },
        )
        .unwrap();
        assert!(page.items[0].source_icon_available);
        let response = runtime.clipboard().source_icon_bytes(id).unwrap();
        assert_eq!(response, png);
        let json = serde_json::to_string(&page.items[0]).unwrap();
        assert!(!json.contains("source-icons"));
        assert!(!json.contains(&hash));

        std::fs::write(
            runtime.storage().resolve_relative_path(&reference).unwrap(),
            b"corrupt",
        )
        .unwrap();
        let error = get_source_icon(&runtime, ClipboardItemIdInput { id: id.to_string() })
            .err()
            .unwrap();
        assert_eq!(error.code, "clipboard_source_icon_unavailable");
        assert!(!error.retryable);
    }
}
