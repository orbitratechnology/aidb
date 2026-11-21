//! Application assembly: startup, managed state, command registration and shutdown.

use std::sync::Once;

use tauri::ipc::Invoke;
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
        .invoke_handler(log_first_command(commands::handler()))
        .build(context())
        .expect("failed to start the aidb runtime");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            shutdown(handle);
        }
    });
}

/// The app's configuration, assets and capabilities, for any runtime (tests use the mock one).
/// The only expansion of `generate_context!` in the crate: on macOS it embeds the Info.plist
/// under a fixed symbol, so a second expansion fails to link.
pub(crate) fn context<R: Runtime>() -> tauri::Context<R> {
    tauri::generate_context!()
}

/// Logs the first command the interface sends, once, by name only. It shows that the webview
/// loaded the bundled interface and reached the core: a blank window (missing assets, a script
/// blocked) leaves no such line. The package smoke tests wait for it.
fn log_first_command<R: Runtime>(
    handler: impl Fn(Invoke<R>) -> bool + Send + Sync + 'static,
) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    let first = Once::new();
    move |invoke| {
        first.call_once(|| {
            tracing::info!(command = invoke.message.command(), "interface connected");
        });
        handler(invoke)
    }
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
        Ok(store) => {
            // Tasks still marked running were cut off when the app last quit.
            match tauri::async_runtime::block_on(store.agent_tasks().close_abandoned()) {
                Ok(0) => {}
                Ok(closed) => tracing::info!(closed, "closed assistant tasks left running"),
                Err(error) => tracing::warn!(error = %error, "could not close abandoned tasks"),
            }
            LocalStoreState::Ready(store)
        }
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

/// Closes database sessions, then the local store so its write-ahead log is checkpointed.
fn shutdown<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    tauri::async_runtime::block_on(state.sessions().close_all());
    if let LocalStoreState::Ready(store) = state.local_store_state() {
        tauri::async_runtime::block_on(store.close());
        tracing::info!("local database closed");
    }
}
