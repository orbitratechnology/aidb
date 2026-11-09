use tauri::State;

use crate::domain::connections::ConnectionId;
use crate::domain::database::OperationId;
use crate::domain::table_data::{PageRequest, TablePage};
use crate::errors::AppResult;
use crate::services::table_data;
use crate::state::AppState;

/// One bounded page of a table or view, filtered and sorted as requested.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn read_table_page(
    state: State<'_, AppState>,
    connection_id: ConnectionId,
    request: PageRequest,
) -> AppResult<TablePage> {
    table_data::read_page(
        state.local_store()?,
        state.sessions(),
        connection_id,
        request,
    )
    .await
}

/// Asks the database to stop a running read. `false` if it already finished.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn cancel_operation(
    state: State<'_, AppState>,
    connection_id: ConnectionId,
    operation_id: OperationId,
) -> AppResult<bool> {
    table_data::cancel(state.sessions(), connection_id, operation_id).await
}
