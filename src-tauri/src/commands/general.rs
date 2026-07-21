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
    data_directory: String,
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
pub fn open_data_directory(
    runtime: State<'_, ApplicationRuntime>,
) -> Result<(), GeneralCommandError> {
    let directory = display_data_directory(runtime.storage().data_root());
    open_directory(&directory).map_err(|message| GeneralCommandError {
        code: "open_data_directory_failed",
        message,
    })
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
        display_data_directory(runtime.storage().data_root()),
    )
}

fn build_view_model(
    version: String,
    autostart_enabled: bool,
    start_minimized: bool,
    close_to_tray: bool,
    data_directory: String,
) -> GeneralViewModel {
    GeneralViewModel {
        version,
        autostart_enabled,
        start_minimized,
        close_to_tray,
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

#[cfg(windows)]
fn open_directory(directory: &str) -> Result<(), String> {
    // `explorer.exe` returns a non-zero exit code even on success, so we only
    // fail when the process cannot be spawned at all.
    std::process::Command::new("explorer")
        .arg(directory)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开数据目录：{error}"))
}

#[cfg(not(windows))]
fn open_directory(_directory: &str) -> Result<(), String> {
    Err("当前平台不支持打开数据目录".to_owned())
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
            r"C:\Users\me\AppData\Roaming\com.opendesktools.app".to_owned(),
        );

        assert_eq!(view_model.version, "1.2.3");
        assert!(view_model.autostart_enabled);
        assert!(view_model.start_minimized);
        assert!(!view_model.close_to_tray);
        assert_eq!(
            view_model.data_directory,
            r"C:\Users\me\AppData\Roaming\com.opendesktools.app"
        );
    }

    #[test]
    fn display_data_directory_strips_the_verbatim_prefix() {
        assert_eq!(
            display_data_directory(std::path::Path::new(
                r"\\?\C:\Users\me\AppData\Roaming\com.opendesktools.app"
            )),
            r"C:\Users\me\AppData\Roaming\com.opendesktools.app"
        );
        assert_eq!(
            display_data_directory(std::path::Path::new("/home/me/.local/share/odt")),
            "/home/me/.local/share/odt"
        );
    }
}
