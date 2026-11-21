//! The query advisor.

use tauri::State;

use crate::domain::advisor::{ExplainRequest, QueryPlan};
use crate::domain::connections::ConnectionId;
use crate::errors::AppResult;
use crate::services::advisor;
use crate::state::AppState;

/// Plans one query (with `analyze`, also runs it, read-only and rolled back) and returns the
/// plan tree with suggestions drawn from it. Cancel it with `cancel_operation`.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn explain_query(
    state: State<'_, AppState>,
    connection_id: ConnectionId,
    request: ExplainRequest,
) -> AppResult<QueryPlan> {
    advisor::explain(state.sessions(), connection_id, &request).await
}
