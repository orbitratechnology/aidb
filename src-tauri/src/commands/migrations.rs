//! Applying schema migrations from a comparison, and their records.

use tauri::State;

use crate::domain::connections::ConnectionId;
use crate::domain::migration::{
    MigrationPreview, MigrationRecord, MigrationResult, MigrationStatus,
};
use crate::domain::policy::approval::ApprovalId;
use crate::errors::AppResult;
use crate::services::migrations;
use crate::state::AppState;

/// Prepares a comparison's migration for the user's approval. Nothing runs.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn prepare_migration(
    state: State<'_, AppState>,
    connection_id: ConnectionId,
    diff_id: uuid::Uuid,
) -> AppResult<MigrationPreview> {
    migrations::prepare(
        state.local_store()?,
        state.sessions(),
        state.approvals(),
        state.schema_diffs(),
        connection_id,
        diff_id,
    )
    .await
}

/// The user's approval of a prepared migration, which then runs. Elevated approvals need the
/// database name in `confirmation`; steps that delete data need `accept_data_loss`.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn apply_migration(
    state: State<'_, AppState>,
    connection_id: ConnectionId,
    approval_id: ApprovalId,
    confirmation: Option<String>,
    accept_data_loss: bool,
) -> AppResult<MigrationResult> {
    let result = migrations::apply(
        state.local_store()?,
        state.sessions(),
        state.approvals(),
        state.schema_diffs(),
        connection_id,
        migrations::Decision {
            approval_id,
            confirmation: confirmation.as_deref(),
            accept_data_loss,
        },
    )
    .await?;
    if result.status != MigrationStatus::Failed {
        state.catalog().forget(connection_id);
    }
    Ok(result)
}

/// Declines a prepared migration.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub fn cancel_migration(state: State<'_, AppState>, approval_id: ApprovalId) -> AppResult<()> {
    migrations::cancel(state.approvals(), approval_id)
}

/// Applied migrations, newest first, for one connection or all.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn list_migration_records(
    state: State<'_, AppState>,
    connection_id: Option<ConnectionId>,
) -> AppResult<Vec<MigrationRecord>> {
    migrations::list(state.local_store()?, connection_id).await
}
