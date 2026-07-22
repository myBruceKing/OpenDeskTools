use serde::Serialize;
use tauri::{AppHandle, Runtime, State};

use crate::infrastructure::application::ApplicationRuntime;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralViewModel {
    version: String,
    autostart_enabled: bool,
    start_minimized: bool,
    close_to_tray: bool,
    crash_diagnostics_enabled: bool,
    data_directory: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataDirectoryMigrationResult {
    data_directory: String,
    restart_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralCommandError {
    code: &'static str,
    message: String,
}

#[tauri::command]
pub fn get_general_settings<R: Runtime>(
    app: AppHandle<R>,
    runtime: State<'_, ApplicationRuntime>,
) -> GeneralViewModel {
    current_view_model(&app, &runtime)
}

#[tauri::command]
pub fn set_autostart_enabled<R: Runtime>(
    app: AppHandle<R>,
    runtime: State<'_, ApplicationRuntime>,
    enabled: bool,
) -> Result<GeneralViewModel, GeneralCommandError> {
    runtime
        .autostart()
        .set(enabled)
        .map_err(|error| GeneralCommandError {
            code: "autostart_update_failed",
            message: format!("开机自启设置未生效：{error}"),
        })?;
    Ok(current_view_model(&app, &runtime))
}

#[tauri::command]
pub fn set_start_minimized<R: Runtime>(
    app: AppHandle<R>,
    runtime: State<'_, ApplicationRuntime>,
    enabled: bool,
) -> Result<GeneralViewModel, GeneralCommandError> {
    runtime
        .set_start_minimized(enabled)
        .map_err(|error| GeneralCommandError {
            code: "start_minimized_update_failed",
            message: format!("启动行为设置未保存：{error}"),
        })?;
    Ok(current_view_model(&app, &runtime))
}

#[tauri::command]
pub fn set_close_to_tray<R: Runtime>(
    app: AppHandle<R>,
    runtime: State<'_, ApplicationRuntime>,
    enabled: bool,
) -> Result<GeneralViewModel, GeneralCommandError> {
    runtime
        .set_close_to_tray(enabled)
        .map_err(|error| GeneralCommandError {
            code: "close_to_tray_update_failed",
            message: format!("关闭行为设置未保存：{error}"),
        })?;
    Ok(current_view_model(&app, &runtime))
}

#[tauri::command]
pub fn set_crash_diagnostics_enabled<R: Runtime>(
    app: AppHandle<R>,
    runtime: State<'_, ApplicationRuntime>,
    enabled: bool,
) -> Result<GeneralViewModel, GeneralCommandError> {
    runtime
        .set_crash_diagnostics_enabled(enabled)
        .map_err(|error| GeneralCommandError {
            code: "crash_diagnostics_update_failed",
            message: format!("本地崩溃日志设置未保存：{error}"),
        })?;
    Ok(current_view_model(&app, &runtime))
}

#[tauri::command]
pub fn select_and_migrate_data_directory(
    runtime: State<'_, ApplicationRuntime>,
) -> Result<Option<DataDirectoryMigrationResult>, GeneralCommandError> {
    #[cfg(windows)]
    let Some(directory) = rfd::FileDialog::new()
        .set_title("选择新的 OpenDeskTools 数据目录")
        .pick_folder()
    else {
        return Ok(None);
    };
    #[cfg(not(windows))]
    let directory = {
        return Err(GeneralCommandError {
            code: "data_directory_selection_unavailable",
            message: "当前平台不支持选择数据目录".to_owned(),
        });
    };
    let copied =
        runtime
            .migrate_data_directory(directory)
            .map_err(|error| GeneralCommandError {
                code: "data_directory_migration_failed",
                message: format!("数据目录迁移未完成：{error}"),
            })?;
    Ok(Some(DataDirectoryMigrationResult {
        data_directory: display_data_directory(&copied),
        restart_required: true,
    }))
}

fn current_view_model<R: Runtime>(
    app: &AppHandle<R>,
    runtime: &ApplicationRuntime,
) -> GeneralViewModel {
    build_view_model(
        app.package_info().version.to_string(),
        runtime.autostart().is_enabled().unwrap_or(false),
        runtime.start_minimized(),
        runtime.close_to_tray(),
        runtime.crash_diagnostics_enabled(),
        display_data_directory(runtime.storage().data_root()),
    )
}

fn build_view_model(
    version: String,
    autostart_enabled: bool,
    start_minimized: bool,
    close_to_tray: bool,
    crash_diagnostics_enabled: bool,
    data_directory: String,
) -> GeneralViewModel {
    GeneralViewModel {
        version,
        autostart_enabled,
        start_minimized,
        close_to_tray,
        crash_diagnostics_enabled,
        data_directory,
    }
}

/// Presents the resolved data root without the Windows `\\?\` verbatim prefix
/// that `fs::canonicalize` introduces, so the settings page shows a familiar
/// path and Explorer can open it.
fn display_data_directory(path: &std::path::Path) -> String {
    let text = path.to_string_lossy();
    text.strip_prefix(r"\\?\")
        .unwrap_or(text.as_ref())
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_model_carries_every_general_preference() {
        let view_model = build_view_model(
            "1.2.3".to_owned(),
            true,
            true,
            false,
            true,
            r"C:\OpenDeskToolsTestData\com.opendesktools.app".to_owned(),
        );

        assert_eq!(view_model.version, "1.2.3");
        assert!(view_model.autostart_enabled);
        assert!(view_model.start_minimized);
        assert!(!view_model.close_to_tray);
        assert!(view_model.crash_diagnostics_enabled);
        assert_eq!(
            view_model.data_directory,
            r"C:\OpenDeskToolsTestData\com.opendesktools.app"
        );
    }

    #[test]
    fn display_data_directory_strips_the_verbatim_prefix() {
        assert_eq!(
            display_data_directory(std::path::Path::new(
                r"\\?\C:\OpenDeskToolsTestData\com.opendesktools.app"
            )),
            r"C:\OpenDeskToolsTestData\com.opendesktools.app"
        );
        assert_eq!(
            display_data_directory(std::path::Path::new("/var/tmp/odt")),
            "/var/tmp/odt"
        );
    }
}
