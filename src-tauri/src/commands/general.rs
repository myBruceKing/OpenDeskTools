use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, State};

use crate::infrastructure::application::ApplicationRuntime;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneralViewModel {
    version: String,
    autostart_enabled: bool,
    elevated_autostart_enabled: bool,
    start_minimized: bool,
    close_to_tray: bool,
    tray_icon_visible: bool,
    administrator_mode: bool,
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
pub async fn set_autostart_enabled<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<GeneralViewModel, GeneralCommandError> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let runtime = worker_app
            .try_state::<ApplicationRuntime>()
            .ok_or_else(general_runtime_unavailable)?;
        if !enabled && runtime.elevated_autostart().is_enabled() {
            runtime
                .elevated_autostart()
                .set(false)
                .map_err(elevated_autostart_error)?;
        }
        runtime
            .autostart()
            .set(enabled)
            .map_err(|error| GeneralCommandError {
                code: "autostart_update_failed",
                message: format!("开机自启设置未生效：{error}"),
            })?;
        Ok(current_view_model(&worker_app, &runtime))
    })
    .await
    .map_err(|_| general_runtime_unavailable())?
}

#[tauri::command]
pub async fn set_elevated_autostart_enabled<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<GeneralViewModel, GeneralCommandError> {
    let worker_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let runtime = worker_app
            .try_state::<ApplicationRuntime>()
            .ok_or_else(general_runtime_unavailable)?;
        if enabled {
            if !runtime.autostart().is_enabled().unwrap_or(false) {
                return Err(GeneralCommandError {
                    code: "elevated_autostart_requires_autostart",
                    message: "请先开启“开机自启动”，再启用管理员权限启动。".to_owned(),
                });
            }
            runtime
                .autostart()
                .set(false)
                .map_err(|error| GeneralCommandError {
                    code: "elevated_autostart_prepare_failed",
                    message: format!("管理员自启配置前未能暂停普通自启：{error}"),
                })?;
            if let Err(error) = runtime.elevated_autostart().set(true) {
                let rollback = runtime.autostart().set(true);
                return Err(GeneralCommandError {
                    code: "elevated_autostart_update_failed",
                    message: elevated_autostart_failure_message(&error, rollback.as_ref().err()),
                });
            }
        } else {
            runtime
                .autostart()
                .set(true)
                .map_err(|error| GeneralCommandError {
                    code: "elevated_autostart_prepare_failed",
                    message: format!("管理员自启关闭前未能恢复普通自启：{error}"),
                })?;
            if let Err(error) = runtime.elevated_autostart().set(false) {
                let rollback = runtime.autostart().set(false);
                return Err(GeneralCommandError {
                    code: "elevated_autostart_update_failed",
                    message: elevated_autostart_failure_message(&error, rollback.as_ref().err()),
                });
            }
        }
        Ok(current_view_model(&worker_app, &runtime))
    })
    .await
    .map_err(|_| general_runtime_unavailable())?
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
pub fn set_tray_icon_visible<R: Runtime>(
    app: AppHandle<R>,
    runtime: State<'_, ApplicationRuntime>,
    enabled: bool,
) -> Result<GeneralViewModel, GeneralCommandError> {
    let previous = runtime.tray_icon_visible();
    runtime
        .set_tray_icon_visible(enabled)
        .map_err(|error| GeneralCommandError {
            code: "tray_icon_update_failed",
            message: format!("托盘图标设置未保存：{error}"),
        })?;
    if let Err(error) = crate::infrastructure::tray::set_visible(&app, enabled) {
        let rollback = runtime.set_tray_icon_visible(previous);
        return Err(GeneralCommandError {
            code: "tray_icon_update_failed",
            message: tray_visibility_failure_message(&error, rollback.as_ref().err()),
        });
    }
    Ok(current_view_model(&app, &runtime))
}

#[tauri::command]
pub fn restart_as_administrator<R: Runtime>(app: AppHandle<R>) -> Result<(), GeneralCommandError> {
    if crate::infrastructure::elevation::is_elevated() {
        return Ok(());
    }
    crate::infrastructure::elevation::launch_current_as_administrator().map_err(|error| {
        GeneralCommandError {
            code: "administrator_restart_failed",
            message: format!("未能以管理员身份重新启动：{error}"),
        }
    })?;
    std::thread::Builder::new()
        .name("administrator-restart-exit".to_owned())
        .spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(250));
            app.exit(0);
        })
        .map_err(|error| GeneralCommandError {
            code: "administrator_restart_failed",
            message: format!("管理员版本已启动，但当前进程未能退出：{error}"),
        })?;
    Ok(())
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
pub async fn select_and_migrate_data_directory<R: Runtime>(
    app: AppHandle<R>,
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
    let (storage, data_directory) = runtime.data_directory_migration_context();
    let copied = tauri::async_runtime::spawn_blocking(move || {
        let copied = storage.copy_to_new_data_root(directory)?;
        data_directory.set(&copied)?;
        Ok::<_, crate::infrastructure::application::DataDirectoryChangeError>(copied)
    })
    .await
    .map_err(|error| GeneralCommandError {
        code: "data_directory_migration_task_failed",
        message: format!("数据目录迁移任务意外终止：{error}"),
    })?
    .map_err(|error| GeneralCommandError {
        code: "data_directory_migration_failed",
        message: format!("数据目录迁移未完成：{error}"),
    })?;
    let result = DataDirectoryMigrationResult {
        data_directory: display_data_directory(&copied),
        restart_required: true,
    };
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(700));
        app.request_restart();
    });
    Ok(Some(result))
}

