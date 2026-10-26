use secrecy::SecretString;
use tauri::State;

use crate::domain::connections::{ConnectionDraft, ConnectionId};
use crate::errors::AppResult;
use crate::services::sessions::{self, ActiveSession, ConnectionTestReport};
use crate::state::AppState;

/// Tests settings from the connection form, saved or not. The password, if given, is used once
/// and never stored.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn test_connection(
    state: State<'_, AppState>,
    connection: ConnectionDraft,
    password: Option<SecretString>,
    saved_id: Option<ConnectionId>,
    allow_unencrypted: bool,
) -> AppResult<ConnectionTestReport> {
    sessions::test(
        state.local_store()?,
        state.credentials(),
        state.sessions(),
        connection,
        password,
        saved_id,
        allow_unencrypted,
    )
    .await
}

/// Opens a session for a saved profile. A password given here is used for this session only.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn connect(
    state: State<'_, AppState>,
    id: ConnectionId,
    password: Option<SecretString>,
) -> AppResult<ActiveSession> {
    sessions::connect(
        state.local_store()?,
        state.credentials(),
        state.sessions(),
        id,
        password,
    )
    .await
}

#[tauri::command]
#[tracing::instrument(skip_all)]
pub async fn disconnect(state: State<'_, AppState>, id: ConnectionId) -> AppResult<bool> {
    let closed = state.sessions().close(id).await;
    tracing::info!(connection = %id, closed, "disconnected");
    Ok(closed)
}

#[tauri::command]
#[tracing::instrument(skip_all)]
pub async fn active_sessions(state: State<'_, AppState>) -> AppResult<Vec<ActiveSession>> {
    Ok(state.sessions().list())
}
