use tauri::{Runtime, State, WebviewWindow};

use crate::adapters::files::{self, DialogRequest};
use crate::domain::audit::Actor;
use crate::domain::catalog::ObjectName;
use crate::domain::connections::ConnectionId;
use crate::domain::csv_import::{
    CsvImportDraft, CsvImportPreview, CsvImportRequest, CsvOptions, MAX_IMPORT_BYTES,
};
use crate::domain::data_change::{DataChange, DataChangePreview, DataChangeResult, PendingChange};
use crate::domain::policy::approval::ApprovalId;
use crate::errors::AppResult;
use crate::services::sessions::not_connected;
use crate::services::{data_changes, imports};
use crate::state::AppState;

/// Plans INSERT, UPDATE and DELETE changes, applied together in order, against the live tables
/// and holds them for approval. Nothing is written.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn preview_data_change(
    state: State<'_, AppState>,
    connection_id: ConnectionId,
    changes: Vec<DataChange>,
) -> AppResult<DataChangePreview> {
    data_changes::preview(
        state.local_store()?,
        state.sessions(),
        state.approvals(),
        connection_id,
        changes,
        Actor::User,
    )
    .await
}

/// The user's approval of previewed changes, which then run together in one checked transaction.
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

/// Declines previewed changes.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn cancel_data_change(
    state: State<'_, AppState>,
    approval_id: ApprovalId,
) -> AppResult<()> {
    data_changes::cancel(state.local_store()?, state.approvals(), approval_id).await
}

/// Data changes waiting for the user's decision (the assistant's proposals among them), on one
/// connection or all.
#[tauri::command]
#[tracing::instrument(skip_all)]
pub fn list_pending_changes(
    state: State<'_, AppState>,
    connection_id: Option<ConnectionId>,
) -> Vec<PendingChange> {
    data_changes::pending(state.approvals(), connection_id)
}

/// Asks for a CSV file and reads it for import into `table`, detecting its encoding, delimiter
/// and header. The file stays in the core; the webview gets its columns, a sample and counts.
/// `None` when the user cancels.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn open_csv_import<R: Runtime>(
    window: WebviewWindow<R>,
    state: State<'_, AppState>,
    connection_id: ConnectionId,
    table: ObjectName,
) -> AppResult<Option<CsvImportDraft>> {
    // Fails before the dialog when not connected.
    state
        .sessions()
        .get(connection_id)
        .ok_or_else(not_connected)?;
    let request = DialogRequest {
        title: "Import CSV",
        file_name: "",
        filter_name: "CSV",
        extensions: &["csv", "tsv", "txt"],
    };
    let Some((file_name, bytes)) =
        files::open_via_dialog(&window, request, MAX_IMPORT_BYTES).await?
    else {
        return Ok(None);
    };
    imports::open(
        state.sessions(),
        state.imports(),
        connection_id,
        table,
        file_name,
        bytes,
    )
    .await
    .map(Some)
}

/// Reads an open import's file again with other options.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub fn reread_csv_import(
    state: State<'_, AppState>,
    import_id: uuid::Uuid,
    options: CsvOptions,
) -> AppResult<CsvImportDraft> {
    imports::reread(state.imports(), import_id, options)
}

/// Checks an import's mapping and plans it against the live table; when it is ready, it is held
/// for approval and applied with `apply_data_change`.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn preview_csv_import(
    state: State<'_, AppState>,
    connection_id: ConnectionId,
    import_id: uuid::Uuid,
    request: CsvImportRequest,
) -> AppResult<CsvImportPreview> {
    imports::preview(
        state.local_store()?,
        state.sessions(),
        state.approvals(),
        state.imports(),
        connection_id,
        import_id,
        request,
    )
    .await
}

/// Forgets an open import's file.
#[tauri::command]
#[tracing::instrument(skip_all)]
pub fn close_csv_import(state: State<'_, AppState>, import_id: uuid::Uuid) {
    imports::close(state.imports(), import_id);
}
