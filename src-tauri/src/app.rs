//! Application assembly: startup, managed state, command registration and shutdown.

use tauri::{AppHandle, Manager, RunEvent, Runtime};

use crate::adapters::credentials::Credentials;
use crate::adapters::local_store::LocalStore;
use crate::commands;
use crate::errors::AppResult;
use crate::logging;
use crate::navigation;
use crate::state::{AppPaths, AppState, LocalStoreState};

/// Builds and runs the Tauri application.
///
/// # Panics
///
/// Panics if the Tauri runtime fails to start, which leaves no UI to report the error to.
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(navigation::guard())
        // Used only from Rust after validation; the webview is not granted its commands.
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        // Native file dialogs, shown by the core for imports and exports. The webview is not
        // granted the plugin's commands, so it never gets file paths or file access.
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            setup(app.handle())?;
            Ok(())
        })
        .invoke_handler(commands::handler())
        .build(tauri::generate_context!())
        .expect("failed to start the aidb runtime");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            shutdown(handle);
        }
    });
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

    let local_store = match tauri::async_runtime::block_on(LocalStore::open(paths.data_dir())) {
        Ok(store) => LocalStoreState::Ready(store),
        Err(error) => {
            tracing::error!(error = ?crate::errors::AppError::from(error), "local database unavailable");
            LocalStoreState::Unavailable {
                reason: "The local database could not be opened or upgraded. Details are in the local log."
                    .to_owned(),
            }
        }
    };

    let credentials = Credentials::detect();
    tracing::info!(backend = ?credentials.backend(), "credential storage selected");

    app.manage(AppState::new(paths, local_store, credentials));
    Ok(())
}

/// Closes the local store cleanly so its write-ahead log is checkpointed.
fn shutdown<R: Runtime>(app: &AppHandle<R>) {
    if let Some(state) = app.try_state::<AppState>()
        && let LocalStoreState::Ready(store) = state.local_store_state()
    {
        tauri::async_runtime::block_on(store.close());
        tracing::info!("local database closed");
    }
}
