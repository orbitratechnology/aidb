use tauri::State;

use crate::domain::audit::Actor;
use crate::domain::connections::ConnectionId;
use crate::domain::data_change::{DataChange, DataChangePreview, DataChangeResult};
use crate::domain::policy::approval::ApprovalId;
use crate::errors::AppResult;
use crate::services::data_changes;
use crate::state::AppState;

/// Plans an INSERT, UPDATE or DELETE against the live table and holds it for approval. Nothing
/// is written.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn preview_data_change(
    state: State<'_, AppState>,
    connection_id: ConnectionId,
    change: DataChange,
) -> AppResult<DataChangePreview> {
    data_changes::preview(
        state.local_store()?,
        state.sessions(),
        state.approvals(),
        connection_id,
        change,
        Actor::User,
    )
    .await
}

/// The user's approval of a previewed change, which then runs in one checked transaction.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn apply_data_change(
    state: State<'_, AppState>,
    connection_id: ConnectionId,
    approval_id: ApprovalId,
    confirmation: Option<String>,
) -> AppResult<DataChangeResult> {
    data_changes::apply(
        state.local_store()?,
        state.sessions(),
        state.approvals(),
        connection_id,
        approval_id,
        confirmation.as_deref(),
    )
    .await
}

/// Declines a previewed change.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn cancel_data_change(
    state: State<'_, AppState>,
    approval_id: ApprovalId,
) -> AppResult<()> {
    data_changes::cancel(state.local_store()?, state.approvals(), approval_id).await
}
