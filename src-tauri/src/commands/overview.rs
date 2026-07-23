use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::infrastructure::application::{ApplicationRuntime, ApplicationStatus};
use crate::infrastructure::hotkey::HotkeyRuntimeState;

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverviewViewModel {
    version: String,
    service_state: &'static str,
    startup_enabled: bool,
    hotkeys: Option<Vec<OverviewHotkeyViewModel>>,
    statistics: Option<OverviewStatistics>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverviewHotkeyViewModel {
    id: String,
    binding: Option<String>,
    enabled: Option<bool>,
    state: Option<String>,
    detail: Option<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverviewStatistics {
    today_triggers: u64,
    week_triggers: u64,
    month_triggers: u64,
    saved_seconds_this_month: u64,
}

#[tauri::command]
pub async fn get_overview_view_model(app: AppHandle) -> Result<OverviewViewModel, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(runtime) = app.try_state::<ApplicationRuntime>() {
                if runtime.is_startup_ready() {
                    return build_view_model(
                        &runtime,
                        app.package_info().version.to_string(),
                        runtime.autostart().is_enabled().unwrap_or(false),
                    );
                }
            }
            if Instant::now() >= deadline {
                return Err("应用启动尚未完成，概览数据暂不可用。".to_owned());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    })
    .await
    .map_err(|error| format!("概览读取任务异常结束：{error}"))?
}

fn build_view_model(
    runtime: &ApplicationRuntime,
    version: String,
    startup_enabled: bool,
) -> Result<OverviewViewModel, String> {
    let hotkeys = runtime
        .hotkeys()
        .snapshot()
        .map_err(|error| format!("无法读取快捷键快照：{error}"))?
        .actions
        .into_iter()
        .map(|hotkey| OverviewHotkeyViewModel {
            id: match hotkey.action_id {
                crate::infrastructure::hotkey::HotkeyActionId::ScreenshotCapture => "capture",
                crate::infrastructure::hotkey::HotkeyActionId::ClipboardPinImage => "pinImage",
                crate::infrastructure::hotkey::HotkeyActionId::ClipboardQrConvert => "clipboardQr",
                crate::infrastructure::hotkey::HotkeyActionId::LauncherOpen => "toolWheel",
                crate::infrastructure::hotkey::HotkeyActionId::ClipboardOpenPanel => {
                    "clipboardPanel"
                }
            }
            .to_owned(),
            binding: Some(hotkey.binding),
            enabled: Some(hotkey.configured_enabled),
            state: Some(
                match hotkey.runtime_state {
                    HotkeyRuntimeState::Registered => "normal",
                    HotkeyRuntimeState::Conflict => "conflict",
                    HotkeyRuntimeState::Disabled
                    | HotkeyRuntimeState::Unavailable
                    | HotkeyRuntimeState::Degraded => "unavailable",
                }
                .to_owned(),
            ),
            detail: hotkey.detail,
        })
        .collect();
    let statistics = runtime
        .usage_statistics()
        .snapshot()
        .map_err(|error| format!("无法读取使用统计：{error}"))?;

    Ok(OverviewViewModel {
        version,
        service_state: match runtime.status() {
            ApplicationStatus::Running => "running",
        },
        startup_enabled,
        hotkeys: Some(hotkeys),
        statistics: Some(OverviewStatistics {
            today_triggers: statistics.today_triggers,
            week_triggers: statistics.week_triggers,
            month_triggers: statistics.month_triggers,
            saved_seconds_this_month: statistics.saved_seconds_this_month,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::ipc::{InvokeResponseBody, IpcResponse};
    use tempfile::{tempdir, TempDir};

    fn test_runtime() -> (TempDir, ApplicationRuntime) {
        let temp = tempdir().expect("temporary directory should be created");
        let runtime = ApplicationRuntime::from_app_data_dir(temp.path().join("app-data"))
            .expect("test runtime storage should initialize");
        (temp, runtime)
    }

    #[test]
    fn view_model_uses_runtime_state_and_supplied_package_version() {
        let (_temp, runtime) = test_runtime();

        let view_model = build_view_model(&runtime, "1.2.3".to_owned(), false).unwrap();

        assert_eq!(view_model.version, "1.2.3");
        assert_eq!(view_model.service_state, "running");
        assert!(!view_model.startup_enabled);
        assert_eq!(
            view_model.statistics,
            Some(OverviewStatistics {
                today_triggers: 0,
                week_triggers: 0,
                month_triggers: 0,
                saved_seconds_this_month: 0,
            })
        );
        let hotkeys = view_model
            .hotkeys
            .expect("hotkeys should be real runtime data");
        assert_eq!(hotkeys.len(), 5);
        assert_eq!(hotkeys[0].id, "capture");
        assert_eq!(hotkeys[0].binding.as_deref(), Some("F1"));
        assert_eq!(hotkeys[0].enabled, Some(true));
        assert_eq!(hotkeys[0].state.as_deref(), Some("unavailable"));
        assert_eq!(hotkeys[4].id, "clipboardPanel");
        assert!(hotkeys[4]
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("Windows")));
    }

    #[test]
    fn view_model_serializes_real_hotkeys_with_the_legacy_overview_ids() {
        let (_temp, runtime) = test_runtime();
        let response = build_view_model(&runtime, "0.1.0".to_owned(), false)
            .unwrap()
            .body()
            .expect("overview view model should serialize");

        let InvokeResponseBody::Json(json) = response else {
            panic!("overview view model must serialize to JSON");
        };

        let json: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(json["version"], "0.1.0");
        assert_eq!(json["serviceState"], "running");
        assert_eq!(json["startupEnabled"], false);
        let hotkeys = json["hotkeys"].as_array().unwrap();
        assert_eq!(hotkeys.len(), 5);
        assert_eq!(hotkeys[0]["id"], "capture");
        assert_eq!(hotkeys[1]["id"], "pinImage");
        assert_eq!(hotkeys[2]["id"], "clipboardQr");
        assert_eq!(hotkeys[3]["id"], "toolWheel");
        assert_eq!(hotkeys[4]["id"], "clipboardPanel");
        assert_eq!(hotkeys[4]["state"], "unavailable");
        assert_eq!(json["statistics"]["todayTriggers"], 0);
        assert_eq!(json["statistics"]["savedSecondsThisMonth"], 0);
    }
}
