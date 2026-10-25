//! State shared with commands through Tauri's managed state.

use std::fs;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager, Runtime};

use crate::errors::{AppError, AppResult, ErrorCode};

/// Directories the app owns on this device.
#[derive(Debug, Clone)]
pub struct AppPaths {
    data_dir: PathBuf,
    log_dir: PathBuf,
}

impl AppPaths {
    /// Resolves the per-user data and log directories and creates them if missing.
    ///
    /// # Errors
    ///
    /// Returns a [`ErrorCode::Storage`] error if a directory cannot be resolved or created.
    pub fn resolve<R: Runtime>(app: &AppHandle<R>) -> AppResult<Self> {
        let resolver = app.path();
        let data_dir = resolver.app_data_dir().map_err(storage_error)?;
        let log_dir = resolver.app_log_dir().map_err(storage_error)?;
        for dir in [&data_dir, &log_dir] {
            fs::create_dir_all(dir).map_err(storage_error)?;
        }
        Ok(Self { data_dir, log_dir })
    }

    /// Where local metadata (settings, profiles, history) is stored.
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    #[must_use]
    pub fn log_dir(&self) -> &Path {
        &self.log_dir
    }
}

fn storage_error(cause: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> AppError {
    AppError::new(
        ErrorCode::Storage,
        "The app could not prepare its local data folder.",
    )
    .with_cause(cause)
}

/// Core state available to every command via `tauri::State<AppState>`.
#[derive(Debug)]
pub struct AppState {
    paths: AppPaths,
}

impl AppState {
    #[must_use]
    pub fn new(paths: AppPaths) -> Self {
        Self { paths }
    }

    #[must_use]
    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }
}
