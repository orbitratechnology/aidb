use tauri::State;

use crate::domain::connections::ConnectionId;
use crate::domain::library::{HistoryEntry, SavedQuery, SavedQueryDraft};
use crate::errors::AppResult;
use crate::services::library;
use crate::state::AppState;

/// SQL run on a connection, newest first; `search` filters by text, ignoring case.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn list_query_history(
    state: State<'_, AppState>,
    connection_id: ConnectionId,
    search: Option<String>,
    limit: Option<u32>,
) -> AppResult<Vec<HistoryEntry>> {
    library::history(state.local_store()?, connection_id, search, limit).await
}

#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn delete_query_history_entry(state: State<'_, AppState>, id: i64) -> AppResult<()> {
    library::delete_history_entry(state.local_store()?, id).await
}

/// Saved queries, most recently changed first; `search` filters by name or text.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn list_saved_queries(
    state: State<'_, AppState>,
    search: Option<String>,
) -> AppResult<Vec<SavedQuery>> {
    library::saved(state.local_store()?, search).await
}

/// Creates a saved query, or updates the one named by `draft.id`.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn save_query(
    state: State<'_, AppState>,
    draft: SavedQueryDraft,
) -> AppResult<SavedQuery> {
    library::save(state.local_store()?, draft).await
}

#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn delete_saved_query(state: State<'_, AppState>, id: String) -> AppResult<()> {
    library::delete_saved(state.local_store()?, &id).await
}
