use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};

use crate::infrastructure::application::ApplicationRuntime;
use crate::infrastructure::image_output::{save_rgba_with_dialog, ImageSaveOutcome};
use crate::infrastructure::pin_image::PinImageError;
use crate::infrastructure::qr::QrError;
use crate::infrastructure::screenshot::overlay::CaptureAction;
use crate::infrastructure::screenshot::service::{
    ScreenshotCaptureOutcome, ScreenshotServiceError,
};
use crate::infrastructure::screenshot::ScreenshotError;
use crate::infrastructure::usage_statistics::UsageAction;
use crate::{clipboard_history_event_sink, record_usage_success};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotCaptureDto {
    pub(crate) status: &'static str,
    pub(crate) width: Option<u32>,
    pub(crate) height: Option<u32>,
    pub(crate) message: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinImageDto {
    pub(crate) pin_id: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) message: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureCommandErrorDto {
    pub(crate) code: &'static str,
    pub(crate) message: &'static str,
    retryable: bool,
}

#[tauri::command]
pub async fn capture_screenshot<R: Runtime>(
    app: AppHandle<R>,
) -> Result<ScreenshotCaptureDto, CaptureCommandErrorDto> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let runtime = worker_app
            .try_state::<ApplicationRuntime>()
            .ok_or_else(internal_command_error)?;
        capture_and_notify(&worker_app, &runtime)
    })
    .await
    .map_err(|_| internal_command_error())?
}

pub(crate) fn capture_and_notify<R: Runtime>(
    app: &AppHandle<R>,
    runtime: &ApplicationRuntime,
) -> Result<ScreenshotCaptureDto, CaptureCommandErrorDto> {
    let result = runtime
        .screenshot()
        .capture_selection()
        .map_err(map_screenshot_error)?;
    match result {
        ScreenshotCaptureOutcome::Cancelled => Ok(ScreenshotCaptureDto {
            status: "cancelled",
            width: None,
            height: None,
            message: "已取消截图。",
        }),
        ScreenshotCaptureOutcome::Selected { image, action } => {
            if action == CaptureAction::Cancel {
                return Ok(ScreenshotCaptureDto {
                    status: "cancelled",
                    width: None,
                    height: None,
                    message: "已取消截图。",
                });
            }
            let (status, message) = match action {
                CaptureAction::Copy | CaptureAction::Finish => {
                    runtime
                        .screenshot()
                        .copy_image(&image, |sequence| {
                            runtime.clipboard_listener().suppress_sequence(sequence)
                        })
                        .map_err(map_screenshot_error)?;
                    ("copied", "截图已保存到内置历史并复制到系统剪贴板。")
                }
                CaptureAction::Save => {
                    match save_rgba_with_dialog(
                        "OpenDeskTools-截图.png",
                        image.width,
                        image.height,
                        &image.rgba,
                    ) {
                        Ok(ImageSaveOutcome::Saved(_)) => {}
                        Ok(ImageSaveOutcome::Cancelled) => {
                            return Ok(ScreenshotCaptureDto {
                                status: "cancelled",
                                width: None,
                                height: None,
                                message: "已取消保存截图。",
                            });
                        }
                        Err(_) => {
                            return Err(CaptureCommandErrorDto {
                                code: "capture_save_failed",
                                message: "截图保存失败，请检查目标位置后重试。",
                                retryable: true,
                            });
                        }
                    }
                    ("saved", "截图已保存为 PNG，并加入内置历史。")
                }
                CaptureAction::Pin => {
                    runtime
                        .pin_image()
                        .pin_rgba(image.width, image.height, image.rgba.clone())
                        .map_err(map_pin_error)?;
                    ("pinned", "截图已贴到屏幕，并加入内置历史。")
                }
                CaptureAction::DecodeQr => {
                    runtime
                        .qr()
                        .decode_image(image.width, image.height, image.rgba.clone(), |sequence| {
                            runtime.clipboard_listener().suppress_sequence(sequence)
                        })
                        .map_err(map_capture_qr_error)?;
                    ("qrDecoded", "已识别截图中的二维码并复制结果。")
                }
                CaptureAction::Cancel => unreachable!(),
            };
            let history_retained = runtime
                .screenshot()
                .record_image(&image)
                .map_err(map_screenshot_error)?;
            if history_retained {
                clipboard_history_event_sink(app)();
            }
            record_usage_success(app, runtime, UsageAction::ScreenshotCapture);
            Ok(ScreenshotCaptureDto {
                status,
                width: Some(image.width),
                height: Some(image.height),
                message,
            })
        }
    }
}