fn current_view_model<R: Runtime>(
    app: &AppHandle<R>,
    runtime: &ApplicationRuntime,
) -> GeneralViewModel {
    build_view_model(
        app.package_info().version.to_string(),
        GeneralBehavior {
            autostart_enabled: runtime.elevated_autostart().is_enabled()
                || runtime.autostart().is_enabled().unwrap_or(false),
            elevated_autostart_enabled: runtime.elevated_autostart().is_enabled(),
            start_minimized: runtime.start_minimized(),
            close_to_tray: runtime.close_to_tray(),
            tray_icon_visible: runtime.tray_icon_visible(),
            administrator_mode: crate::infrastructure::elevation::is_elevated(),
            crash_diagnostics_enabled: runtime.crash_diagnostics_enabled(),
        },
        display_data_directory(runtime.storage().data_root()),
    )
}

#[derive(Debug, Clone, Copy)]
struct GeneralBehavior {
    autostart_enabled: bool,
    elevated_autostart_enabled: bool,
    start_minimized: bool,
    close_to_tray: bool,
    tray_icon_visible: bool,
    administrator_mode: bool,
    crash_diagnostics_enabled: bool,
}

fn build_view_model(
    version: String,
    behavior: GeneralBehavior,
    data_directory: String,
) -> GeneralViewModel {
    GeneralViewModel {
        version,
        autostart_enabled: behavior.autostart_enabled,
        elevated_autostart_enabled: behavior.elevated_autostart_enabled,
        start_minimized: behavior.start_minimized,
        close_to_tray: behavior.close_to_tray,
        tray_icon_visible: behavior.tray_icon_visible,
        administrator_mode: behavior.administrator_mode,
        crash_diagnostics_enabled: behavior.crash_diagnostics_enabled,
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

fn tray_visibility_failure_message(
    apply: &dyn std::fmt::Display,
    rollback: Option<&crate::infrastructure::storage::StorageError>,
) -> String {
    match rollback {
        None => format!("托盘图标设置未生效，保存状态已回滚：{apply}"),
        Some(rollback) => format!(
            "托盘图标设置未生效，且保存状态回滚失败；请重启应用后检查设置：应用错误={apply}；回滚错误={rollback}"
        ),
    }
}

fn elevated_autostart_error(
    error: crate::infrastructure::elevated_autostart::ElevatedAutostartError,
) -> GeneralCommandError {
    GeneralCommandError {
        code: "elevated_autostart_update_failed",
        message: format!("管理员权限自启设置未生效：{error}"),
    }
}

fn elevated_autostart_failure_message(
    apply: &dyn std::fmt::Display,
    rollback: Option<&crate::infrastructure::autostart::AutostartError>,
) -> String {
    match rollback {
        None => format!("管理员权限自启设置未生效，普通自启状态已恢复：{apply}"),
        Some(rollback) => format!(
            "管理员权限自启设置未生效，且普通自启状态恢复失败；请重启应用后检查设置：授权错误={apply}；回滚错误={rollback}"
        ),
    }
}

fn general_runtime_unavailable() -> GeneralCommandError {
    GeneralCommandError {
        code: "general_runtime_unavailable",
        message: "常规设置后台暂时不可用，请重启应用后重试。".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_model_carries_every_general_preference() {
        let view_model = build_view_model(
            "1.2.3".to_owned(),
            GeneralBehavior {
                autostart_enabled: true,
                elevated_autostart_enabled: true,
                start_minimized: true,
                close_to_tray: false,
                tray_icon_visible: true,
                administrator_mode: true,
                crash_diagnostics_enabled: true,
            },
            r"C:\OpenDeskToolsTestData\com.opendesktools.app".to_owned(),
        );

        assert_eq!(view_model.version, "1.2.3");
        assert!(view_model.autostart_enabled);
        assert!(view_model.elevated_autostart_enabled);
        assert!(view_model.start_minimized);
        assert!(!view_model.close_to_tray);
        assert!(view_model.tray_icon_visible);
        assert!(view_model.administrator_mode);
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

    #[test]
    fn tray_visibility_failure_distinguishes_successful_and_failed_rollback() {
        let apply = "tray unavailable";
        assert!(
            tray_visibility_failure_message(&apply, None).contains("保存状态已回滚"),
            "successful rollback must be explicit"
        );
        let rollback = crate::infrastructure::storage::StorageError::LockPoisoned;
        assert!(
            tray_visibility_failure_message(&apply, Some(&rollback)).contains("回滚失败"),
            "failed rollback must not be reported as a normal apply failure"
        );
    }
}
