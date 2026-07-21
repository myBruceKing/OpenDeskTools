use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use crate::infrastructure::application::ApplicationRuntime;
use crate::infrastructure::hotkey::{
    classify_binding, HotkeyActionId, HotkeyBinding, HotkeyBindingClassification, HotkeyError,
    HotkeySnapshot, HotkeyValidationError, TauriHotkeyRegistrar, UpdateHotkeyBinding,
};
use crate::infrastructure::hotkey_capture::{
    HotkeyCaptureError, HotkeyCaptureSession, HotkeyCaptureStopResult,
};

const HOTKEY_CAPTURE_EVENT: &str = "hotkey://capture-token";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateHotkeyBindingPatch {
    action_id: String,
    expected_revision: u64,
    binding: String,
    force_override_system: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyBindingClassificationResponse {
    binding: String,
    normalized_binding: String,
    classification: HotkeyBindingClassification,
    message: String,
    force_override_allowed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyCommandError {
    code: &'static str,
    message: String,
    actual_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyCaptureCommandError {
    code: &'static str,
    message: String,
}

#[tauri::command]
pub fn start_hotkey_capture<R: Runtime>(
    app: AppHandle<R>,
    runtime: State<'_, ApplicationRuntime>,
) -> Result<HotkeyCaptureSession, HotkeyCaptureCommandError> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| capture_command_error("capture_window_unavailable"))?;
    #[cfg(windows)]
    let target_window = window
        .hwnd()
        .map_err(|_| capture_command_error("capture_window_unavailable"))?
        .0 as usize;
    #[cfg(not(windows))]
    let target_window = {
        let _ = window;
        0
    };
    let event_app = app.clone();
    runtime
        .hotkey_capture()
        .start(target_window, move |event| {
            let _ = event_app.emit(HOTKEY_CAPTURE_EVENT, event);
        })
        .map_err(map_capture_error)
}

#[tauri::command]
pub fn stop_hotkey_capture(
    runtime: State<'_, ApplicationRuntime>,
    session_id: String,
) -> Result<HotkeyCaptureStopResult, HotkeyCaptureCommandError> {
    runtime
        .hotkey_capture()
        .stop(&session_id)
        .map_err(map_capture_error)
}

#[tauri::command]
pub fn get_hotkey_snapshot(
    runtime: State<'_, ApplicationRuntime>,
) -> Result<HotkeySnapshot, HotkeyCommandError> {
    runtime.hotkeys().snapshot().map_err(map_error)
}

#[tauri::command]
pub fn classify_hotkey_binding(
    binding: String,
) -> Result<HotkeyBindingClassificationResponse, HotkeyCommandError> {
    let parsed = HotkeyBinding::parse(&binding).map_err(map_validation_error)?;
    let classification = classify_binding(&binding).map_err(map_validation_error)?;
    let message = match classification {
        HotkeyBindingClassification::Ordinary => "此快捷键可以保存。",
        HotkeyBindingClassification::SystemReserved => {
            "此组合由 Windows 使用，需要明确确认强制覆盖。"
        }
        HotkeyBindingClassification::Blocked => "此系统安全组合不能被接管。",
        HotkeyBindingClassification::UnsupportedSequence => "当前全局快捷键后端不支持连续按键。",
    }
    .to_owned();
    Ok(HotkeyBindingClassificationResponse {
        binding,
        normalized_binding: parsed.normalized(),
        classification,
        message,
        force_override_allowed: classification == HotkeyBindingClassification::SystemReserved,
    })
}

#[tauri::command]
pub fn update_hotkey_binding<R: Runtime>(
    app: AppHandle<R>,
    runtime: State<'_, ApplicationRuntime>,
    patch: UpdateHotkeyBindingPatch,
) -> Result<HotkeySnapshot, HotkeyCommandError> {
    let event_app = app.clone();
    let registrar = TauriHotkeyRegistrar::new(&app, runtime.keyboard_hook(), move |event| {
        crate::queue_forced_hotkey_event(&event_app, event)
    });
    let action_id = HotkeyActionId::parse(&patch.action_id).map_err(map_validation_error)?;
    let updated = runtime
        .hotkeys()
        .update_binding(
            UpdateHotkeyBinding {
                action_id,
                expected_revision: patch.expected_revision,
                binding: patch.binding,
                force_override_system: patch.force_override_system,
            },
            &registrar,
        )
        .map_err(map_error)?;
    runtime.ordinary_hotkey_latch().clear_action(action_id);
    Ok(updated)
}

fn map_error(error: HotkeyError) -> HotkeyCommandError {
    match error {
        HotkeyError::RevisionConflict { actual, .. } => HotkeyCommandError {
            code: "hotkey_revision_conflict",
            message: "快捷键设置已在其他位置更新，请刷新后重试。".to_owned(),
            actual_revision: Some(actual),
        },
        HotkeyError::Validation(error) => map_validation_error(error),
        HotkeyError::Storage(_) => HotkeyCommandError {
            code: "hotkey_storage_failed",
            message: "快捷键设置未能保存，请稍后重试。".to_owned(),
            actual_revision: None,
        },
        HotkeyError::StateLockPoisoned => HotkeyCommandError {
            code: "hotkey_state_unavailable",
            message: "快捷键服务暂时不可用，请重启应用后重试。".to_owned(),
            actual_revision: None,
        },
        HotkeyError::RevisionOverflow => HotkeyCommandError {
            code: "hotkey_revision_overflow",
            message: "快捷键设置版本已达到上限。".to_owned(),
            actual_revision: None,
        },
        HotkeyError::CorruptSettings { .. } => HotkeyCommandError {
            code: "hotkey_settings_corrupt",
            message: "快捷键配置已损坏，应用未覆盖原始数据。".to_owned(),
            actual_revision: None,
        },
    }
}

fn map_validation_error(error: HotkeyValidationError) -> HotkeyCommandError {
    let (code, message) = match error {
        HotkeyValidationError::ForceRequired => (
            "force_required",
            "此快捷键由 Windows 使用；必须明确确认强制覆盖。".to_owned(),
        ),
        HotkeyValidationError::ForceOverrideNotApplicable => (
            "force_override_not_applicable",
            "普通快捷键不能启用系统组合强制覆盖。".to_owned(),
        ),
        HotkeyValidationError::Blocked => (
            "blocked",
            "此系统安全快捷键不能被 OpenDeskTools 接管。".to_owned(),
        ),
        HotkeyValidationError::UnsupportedSequence => (
            "unsupported_sequence",
            "当前全局快捷键后端不支持连续按键。".to_owned(),
        ),
        HotkeyValidationError::UnknownAction(_) => {
            ("unknown_action", "无法识别快捷键对应的操作。".to_owned())
        }
        HotkeyValidationError::InvalidBinding(_) | HotkeyValidationError::UnsupportedKey(_) => {
            ("invalid_binding", "无法识别此快捷键组合。".to_owned())
        }
    };
    HotkeyCommandError {
        code,
        message,
        actual_revision: None,
    }
}

fn map_capture_error(error: HotkeyCaptureError) -> HotkeyCaptureCommandError {
    let code = match error {
        HotkeyCaptureError::Hook(
            crate::infrastructure::keyboard_hook::KeyboardHookError::HookInstall(_),
        ) => "capture_hook_unavailable",
        #[cfg(not(windows))]
        HotkeyCaptureError::Hook(
            crate::infrastructure::keyboard_hook::KeyboardHookError::UnsupportedPlatform,
        ) => "capture_unsupported_platform",
        HotkeyCaptureError::Hook(_) | HotkeyCaptureError::StateLockPoisoned => {
            "capture_service_unavailable"
        }
    };
    capture_command_error(code)
}

fn capture_command_error(code: &'static str) -> HotkeyCaptureCommandError {
    let message = match code {
        "capture_unsupported_platform" => "当前平台不支持原生快捷键捕获。",
        "capture_window_unavailable" => "主窗口暂时不可用，无法开始快捷键捕获。",
        "capture_hook_unavailable" => "无法启用系统快捷键捕获，请稍后重试。",
        _ => "快捷键捕获服务暂时不可用，请关闭弹窗后重试。",
    };
    HotkeyCaptureCommandError {
        code,
        message: message.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_response_is_normalized_and_marks_force_capability() {
        let response = classify_hotkey_binding("super + keyv".to_owned()).unwrap();

        assert_eq!(response.normalized_binding, "Win+V");
        assert_eq!(
            response.classification,
            HotkeyBindingClassification::SystemReserved
        );
        assert!(response.force_override_allowed);
    }

    #[test]
    fn validation_errors_are_safe_stable_codes() {
        assert_eq!(
            map_validation_error(HotkeyValidationError::ForceRequired).code,
            "force_required"
        );
        assert_eq!(
            map_validation_error(HotkeyValidationError::Blocked).code,
            "blocked"
        );
        assert_eq!(
            map_validation_error(HotkeyValidationError::UnsupportedSequence).code,
            "unsupported_sequence"
        );
        assert_eq!(
            HotkeyActionId::parse("removed.action")
                .map_err(map_validation_error)
                .unwrap_err()
                .code,
            "unknown_action"
        );
    }
}
