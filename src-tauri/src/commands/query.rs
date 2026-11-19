use tauri::State;

use crate::domain::connections::ConnectionId;
use crate::domain::query::{QueryRequest, QueryRun};
use crate::domain::sql::SqlAnalysis;
use crate::errors::AppResult;
use crate::services::query;
use crate::state::AppState;

/// Splits SQL into statements and says which can run, without touching a database.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub fn analyze_sql(sql: String) -> AppResult<SqlAnalysis> {
    query::analyze(&sql)
}

/// Runs one read statement on an open connection. Statements that are not reads, several
/// statements at once, and statement errors come back as outcomes rather than errors.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn run_query(
    state: State<'_, AppState>,
    connection_id: ConnectionId,
    request: QueryRequest,
) -> AppResult<QueryRun> {
    query::run(
        state.local_store()?,
        state.sessions(),
        connection_id,
        request,
    )
    .await
}
