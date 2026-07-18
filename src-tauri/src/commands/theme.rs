use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::infrastructure::application::ApplicationRuntime;
use crate::infrastructure::theme::{
    AccentColor, AnimationSpeed, ThemeMode, ThemePreferencesPatch, ThemeSnapshot,
    ThemeValidationError,
};

pub const THEME_CHANGED_EVENT: &str = "theme://changed";

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThemePreferencesPatchInput {
    expected_revision: u64,
    mode: Option<String>,
    accent: Option<String>,
    animation_speed: Option<String>,
    reduce_transparency: Option<bool>,
}

impl TryFrom<ThemePreferencesPatchInput> for ThemePreferencesPatch {
    type Error = ThemeValidationError;

    fn try_from(input: ThemePreferencesPatchInput) -> Result<Self, Self::Error> {
        let patch = Self {
            mode: input.mode.as_deref().map(ThemeMode::parse).transpose()?,
            accent: input
                .accent
                .as_deref()
                .map(AccentColor::parse)
                .transpose()?,
            animation_speed: input
                .animation_speed
                .as_deref()
                .map(AnimationSpeed::parse)
                .transpose()?,
            reduce_transparency: input.reduce_transparency,
        };
        if patch.is_empty() {
            return Err(ThemeValidationError::EmptyPatch);
        }
        Ok(patch)
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeCommandErrorDto {
    code: &'static str,
    message: &'static str,
    field: Option<&'static str>,
    retryable: bool,
    applied: bool,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeBroadcastWarningDto {
    code: &'static str,
    message: &'static str,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeUpdateResultDto {
    #[serde(flatten)]
    snapshot: ThemeSnapshot,
    broadcast_warning: Option<ThemeBroadcastWarningDto>,
}

#[tauri::command]
pub fn get_theme_preferences(
    runtime: State<'_, ApplicationRuntime>,
) -> Result<ThemeSnapshot, ThemeCommandErrorDto> {
    get_preferences(&runtime)
}

#[tauri::command]
pub fn update_theme_preferences(
    app: AppHandle,
    runtime: State<'_, ApplicationRuntime>,
    patch: ThemePreferencesPatchInput,
) -> Result<ThemeUpdateResultDto, ThemeCommandErrorDto> {
    update_preferences_with_broadcast(&runtime, patch, |updated| {
        app.emit(THEME_CHANGED_EVENT, updated)
            .map_err(|error| error.to_string())
    })
}

fn get_preferences(runtime: &ApplicationRuntime) -> Result<ThemeSnapshot, ThemeCommandErrorDto> {
    runtime
        .theme()
        .current()
        .map_err(|_error| ThemeCommandErrorDto {
            code: "theme_unavailable",
            message: "Theme preferences are temporarily unavailable.",
            field: None,
            retryable: true,
            applied: false,
        })
}

fn update_preferences_with_broadcast<F>(
    runtime: &ApplicationRuntime,
    input: ThemePreferencesPatchInput,
    broadcast: F,
) -> Result<ThemeUpdateResultDto, ThemeCommandErrorDto>
where
    F: FnOnce(&ThemeSnapshot) -> Result<(), String>,
{
    let expected_revision = input.expected_revision;
    let patch = ThemePreferencesPatch::try_from(input).map_err(validation_error)?;
    let updated = runtime
        .theme()
        .update(expected_revision, patch)
        .map_err(theme_update_error)?;

    let broadcast_warning = broadcast(&updated).err().map(|_| ThemeBroadcastWarningDto {
        code: "theme_broadcast_failed",
        message: "Theme saved, but some windows may not update immediately.",
    });
    Ok(ThemeUpdateResultDto {
        snapshot: updated,
        broadcast_warning,
    })
}

fn validation_error(error: ThemeValidationError) -> ThemeCommandErrorDto {
    let (message, field) = match error {
        ThemeValidationError::InvalidThemeMode(_) => ("Unsupported theme mode.", Some("mode")),
        ThemeValidationError::InvalidAccent(_) => ("Unsupported accent color.", Some("accent")),
        ThemeValidationError::InvalidAnimationSpeed(_) => {
            ("Unsupported animation speed.", Some("animationSpeed"))
        }
        ThemeValidationError::EmptyPatch => (
            "Theme update must include at least one field.",
            Some("patch"),
        ),
    };
    ThemeCommandErrorDto {
        code: "invalid_theme_preferences",
        message,
        field,
        retryable: false,
        applied: false,
    }
}

fn theme_update_error(error: crate::infrastructure::theme::ThemeError) -> ThemeCommandErrorDto {
    if matches!(
        error,
        crate::infrastructure::theme::ThemeError::RevisionConflict { .. }
    ) {
        return ThemeCommandErrorDto {
            code: "theme_revision_conflict",
            message: "Theme preferences changed; reload and retry.",
            field: Some("expectedRevision"),
            retryable: true,
            applied: false,
        };
    }

    ThemeCommandErrorDto {
        code: "theme_update_failed",
        message: "Unable to save theme preferences.",
        field: None,
        retryable: true,
        applied: false,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use tauri::ipc::{InvokeResponseBody, IpcResponse};
    use tempfile::tempdir;

    use super::*;

    fn runtime() -> (tempfile::TempDir, ApplicationRuntime) {
        let temp = tempdir().unwrap();
        let runtime = ApplicationRuntime::from_app_data_dir(temp.path().join("app-data")).unwrap();
        (temp, runtime)
    }

    #[test]
    fn get_command_returns_full_default_snapshot_serialization_contract() {
        let (_temp, runtime) = runtime();
        let response = get_preferences(&runtime)
            .unwrap()
            .body()
            .expect("theme snapshot should serialize");

        let InvokeResponseBody::Json(json) = response else {
            panic!("theme snapshot must serialize to JSON");
        };
        assert_eq!(
            json,
            r##"{"mode":"light","accent":"#216bd9","animationSpeed":"normal","reduceTransparency":false,"revision":0}"##
        );
    }

    #[test]
    fn partial_update_persists_then_broadcasts_full_snapshot() {
        let (_temp, runtime) = runtime();
        let broadcast = Mutex::new(None);
        let input = ThemePreferencesPatchInput {
            expected_revision: 0,
            mode: Some("dark".to_owned()),
            accent: Some("#7955c7".to_owned()),
            ..ThemePreferencesPatchInput::default()
        };

        let updated = update_preferences_with_broadcast(&runtime, input, |snapshot| {
            *broadcast.lock().unwrap() = Some(snapshot.clone());
            Ok(())
        })
        .unwrap();

        assert_eq!(
            broadcast.into_inner().unwrap(),
            Some(updated.snapshot.clone())
        );
        assert_eq!(runtime.theme().current().unwrap(), updated.snapshot);
        assert_eq!(
            updated.snapshot.preferences.animation_speed,
            AnimationSpeed::Normal
        );
        assert!(!updated.snapshot.preferences.reduce_transparency);
        assert_eq!(updated.snapshot.revision, 1);
        assert_eq!(updated.broadcast_warning, None);
        assert_eq!(THEME_CHANGED_EVENT, "theme://changed");
    }

    #[test]
    fn event_delivery_failure_does_not_turn_committed_update_into_command_failure() {
        let (_temp, runtime) = runtime();
        let input = ThemePreferencesPatchInput {
            expected_revision: 0,
            mode: Some("dark".to_owned()),
            ..ThemePreferencesPatchInput::default()
        };

        let updated = update_preferences_with_broadcast(&runtime, input, |_| {
            Err("receiver unavailable".to_owned())
        })
        .expect("a committed update must remain successful");

        assert_eq!(runtime.theme().current().unwrap(), updated.snapshot);
        assert_eq!(updated.snapshot.revision, 1);
        assert_eq!(
            updated.broadcast_warning,
            Some(ThemeBroadcastWarningDto {
                code: "theme_broadcast_failed",
                message: "Theme saved, but some windows may not update immediately."
            })
        );
        let response = updated
            .body()
            .expect("update result with warning should serialize");
        let InvokeResponseBody::Json(json) = response else {
            panic!("update result must serialize to JSON");
        };
        assert_eq!(
            json,
            r##"{"mode":"dark","accent":"#216bd9","animationSpeed":"normal","reduceTransparency":false,"revision":1,"broadcastWarning":{"code":"theme_broadcast_failed","message":"Theme saved, but some windows may not update immediately."}}"##
        );
    }

    #[test]
    fn invalid_and_empty_patches_return_serializable_error_without_broadcast() {
        let (_temp, runtime) = runtime();
        let broadcast_count = Mutex::new(0_u32);
        let input = ThemePreferencesPatchInput {
            expected_revision: 0,
            accent: Some("#ffffff".to_owned()),
            ..ThemePreferencesPatchInput::default()
        };

        let error = update_preferences_with_broadcast(&runtime, input, |_| {
            *broadcast_count.lock().unwrap() += 1;
            Ok(())
        })
        .unwrap_err();

        assert_eq!(*broadcast_count.lock().unwrap(), 0);
        assert_eq!(error.code, "invalid_theme_preferences");
        assert_eq!(error.field, Some("accent"));
        assert!(!error.retryable);
        assert!(!error.applied);
        let response = error.body().expect("command error should serialize");
        let InvokeResponseBody::Json(json) = response else {
            panic!("command error must serialize to JSON");
        };
        assert_eq!(
            json,
            r#"{"code":"invalid_theme_preferences","message":"Unsupported accent color.","field":"accent","retryable":false,"applied":false}"#
        );

        let empty_error = update_preferences_with_broadcast(
            &runtime,
            ThemePreferencesPatchInput {
                expected_revision: 0,
                ..ThemePreferencesPatchInput::default()
            },
            |_| Ok(()),
        )
        .unwrap_err();
        assert_eq!(
            empty_error.message,
            "Theme update must include at least one field."
        );
    }

    #[test]
    fn stale_revision_returns_safe_retryable_error_contract() {
        let (_temp, runtime) = runtime();
        let first = ThemePreferencesPatchInput {
            expected_revision: 0,
            mode: Some("dark".to_owned()),
            ..ThemePreferencesPatchInput::default()
        };
        update_preferences_with_broadcast(&runtime, first, |_| Ok(())).unwrap();

        let stale = ThemePreferencesPatchInput {
            expected_revision: 0,
            mode: Some("system".to_owned()),
            ..ThemePreferencesPatchInput::default()
        };
        let error = update_preferences_with_broadcast(&runtime, stale, |_| Ok(())).unwrap_err();

        assert_eq!(error.code, "theme_revision_conflict");
        assert_eq!(error.field, Some("expectedRevision"));
        assert!(error.retryable);
        assert!(!error.applied);
        assert_eq!(runtime.theme().current().unwrap().revision, 1);
    }
}
