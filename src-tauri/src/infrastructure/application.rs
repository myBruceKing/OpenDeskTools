use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, Manager, Runtime};
use thiserror::Error;

use super::hotkey::{HotkeyError, HotkeyManager};
use super::hotkey_capture::HotkeyCaptureManager;
use super::storage::{StorageError, StorageService};
use super::theme::{ThemeError, ThemeService};

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
    hotkeys: HotkeyManager,
    hotkey_capture: HotkeyCaptureManager,
    theme: ThemeService,
}

#[derive(Debug, Error)]
pub enum ApplicationRuntimeError {
    #[error("failed to resolve the application data directory: {0}")]
    AppDataDirectory(#[from] tauri::Error),
    #[error("failed to initialize application storage: {0}")]
    Storage(#[from] StorageError),
    #[error("failed to initialize theme service: {0}")]
    Theme(#[from] ThemeError),
    #[error("failed to initialize hotkey manager: {0}")]
    Hotkey(#[from] HotkeyError),
}

impl ApplicationRuntime {
    pub fn initialize<R: Runtime>(app: &AppHandle<R>) -> Result<Self, ApplicationRuntimeError> {
        let app_data_dir = app.path().app_data_dir()?;
        Self::from_app_data_dir(app_data_dir)
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

    pub(crate) fn hotkeys(&self) -> &HotkeyManager {
        &self.hotkeys
    }

    pub(crate) fn hotkey_capture(&self) -> &HotkeyCaptureManager {
        &self.hotkey_capture
    }

    pub(crate) fn from_app_data_dir(
        app_data_dir: PathBuf,
    ) -> Result<Self, ApplicationRuntimeError> {
        let storage = Arc::new(StorageService::initialize(app_data_dir)?);
        let theme = ThemeService::initialize(Arc::clone(&storage))?;
        let hotkeys = HotkeyManager::initialize(Arc::clone(&storage))?;
        Ok(Self {
            storage,
            hotkeys,
            hotkey_capture: HotkeyCaptureManager::default(),
            theme,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

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
