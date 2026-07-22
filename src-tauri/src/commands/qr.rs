use serde::Serialize;
use tauri::State;

use crate::infrastructure::application::ApplicationRuntime;
use crate::infrastructure::qr::{QrConversionKind, QrConversionResult, QrError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QrConversionDto {
    pub(crate) kind: &'static str,
    pub(crate) system_clipboard_synced: bool,
    pub(crate) message: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QrCommandErrorDto {
    pub(crate) code: &'static str,
    pub(crate) message: &'static str,
    retryable: bool,
}

#[tauri::command]
pub fn convert_latest_clipboard_qr(
    runtime: State<'_, ApplicationRuntime>,
) -> Result<QrConversionDto, QrCommandErrorDto> {
    convert_latest(&runtime)
}

pub(crate) fn convert_latest(
    runtime: &ApplicationRuntime,
) -> Result<QrConversionDto, QrCommandErrorDto> {
    runtime
        .qr()
        .convert_latest(|sequence| runtime.clipboard_listener().suppress_sequence(sequence))
        .map(conversion_dto)
        .map_err(map_error)
}

pub(crate) fn conversion_dto(result: QrConversionResult) -> QrConversionDto {
    let message = match (result.kind, result.system_clipboard_synced) {
        (QrConversionKind::TextToImage, true) => "已从最新内部文本生成二维码。",
        (QrConversionKind::TextToImage, false) => "已生成二维码并保存到历史；系统剪贴板同步失败。",
        (QrConversionKind::ImageToText, true) => "已识别最新内部图片中的二维码。",
        (QrConversionKind::ImageToText, false) => "已识别二维码并保存到历史；系统剪贴板同步失败。",
    };
    QrConversionDto {
        kind: result.kind.as_str(),
        system_clipboard_synced: result.system_clipboard_synced,
        message,
    }
}

pub(crate) fn map_error(error: QrError) -> QrCommandErrorDto {
    match error {
        QrError::NoLatestItem => QrCommandErrorDto {
            code: "clipboard_history_empty",
            message: "内置剪贴板没有可转换的最新记录。",
            retryable: false,
        },
        QrError::UnsupportedContent => QrCommandErrorDto {
            code: "qr_unsupported_content",
            message: "最新记录不是文本或图片，无法转换二维码。",
            retryable: false,
        },
        QrError::EmptyText => QrCommandErrorDto {
            code: "qr_empty_text",
            message: "最新文本为空，无法生成二维码。",
            retryable: false,
        },
        QrError::TextTooLarge => QrCommandErrorDto {
            code: "qr_text_too_large",
            message: "最新文本过长，无法生成二维码。",
            retryable: false,
        },
        QrError::UnreadableImage | QrError::NonTextPayload => QrCommandErrorDto {
            code: "qr_image_unreadable",
            message: "最新图片中未识别到可用二维码。",
            retryable: false,
        },
        QrError::Clipboard(_) => QrCommandErrorDto {
            code: "qr_history_unavailable",
            message: "二维码结果未能保存到内置剪贴板，请重试。",
            retryable: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversion_messages_distinguish_internal_success_from_system_sync() {
        assert_eq!(
            conversion_dto(QrConversionResult {
                kind: QrConversionKind::TextToImage,
                system_clipboard_synced: false,
            })
            .message,
            "已生成二维码并保存到历史；系统剪贴板同步失败。"
        );
        assert_eq!(
            map_error(QrError::NoLatestItem).code,
            "clipboard_history_empty"
        );
    }
}
