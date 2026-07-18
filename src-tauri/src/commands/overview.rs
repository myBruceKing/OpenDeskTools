use serde::Serialize;
use tauri::{AppHandle, State};

use crate::infrastructure::application::{ApplicationRuntime, ApplicationStatus, StartupMode};

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
    today_triggers: Option<u64>,
    week_triggers: Option<u64>,
    month_triggers: Option<u64>,
    saved_minutes_this_month: Option<u64>,
}

#[tauri::command]
pub fn get_overview_view_model(
    app: AppHandle,
    runtime: State<'_, ApplicationRuntime>,
) -> OverviewViewModel {
    build_view_model(&runtime, app.package_info().version.to_string())
}

fn build_view_model(runtime: &ApplicationRuntime, version: String) -> OverviewViewModel {
    OverviewViewModel {
        version,
        service_state: match runtime.status() {
            ApplicationStatus::Running => "running",
        },
        startup_enabled: match runtime.startup_mode() {
            StartupMode::Manual => false,
        },
        hotkeys: None,
        statistics: None,
    }
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

        let view_model = build_view_model(&runtime, "1.2.3".to_owned());

        assert_eq!(
            view_model,
            OverviewViewModel {
                version: "1.2.3".to_owned(),
                service_state: "running",
                startup_enabled: false,
                hotkeys: None,
                statistics: None,
            }
        );
    }

    #[test]
    fn view_model_serializes_the_frontend_contract_with_null_unimplemented_data() {
        let (_temp, runtime) = test_runtime();
        let response = build_view_model(&runtime, "0.1.0".to_owned())
            .body()
            .expect("overview view model should serialize");

        let InvokeResponseBody::Json(json) = response else {
            panic!("overview view model must serialize to JSON");
        };

        assert_eq!(
            json,
            r#"{"version":"0.1.0","serviceState":"running","startupEnabled":false,"hotkeys":null,"statistics":null}"#
        );
    }
}
