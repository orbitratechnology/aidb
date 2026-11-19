use tauri::State;

use crate::domain::audit::{AuditEntry, AuditFilter};
use crate::errors::AppResult;
use crate::services::audit;
use crate::state::AppState;

/// Audit events, newest first: approvals, refused checks, assistant tasks, connection changes.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn list_audit_events(
    state: State<'_, AppState>,
    filter: AuditFilter,
) -> AppResult<Vec<AuditEntry>> {
    audit::list(state.local_store()?, filter).await
}
