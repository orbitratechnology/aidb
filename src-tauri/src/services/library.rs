//! Query history and saved queries.

use crate::adapters::local_store::{LIST_LIMIT, LocalStore, StoreError};
use crate::domain::connections::ConnectionId;
use crate::domain::library::{
    HistoryEntry, HistoryStatus, NewHistoryEntry, SavedQuery, SavedQueryDraft, history_text,
};
use crate::domain::query::QueryRun;
use crate::errors::{AppError, AppResult, ErrorCode};

/// Longest search text, in characters.
const MAX_SEARCH_CHARS: usize = 200;

/// Records a run in the history, with credentials redacted from its text. A failure to record
/// is logged, never shown: it must not hide the run's own outcome.
pub async fn record(store: &LocalStore, connection: ConnectionId, sql: &str, run: &QueryRun) {
    let (status, row_count, elapsed_ms, error_code) = match run {
        QueryRun::Completed(result) => (
            HistoryStatus::Completed,
            u64::try_from(result.rows.len()).ok(),
            Some(result.elapsed_ms),
            None,
        ),
        QueryRun::Refused(_) => (HistoryStatus::Refused, None, None, None),
        QueryRun::Failed(failure) if failure.code == ErrorCode::Cancelled => {
            (HistoryStatus::Cancelled, None, None, Some(failure.code))
        }
        QueryRun::Failed(failure) => (HistoryStatus::Failed, None, None, Some(failure.code)),
    };
    let entry = NewHistoryEntry {
        connection_id: connection,
        sql: history_text(sql),
        status,
        row_count,
        elapsed_ms,
        error_code,
    };
    match store.query_history().record(&entry).await {
        // The profile was forgotten while the statement ran: nothing to keep.
        Ok(()) | Err(StoreError::NotFound) => {}
        Err(error) => {
            tracing::warn!(error = ?AppError::from(error), "query history not recorded");
        }
    }
}

fn search_text(search: Option<String>) -> Option<String> {
    search
        .map(|text| {
            text.trim()
                .chars()
                .take(MAX_SEARCH_CHARS)
                .collect::<String>()
        })
        .filter(|text| !text.is_empty())
}

/// History of one connection, newest first.
///
/// # Errors
///
/// `storage` if the local database cannot be read.
pub async fn history(
    store: &LocalStore,
    connection: ConnectionId,
    search: Option<String>,
    limit: Option<u32>,
) -> AppResult<Vec<HistoryEntry>> {
    let search = search_text(search);
    Ok(store
        .query_history()
        .list(connection, search.as_deref(), limit.unwrap_or(LIST_LIMIT))
        .await?)
}

/// # Errors
///
/// `not_found` if the entry is gone; `storage` on a database error.
pub async fn delete_history_entry(store: &LocalStore, id: i64) -> AppResult<()> {
    store
        .query_history()
        .delete(id)
        .await
        .map_err(|error| missing(error, "That history entry no longer exists."))
}

/// # Errors
///
/// `storage` if the local database cannot be read.
pub async fn saved(store: &LocalStore, search: Option<String>) -> AppResult<Vec<SavedQuery>> {
    let search = search_text(search);
    Ok(store.saved_queries().list(search.as_deref()).await?)
}

/// Creates a saved query, or updates the one named by `draft.id`.
///
/// # Errors
///
/// `invalid_input` for a bad name or text; `not_found` when updating a deleted query.
pub async fn save(store: &LocalStore, draft: SavedQueryDraft) -> AppResult<SavedQuery> {
    let draft = draft.validated()?;
    store
        .saved_queries()
        .save(&draft)
        .await
        .map_err(|error| missing(error, "That saved query no longer exists."))
}

/// # Errors
///
/// `not_found` if the query is gone; `storage` on a database error.
pub async fn delete_saved(store: &LocalStore, id: &str) -> AppResult<()> {
    store
        .saved_queries()
        .delete(id)
        .await
        .map_err(|error| missing(error, "That saved query no longer exists."))
}

fn missing(error: StoreError, message: &'static str) -> AppError {
    match error {
        StoreError::NotFound => AppError::not_found(message),
        other => other.into(),
    }
}
