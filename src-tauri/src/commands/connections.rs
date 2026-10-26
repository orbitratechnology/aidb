use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use tauri::{Runtime, State, WebviewWindow};

use crate::adapters::credentials::CredentialBackend;
use crate::adapters::files::{self, DialogRequest};
use crate::domain::connection_transfer::MAX_FILE_BYTES;
use crate::domain::connection_uri::{self, ParsedConnectionUri};
use crate::domain::connections::{ConnectionDraft, ConnectionId};
use crate::errors::{AppError, AppResult};
use crate::services::connection_secrets;
use crate::services::connections::{self, ConnectionDetails, ImportOutcome, ImportPreview};
use crate::state::AppState;

const JSON_FILTER: &str = "aidb connections";

#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn list_connections(state: State<'_, AppState>) -> AppResult<Vec<ConnectionDetails>> {
    connections::list(state.local_store()?, state.credentials()).await
}

#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn create_connection(
    state: State<'_, AppState>,
    connection: ConnectionDraft,
    allow_unencrypted: bool,
) -> AppResult<ConnectionDetails> {
    connections::create(
        state.local_store()?,
        state.credentials(),
        connection,
        allow_unencrypted,
    )
    .await
}

#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn update_connection(
    state: State<'_, AppState>,
    id: ConnectionId,
    connection: ConnectionDraft,
    allow_unencrypted: bool,
) -> AppResult<ConnectionDetails> {
    let updated = connections::update(
        state.local_store()?,
        state.credentials(),
        id,
        connection,
        allow_unencrypted,
    )
    .await?;
    // An open session keeps the settings it started with; close it so the new ones apply.
    state.sessions().close(id).await;
    Ok(updated)
}

#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn delete_connection(
    state: State<'_, AppState>,
    id: ConnectionId,
    forget_password: bool,
) -> AppResult<()> {
    connections::delete(
        state.local_store()?,
        state.credentials(),
        id,
        forget_password,
    )
    .await?;
    state.sessions().close(id).await;
    Ok(())
}

#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn save_connection_password(
    state: State<'_, AppState>,
    id: ConnectionId,
    password: SecretString,
) -> AppResult<CredentialBackend> {
    connection_secrets::save_password(state.local_store()?, state.credentials(), id, password).await
}

#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn forget_connection_password(
    state: State<'_, AppState>,
    id: ConnectionId,
) -> AppResult<bool> {
    connection_secrets::forget_password(state.local_store()?, state.credentials(), id).await
}

/// Takes the URI as a secret: it may carry a password, so it is never logged or kept.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn parse_connection_uri(uri: SecretString) -> AppResult<ParsedConnectionUri> {
    Ok(connection_uri::parse(uri.expose_secret())?)
}

/// Where an export went.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ExportOutcome {
    pub count: u32,
    pub file_name: String,
}

/// Writes every profile, without passwords, to a file the user picks. `None` if cancelled.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn export_connections<R: Runtime>(
    window: WebviewWindow<R>,
    state: State<'_, AppState>,
) -> AppResult<Option<ExportOutcome>> {
    let (bytes, count) = connections::export(state.local_store()?).await?;
    if count == 0 {
        return Err(AppError::invalid_input(
            "There are no connections to export.",
        ));
    }
    let file_name = connections::export_file_name();
    let request = DialogRequest {
        title: "Export connections",
        file_name: &file_name,
        filter_name: JSON_FILTER,
        extensions: &["json"],
    };
    let Some(file_name) = files::save_via_dialog(&window, request, bytes).await? else {
        return Ok(None);
    };
    tracing::info!(count, "connections exported");
    Ok(Some(ExportOutcome { count, file_name }))
}

/// Reads a file the user picks and reports what importing it would do. `None` if cancelled.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn preview_connection_import<R: Runtime>(
    window: WebviewWindow<R>,
    state: State<'_, AppState>,
) -> AppResult<Option<ImportPreview>> {
    let request = DialogRequest {
        title: "Import connections",
        file_name: "",
        filter_name: JSON_FILTER,
        extensions: &["json"],
    };
    let Some((file_name, bytes)) = files::open_via_dialog(&window, request, MAX_FILE_BYTES).await?
    else {
        return Ok(None);
    };
    connections::preview_import(state.local_store()?, file_name, &bytes)
        .await
        .map(Some)
}

#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn import_connections(
    state: State<'_, AppState>,
    connections: Vec<ConnectionDraft>,
    allow_unencrypted: bool,
) -> AppResult<ImportOutcome> {
    connections::import(state.local_store()?, connections, allow_unencrypted).await
}