#[tauri::command]
pub async fn pin_latest_image<R: Runtime>(
    app: AppHandle<R>,
) -> Result<PinImageDto, CaptureCommandErrorDto> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let runtime = worker_app
            .try_state::<ApplicationRuntime>()
            .ok_or_else(internal_command_error)?;
        pin_latest_and_record(&worker_app, &runtime)
    })
    .await
    .map_err(|_| internal_command_error())?
}

pub(crate) fn pin_latest_and_record<R: Runtime>(
    app: &AppHandle<R>,
    runtime: &ApplicationRuntime,
) -> Result<PinImageDto, CaptureCommandErrorDto> {
    let outcome = runtime.pin_image().pin_latest().map_err(map_pin_error)?;
    record_usage_success(app, runtime, UsageAction::PinImage);
    Ok(PinImageDto {
        pin_id: outcome.pin_id.to_string(),
        width: outcome.width,
        height: outcome.height,
        message: "已贴出最新的内部剪贴板图片或文字。",
    })
}

fn map_screenshot_error(error: ScreenshotServiceError) -> CaptureCommandErrorDto {
    match error {
        ScreenshotServiceError::Screenshot(ScreenshotError::SessionAlreadyActive) => {
            CaptureCommandErrorDto {
                code: "capture_busy",
                message: "已有截图选区正在进行。",
                retryable: false,
            }
        }
        ScreenshotServiceError::Writer(_) => CaptureCommandErrorDto {
            code: "capture_clipboard_failed",
            message: "截图已生成，但系统剪贴板写入失败。",
            retryable: true,
        },
        ScreenshotServiceError::Clipboard(_) => CaptureCommandErrorDto {
            code: "capture_history_failed",
            message: "截图未能保存到内置历史，请重试。",
            retryable: true,
        },
        ScreenshotServiceError::Screenshot(_) => CaptureCommandErrorDto {
            code: "capture_unavailable",
            message: "当前桌面暂时无法完成区域截图，请重试。",
            retryable: true,
        },
    }
}

fn map_capture_qr_error(error: QrError) -> CaptureCommandErrorDto {
    match error {
        QrError::UnreadableImage | QrError::NonTextPayload => CaptureCommandErrorDto {
            code: "capture_qr_unreadable",
            message: "选区中没有识别到可用二维码，请调整选区后重试。",
            retryable: false,
        },
        QrError::Clipboard(_) => CaptureCommandErrorDto {
            code: "capture_qr_history_failed",
            message: "二维码已识别，但结果未能保存，请重试。",
            retryable: true,
        },
        _ => CaptureCommandErrorDto {
            code: "capture_qr_failed",
            message: "当前选区无法识别为二维码。",
            retryable: false,
        },
    }
}

fn map_pin_error(error: PinImageError) -> CaptureCommandErrorDto {
    match error {
        PinImageError::ImageUnavailable => CaptureCommandErrorDto {
            code: "pin_image_unavailable",
            message: "剪贴板历史中没有可贴出的图片或文字。",
            retryable: false,
        },
        PinImageError::Clipboard(_) => CaptureCommandErrorDto {
            code: "pin_image_history_failed",
            message: "读取剪贴板内容失败，请稍后重试。",
            retryable: true,
        },
        PinImageError::NativeSurface(_) => CaptureCommandErrorDto {
            code: "pin_image_surface_failed",
            message: "贴图窗口暂时不可用，请稍后重试。",
            retryable: true,
        },
    }
}

fn internal_command_error() -> CaptureCommandErrorDto {
    CaptureCommandErrorDto {
        code: "capture_worker_failed",
        message: "后台任务意外结束，请重试。",
        retryable: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_and_pin_errors_keep_stable_safe_messages() {
        assert_eq!(
            map_screenshot_error(ScreenshotServiceError::Screenshot(
                ScreenshotError::SessionAlreadyActive
            ))
            .code,
            "capture_busy"
        );
        assert_eq!(
            map_pin_error(PinImageError::ImageUnavailable).code,
            "pin_image_unavailable"
        );
    }
}
