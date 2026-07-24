use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use crate::infrastructure::application::ApplicationRuntime;
use crate::infrastructure::disabled_hotkeys::{self, DisabledHotkeysOutcome};
use crate::infrastructure::hotkey::{
    classify_binding, HotkeyActionId, HotkeyBinding, HotkeyBindingClassification, HotkeyError,
    HotkeySnapshot, HotkeyValidationError, TauriHotkeyRegistrar, UpdateHotkeyBinding,
    UpdateHotkeyEnabled,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateHotkeyEnabledPatch {
    action_id: String,
    expected_revision: u64,
    enabled: bool,
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
pub struct SystemHotkeyNotice {
    binding: String,
    letter: String,
    restart_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateHotkeyBindingResponse {
    snapshot: HotkeySnapshot,
    system_hotkey_notice: Option<SystemHotkeyNotice>,
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
) -> Result<UpdateHotkeyBindingResponse, HotkeyCommandError> {
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
    let outcome = runtime.sync_system_hotkey_disable(&updated);
    let system_hotkey_notice = build_system_hotkey_notice(action_id, &updated, outcome.as_ref());
    Ok(UpdateHotkeyBindingResponse {
        snapshot: updated,
        system_hotkey_notice,
    })
}

#[tauri::command]
pub fn update_hotkey_enabled<R: Runtime>(
    app: AppHandle<R>,
    runtime: State<'_, ApplicationRuntime>,
    patch: UpdateHotkeyEnabledPatch,
) -> Result<UpdateHotkeyBindingResponse, HotkeyCommandError> {
    let event_app = app.clone();
    let registrar = TauriHotkeyRegistrar::new(&app, runtime.keyboard_hook(), move |event| {
        crate::queue_forced_hotkey_event(&event_app, event)
    });
    let action_id = HotkeyActionId::parse(&patch.action_id).map_err(map_validation_error)?;
    let updated = runtime
        .hotkeys()
        .update_enabled(
            UpdateHotkeyEnabled {
                action_id,
                expected_revision: patch.expected_revision,
                enabled: patch.enabled,
            },
            &registrar,
        )
        .map_err(map_error)?;
    runtime.ordinary_hotkey_latch().clear_action(action_id);
    let outcome = runtime.sync_system_hotkey_disable(&updated);
    let system_hotkey_notice = build_system_hotkey_notice(action_id, &updated, outcome.as_ref());
    Ok(UpdateHotkeyBindingResponse {
        snapshot: updated,
        system_hotkey_notice,
    })
}

/// Produces an Explorer-restart notice only when saving this action actually
/// added a bare `Win+<letter>` to the system `DisabledHotkeys` value. When the
/// letter was already disabled (no registry change) no notice is emitted.
fn build_system_hotkey_notice(
    action_id: HotkeyActionId,
    snapshot: &HotkeySnapshot,
    outcome: Option<&DisabledHotkeysOutcome>,
) -> Option<SystemHotkeyNotice> {
    let outcome = outcome?;
    if !outcome.changed {
        return None;
    }
    let action = snapshot
        .actions
        .iter()
        .find(|candidate| candidate.action_id == action_id)?;
    if !action.configured_enabled || !action.force_override_system || !action.action_available {
        return None;
    }
    let letter = disabled_hotkeys::win_single_letter(&action.binding)?;
    if !outcome.managed_letters.contains(&letter) {
        return None;
    }
    Some(SystemHotkeyNotice {
        binding: action.binding.clone(),
        letter: letter.to_string(),
        restart_required: true,
    })
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
        HotkeyError::StateLockPoisoned | HotkeyError::AvailabilityAlreadyReconciled => {
            HotkeyCommandError {
                code: "hotkey_state_unavailable",
                message: "快捷键服务暂时不可用，请重启应用后重试。".to_owned(),
                actual_revision: None,
            }
        }
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
    use crate::infrastructure::hotkey::{HotkeyRegistrationSnapshot, HotkeyRuntimeState};

    fn win_v_snapshot(
        configured_enabled: bool,
        force_override_system: bool,
        action_available: bool,
        binding: &str,
    ) -> HotkeySnapshot {
        HotkeySnapshot {
            revision: 1,
            actions: vec![HotkeyRegistrationSnapshot {
                action_id: HotkeyActionId::ClipboardOpenPanel,
                binding: binding.to_owned(),
                configured_enabled,
                force_override_system,
                action_available,
                classification: HotkeyBindingClassification::SystemReserved,
                runtime_state: HotkeyRuntimeState::Registered,
                runtime_backend: None,
                detail: None,
            }],
        }
    }

    fn outcome(changed: bool, managed: &str) -> DisabledHotkeysOutcome {
        DisabledHotkeysOutcome {
            changed,
            managed_letters: managed.chars().collect(),
            registry_value: managed.to_owned(),
        }
    }

    #[test]
    fn notice_emitted_only_when_registry_added_the_saved_win_letter() {
        let snapshot = win_v_snapshot(true, true, true, "Win+V");
        let notice = build_system_hotkey_notice(
            HotkeyActionId::ClipboardOpenPanel,
            &snapshot,
            Some(&outcome(true, "V")),
        )
        .expect("a changed registry with the managed letter should produce a notice");
        assert_eq!(notice.binding, "Win+V");
        assert_eq!(notice.letter, "V");
        assert!(notice.restart_required);
    }

    #[test]
    fn notice_suppressed_when_registry_unchanged_or_letter_not_managed() {
        let snapshot = win_v_snapshot(true, true, true, "Win+V");
        // No change means the letter was already disabled: no restart needed.
        assert!(build_system_hotkey_notice(
            HotkeyActionId::ClipboardOpenPanel,
            &snapshot,
            Some(&outcome(false, "V")),
        )
        .is_none());
        // Changed but the saved letter is not among the managed letters.
        assert!(build_system_hotkey_notice(
            HotkeyActionId::ClipboardOpenPanel,
            &snapshot,
            Some(&outcome(true, "R")),
        )
        .is_none());
        // No reconcile outcome at all (registry access failed).
        assert!(
            build_system_hotkey_notice(HotkeyActionId::ClipboardOpenPanel, &snapshot, None)
                .is_none()
        );
    }

    #[test]
    fn notice_suppressed_for_disabled_unforced_or_non_win_letter_bindings() {
        let managed = outcome(true, "V");
        // Disabled binding.
        assert!(build_system_hotkey_notice(
            HotkeyActionId::ClipboardOpenPanel,
            &win_v_snapshot(false, true, true, "Win+V"),
            Some(&managed),
        )
        .is_none());
        // Force override not requested.
        assert!(build_system_hotkey_notice(
            HotkeyActionId::ClipboardOpenPanel,
            &win_v_snapshot(true, false, true, "Win+V"),
            Some(&managed),
        )
        .is_none());
        // Not a bare Win+letter combination.
        assert!(build_system_hotkey_notice(
            HotkeyActionId::ClipboardOpenPanel,
            &win_v_snapshot(true, true, true, "Win+Shift+S"),
            Some(&outcome(true, "")),
        )
        .is_none());
    }

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
