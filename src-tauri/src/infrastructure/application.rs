use std::path::PathBuf;

use tauri::{AppHandle, Manager, Runtime};

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
    // Resolved once through Tauri and retained as the future storage root.
    #[allow(dead_code)]
    app_data_dir: PathBuf,
}

impl ApplicationRuntime {
    pub fn initialize<R: Runtime>(app: &AppHandle<R>) -> Result<Self, tauri::Error> {
        let app_data_dir = app.path().app_data_dir()?;
        Ok(Self::from_app_data_dir(app_data_dir))
    }

    pub fn status(&self) -> ApplicationStatus {
        ApplicationStatus::Running
    }

    pub fn startup_mode(&self) -> StartupMode {
        // The minimal shell does not register an operating-system startup entry.
        StartupMode::Manual
    }

    #[cfg(test)]
    pub(crate) fn app_data_dir(&self) -> &std::path::Path {
        &self.app_data_dir
    }

    pub(crate) fn from_app_data_dir(app_data_dir: PathBuf) -> Self {
        Self { app_data_dir }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_keeps_the_resolved_app_data_directory_as_the_storage_entry() {
        let app_data_dir = PathBuf::from("test-data");
        let runtime = ApplicationRuntime::from_app_data_dir(app_data_dir.clone());

        assert_eq!(runtime.app_data_dir(), app_data_dir);
        assert_eq!(runtime.status(), ApplicationStatus::Running);
        assert_eq!(runtime.startup_mode(), StartupMode::Manual);
    }
}
