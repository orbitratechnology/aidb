//! Application assembly: startup, managed state and command registration.

use tauri::{AppHandle, Manager, Runtime};

use crate::commands;
use crate::errors::AppResult;
use crate::logging;
use crate::state::{AppPaths, AppState};

/// Builds and runs the Tauri application.
///
/// # Panics
///
/// Panics if the Tauri runtime fails to start, which leaves no UI to report the error to.
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            setup(app.handle())?;
            Ok(())
        })
        .invoke_handler(commands::handler())
        .run(tauri::generate_context!())
        .expect("failed to start the aidb runtime");
}

fn setup<R: Runtime>(app: &AppHandle<R>) -> AppResult<()> {
    let paths = AppPaths::resolve(app)?;

    if let Err(error) = logging::init(paths.log_dir()) {
        logging::init_stderr_only();
        tracing::warn!(error = %error, "file logging unavailable; logging to stderr only");
    }

    tracing::info!(
        version = %app.package_info().version,
        os = std::env::consts::OS,
        "aidb core started"
    );

    app.manage(AppState::new(paths));
    Ok(())
}
