use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use tauri::{AppHandle, Manager, Runtime};
use thiserror::Error;

use super::clipboard::ClipboardService;
use super::clipboard_input::ClipboardInputCoordinator;
use super::clipboard_listener::{
    ClipboardHistoryEventSink, ClipboardListenerError, ClipboardListenerManager,
};
use super::hotkey::{HotkeyError, HotkeyManager, OrdinaryHotkeyLatch};
use super::hotkey_capture::HotkeyCaptureManager;
use super::keyboard_hook::KeyboardHookBroker;
use super::storage::{StorageError, StorageService};
use super::surface::SurfaceManager;
use super::theme::{ThemeError, ThemeService};

const DATA_DIR_ARGUMENT: &str = "--data-dir";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationStatus {
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupMode {
    Manual,
}

#[derive(Debug)]
pub struct ApplicationRuntime {
    storage: Arc<StorageService>,
    clipboard: Arc<ClipboardService>,
    clipboard_listener: ClipboardListenerManager,
    clipboard_input: ClipboardInputCoordinator,
    surface: Arc<SurfaceManager>,
    hotkeys: HotkeyManager,
    ordinary_hotkey_latch: OrdinaryHotkeyLatch,
    keyboard_hook: Arc<KeyboardHookBroker>,
    hotkey_capture: HotkeyCaptureManager,
    theme: ThemeService,
}

#[derive(Debug, Error)]
pub enum ApplicationRuntimeError {
    #[error("failed to resolve the application data directory: {0}")]
    AppDataDirectory(#[from] tauri::Error),
    #[error("--data-dir must be followed by a non-empty path")]
    MissingDataDirectoryOverride,
    #[error("--data-dir may only be provided once")]
    DuplicateDataDirectoryOverride,
    #[error("failed to resolve the executable path for a relative --data-dir: {0}")]
    CurrentExecutable(#[source] std::io::Error),
    #[error("the executable path has no parent directory: {0}")]
    ExecutableDirectory(PathBuf),
    #[error(
        "--data-dir must be fully absolute or a plain relative path without a drive or root prefix: {0}"
    )]
    AmbiguousDataDirectoryOverride(PathBuf),
    #[error("failed to initialize application storage: {0}")]
    Storage(#[from] StorageError),
    #[error("failed to initialize theme service: {0}")]
    Theme(#[from] ThemeError),
    #[error("failed to initialize hotkey manager: {0}")]
    Hotkey(#[from] HotkeyError),
    #[error("failed to initialize clipboard service: {0}")]
    Clipboard(String),
}

impl ApplicationRuntime {
    pub fn initialize<R: Runtime>(app: &AppHandle<R>) -> Result<Self, ApplicationRuntimeError> {
        let data_root_override = parse_data_dir_override(std::env::args_os())?;
        let data_root = match data_root_override {
            None => app.path().app_data_dir()?,
            Some(path) if path.is_absolute() => path,
            Some(path) => {
                let executable =
                    std::env::current_exe().map_err(ApplicationRuntimeError::CurrentExecutable)?;
                resolve_relative_data_root(&executable, path)?
            }
        };
        Self::from_app_data_dir(data_root)
    }

    pub fn status(&self) -> ApplicationStatus {
        ApplicationStatus::Running
    }

    pub fn startup_mode(&self) -> StartupMode {
        // The minimal shell does not register an operating-system startup entry.
        StartupMode::Manual
    }

    #[allow(dead_code)]
    pub(crate) fn storage(&self) -> &StorageService {
        &self.storage
    }

    pub(crate) fn theme(&self) -> &ThemeService {
        &self.theme
    }

    pub(crate) fn clipboard(&self) -> &ClipboardService {
        &self.clipboard
    }

    pub(crate) fn clipboard_listener(&self) -> &ClipboardListenerManager {
        &self.clipboard_listener
    }

    pub(crate) fn clipboard_input(&self) -> &ClipboardInputCoordinator {
        &self.clipboard_input
    }

    pub(crate) fn surface(&self) -> &SurfaceManager {
        &self.surface
    }

    pub(crate) fn start_clipboard_listener(
        &self,
        sink: ClipboardHistoryEventSink,
    ) -> Result<(), ClipboardListenerError> {
        self.clipboard_listener
            .start(Arc::clone(&self.clipboard), sink)
    }

    pub(crate) fn hotkeys(&self) -> &HotkeyManager {
        &self.hotkeys
    }

    pub(crate) fn hotkey_capture(&self) -> &HotkeyCaptureManager {
        &self.hotkey_capture
    }

    pub(crate) fn keyboard_hook(&self) -> &KeyboardHookBroker {
        &self.keyboard_hook
    }

    pub(crate) fn ordinary_hotkey_latch(&self) -> &OrdinaryHotkeyLatch {
        &self.ordinary_hotkey_latch
    }

    pub(crate) fn from_app_data_dir(
        app_data_dir: PathBuf,
    ) -> Result<Self, ApplicationRuntimeError> {
        let storage = Arc::new(StorageService::initialize(app_data_dir)?);
        let clipboard = Arc::new(
            ClipboardService::try_initialize(Arc::clone(&storage))
                .map_err(|error| ApplicationRuntimeError::Clipboard(error.to_string()))?,
        );
        let theme = ThemeService::initialize(Arc::clone(&storage))?;
        let hotkeys = HotkeyManager::initialize(Arc::clone(&storage))?;
        let keyboard_hook = Arc::new(KeyboardHookBroker::default());
        let surface = Arc::new(SurfaceManager::default());
        let clipboard_input =
            ClipboardInputCoordinator::new(Arc::clone(&clipboard), Arc::clone(&surface));
        Ok(Self {
            storage,
            clipboard,
            clipboard_listener: ClipboardListenerManager::default(),
            clipboard_input,
            surface,
            hotkeys,
            ordinary_hotkey_latch: OrdinaryHotkeyLatch::default(),
            hotkey_capture: HotkeyCaptureManager::new(Arc::clone(&keyboard_hook)),
            keyboard_hook,
            theme,
        })
    }
}

fn parse_data_dir_override(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<Option<PathBuf>, ApplicationRuntimeError> {
    let mut arguments = arguments.into_iter();
    let _executable = arguments.next();
    let mut override_path = None;

    while let Some(argument) = arguments.next() {
        let candidate = if argument == OsStr::new(DATA_DIR_ARGUMENT) {
            let value = arguments
                .next()
                .ok_or(ApplicationRuntimeError::MissingDataDirectoryOverride)?;
            if value.is_empty() || value.to_string_lossy().starts_with("--") {
                return Err(ApplicationRuntimeError::MissingDataDirectoryOverride);
            }
            Some(PathBuf::from(value))
        } else {
            argument
                .to_str()
                .and_then(|value| value.strip_prefix("--data-dir="))
                .map(|value| {
                    if value.is_empty() {
                        Err(ApplicationRuntimeError::MissingDataDirectoryOverride)
                    } else {
                        Ok(PathBuf::from(value))
                    }
                })
                .transpose()?
        };

        if let Some(candidate) = candidate {
            if override_path.replace(candidate).is_some() {
                return Err(ApplicationRuntimeError::DuplicateDataDirectoryOverride);
            }
        }
    }

    Ok(override_path)
}

fn resolve_relative_data_root(
    executable: &Path,
    relative_path: PathBuf,
) -> Result<PathBuf, ApplicationRuntimeError> {
    if relative_path
        .components()
        .any(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
    {
        return Err(ApplicationRuntimeError::AmbiguousDataDirectoryOverride(
            relative_path,
        ));
    }
    let executable_directory = executable
        .parent()
        .filter(|directory| !directory.as_os_str().is_empty())
        .ok_or_else(|| ApplicationRuntimeError::ExecutableDirectory(executable.to_path_buf()))?;
    Ok(executable_directory.join(relative_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn data_directory_override_accepts_separate_and_equals_forms() {
        assert_eq!(
            parse_data_dir_override(arguments(&["OpenDeskTools.exe", "--data-dir", "data"]))
                .unwrap(),
            Some(PathBuf::from("data"))
        );
        assert_eq!(
            parse_data_dir_override(arguments(&["OpenDeskTools.exe", "--data-dir=便携数据"]))
                .unwrap(),
            Some(PathBuf::from("便携数据"))
        );
    }

    #[test]
    fn data_directory_override_ignores_unrelated_arguments_and_preserves_default() {
        assert_eq!(
            parse_data_dir_override(arguments(&["OpenDeskTools.exe", "--unrelated", "value"]))
                .unwrap(),
            None
        );
    }

    #[test]
    fn data_directory_override_rejects_missing_empty_and_duplicate_values() {
        for values in [
            vec!["OpenDeskTools.exe", "--data-dir"],
            vec!["OpenDeskTools.exe", "--data-dir="],
            vec!["OpenDeskTools.exe", "--data-dir", "--other"],
        ] {
            assert!(matches!(
                parse_data_dir_override(arguments(&values)),
                Err(ApplicationRuntimeError::MissingDataDirectoryOverride)
            ));
        }

        assert!(matches!(
            parse_data_dir_override(arguments(&[
                "OpenDeskTools.exe",
                "--data-dir",
                "first",
                "--data-dir=second"
            ])),
            Err(ApplicationRuntimeError::DuplicateDataDirectoryOverride)
        ));
    }

    #[test]
    fn relative_data_directory_is_anchored_to_the_executable_directory() {
        let executable = Path::new("C:/Portable/OpenDeskTools/OpenDeskTools.exe");

        assert_eq!(
            resolve_relative_data_root(executable, PathBuf::from("data")).unwrap(),
            PathBuf::from("C:/Portable/OpenDeskTools/data")
        );
    }

    #[test]
    fn absolute_data_directory_override_does_not_depend_on_the_executable_directory() {
        let temp = tempdir().unwrap();
        let absolute = temp.path().join("portable-data");

        assert!(absolute.is_absolute());
        assert_eq!(
            parse_data_dir_override(vec![
                OsString::from("OpenDeskTools.exe"),
                OsString::from(format!("--data-dir={}", absolute.display()))
            ])
            .unwrap(),
            Some(absolute)
        );
    }

    #[cfg(windows)]
    #[test]
    fn drive_relative_and_root_relative_overrides_are_rejected_as_ambiguous() {
        let executable = Path::new(r"C:\Portable\OpenDeskTools\OpenDeskTools.exe");

        for path in [PathBuf::from(r"\data"), PathBuf::from(r"C:data")] {
            assert!(matches!(
                resolve_relative_data_root(executable, path),
                Err(ApplicationRuntimeError::AmbiguousDataDirectoryOverride(_))
            ));
        }
    }

    #[test]
    fn runtime_initializes_storage_in_the_resolved_application_data_directory() {
        let temp = tempdir().unwrap();
        let app_data_dir = temp.path().join("application-data");
        let runtime = ApplicationRuntime::from_app_data_dir(app_data_dir.clone()).unwrap();

        assert_eq!(
            runtime.storage().data_root(),
            app_data_dir.canonicalize().unwrap()
        );
        assert!(runtime.storage().database_path().is_file());
        assert_eq!(
            runtime.theme().current().unwrap(),
            super::super::theme::ThemeSnapshot::default()
        );
        assert_eq!(runtime.status(), ApplicationStatus::Running);
        assert_eq!(runtime.startup_mode(), StartupMode::Manual);
        assert_eq!(
            runtime.clipboard_listener().status(),
            super::super::clipboard_listener::ClipboardListenerStatus::Stopped
        );
    }

    #[test]
    fn runtime_does_not_exist_when_storage_initialization_fails() {
        let temp = tempdir().unwrap();
        let blocked_root = temp.path().join("blocked-root");
        std::fs::write(&blocked_root, b"occupied").unwrap();

        let error = ApplicationRuntime::from_app_data_dir(blocked_root).unwrap_err();

        assert!(matches!(error, ApplicationRuntimeError::Storage(_)));
    }
}
