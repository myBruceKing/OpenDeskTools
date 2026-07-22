use serde::{Deserialize, Serialize};
use tauri::{ipc::Response, AppHandle, Emitter, Manager, Runtime, State};

use crate::infrastructure::application::ApplicationRuntime;
use crate::infrastructure::quick_launch::{QuickLaunchError, QuickLaunchSnapshot, ToolMenuPreferences};

const QUICK_LAUNCH_CHANGED_EVENT: &str = "quick-launch://changed";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PathInput { path: String }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PinInput { path: String, source: Option<String> }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisibilityInput { path: String, visible: bool }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReorderInput { active_path: String, over_path: String }

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolMenuPreferencesInput { preferences: ToolMenuPreferences }

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickLaunchCommandError { code: &'static str, message: String }

#[tauri::command]
pub async fn get_quick_launch_snapshot(runtime: State<'_, ApplicationRuntime>) -> Result<QuickLaunchSnapshot, QuickLaunchCommandError> {
    run_background(runtime.quick_launch(), |service| service.snapshot()).await
}

#[tauri::command]
pub async fn rescan_quick_launch(app: AppHandle, runtime: State<'_, ApplicationRuntime>) -> Result<QuickLaunchSnapshot, QuickLaunchCommandError> {
    publish_snapshot(&app, run_background(runtime.quick_launch(), |service| service.rescan()).await)
}

#[tauri::command]
pub async fn pin_quick_launch_app(app: AppHandle, runtime: State<'_, ApplicationRuntime>, input: PinInput) -> Result<QuickLaunchSnapshot, QuickLaunchCommandError> {
    publish_snapshot(&app, run_background(runtime.quick_launch(), move |service| service.pin(input.path, input.source)).await)
}

#[tauri::command]
pub async fn unpin_quick_launch_app(app: AppHandle, runtime: State<'_, ApplicationRuntime>, input: PathInput) -> Result<QuickLaunchSnapshot, QuickLaunchCommandError> {
    publish_snapshot(&app, run_background(runtime.quick_launch(), move |service| service.unpin(&input.path)).await)
}

#[tauri::command]
pub async fn set_quick_launch_visible(app: AppHandle, runtime: State<'_, ApplicationRuntime>, input: VisibilityInput) -> Result<QuickLaunchSnapshot, QuickLaunchCommandError> {
    publish_snapshot(&app, run_background(runtime.quick_launch(), move |service| service.set_visible(&input.path, input.visible)).await)
}

#[tauri::command]
pub async fn reorder_quick_launch_apps(app: AppHandle, runtime: State<'_, ApplicationRuntime>, input: ReorderInput) -> Result<QuickLaunchSnapshot, QuickLaunchCommandError> {
    publish_snapshot(&app, run_background(runtime.quick_launch(), move |service| service.reorder(&input.active_path, &input.over_path)).await)
}

#[tauri::command]
pub async fn update_tool_menu_preferences(app: AppHandle, runtime: State<'_, ApplicationRuntime>, input: ToolMenuPreferencesInput) -> Result<QuickLaunchSnapshot, QuickLaunchCommandError> {
    publish_snapshot(&app, run_background(runtime.quick_launch(), move |service| service.update_tool_menu_preferences(input.preferences)).await)
}

#[tauri::command]
pub async fn launch_quick_launch_app(runtime: State<'_, ApplicationRuntime>, input: PathInput) -> Result<(), QuickLaunchCommandError> {
    run_background(runtime.quick_launch(), move |service| service.launch(&input.path)).await
}

#[tauri::command]
pub async fn get_quick_launch_icon(runtime: State<'_, ApplicationRuntime>, input: PathInput) -> Result<Response, QuickLaunchCommandError> {
    run_background(runtime.quick_launch(), move |service| service.icon_bytes(&input.path)).await.map(Response::new)
}

#[tauri::command]
pub fn select_quick_launch_app() -> Result<Option<String>, QuickLaunchCommandError> {
    #[cfg(windows)]
    {
        Ok(rfd::FileDialog::new().add_filter("程序或快捷方式", &["exe", "lnk"]).pick_file().map(|path| path.to_string_lossy().into_owned()))
    }
    #[cfg(not(windows))]
    { Err(QuickLaunchCommandError { code: "quick_launch_selection_unavailable", message: "当前平台不支持选择程序".to_owned() }) }
}

#[tauri::command]
pub fn close_tool_menu_surface<R: Runtime>(app: AppHandle<R>) -> Result<(), QuickLaunchCommandError> {
    crate::infrastructure::tool_menu_surface_window::request_hide(&app)
        .map_err(|error| QuickLaunchCommandError {
            code: "tool_menu_surface_unavailable",
            message: format!("无法关闭工具盘：{error}"),
        })
}

async fn run_background<T: Send + 'static>(
    service: std::sync::Arc<crate::infrastructure::quick_launch::QuickLaunchService>,
    operation: impl FnOnce(&crate::infrastructure::quick_launch::QuickLaunchService) -> Result<T, QuickLaunchError> + Send + 'static,
) -> Result<T, QuickLaunchCommandError> {
    tauri::async_runtime::spawn_blocking(move || operation(&service))
        .await
        .map_err(|_| QuickLaunchCommandError { code: "quick_launch_operation_failed", message: "快速启动后台任务意外结束，请重试。".to_owned() })?
        .map_err(map_error)
}

fn publish_snapshot(
    app: &AppHandle,
    result: Result<QuickLaunchSnapshot, QuickLaunchCommandError>,
) -> Result<QuickLaunchSnapshot, QuickLaunchCommandError> {
    let snapshot = result?;
    // The hidden tool-menu WebView is a separate React runtime. Publishing
    // every persisted mutation keeps it in the same item-count/layout state
    // as the settings page before the next hotkey opens it.
    let _ = app.emit(QUICK_LAUNCH_CHANGED_EVENT, &snapshot);
    if let Some(tool_menu) = app.get_webview_window(crate::infrastructure::tool_menu_surface_window::TOOL_MENU_SURFACE_LABEL) {
        let _ = tool_menu.emit(
            crate::infrastructure::tool_menu_surface_window::TOOL_MENU_SURFACE_SNAPSHOT_EVENT,
            &snapshot,
        );
    }
    Ok(snapshot)
}

fn map_error(error: QuickLaunchError) -> QuickLaunchCommandError {
    let (code, message) = match error {
        QuickLaunchError::InvalidPath => ("quick_launch_invalid_path", "所选文件不是可启动的程序或快捷方式。"),
        QuickLaunchError::AlreadyPinned => ("quick_launch_already_pinned", "该程序已固定。"),
        QuickLaunchError::NotPinned => ("quick_launch_not_pinned", "该程序已不在固定列表中。"),
        QuickLaunchError::Unavailable => ("quick_launch_unavailable", "该程序或图标当前不可用。"),
        QuickLaunchError::LaunchFailed => ("quick_launch_launch_failed", "无法启动该程序，请检查目标是否仍存在。"),
        QuickLaunchError::Storage(_) | QuickLaunchError::Icon(_) | QuickLaunchError::DiscoveryState => ("quick_launch_operation_failed", "快速启动操作未完成，请重试。"),
    };
    QuickLaunchCommandError { code, message: message.to_owned() }
}
