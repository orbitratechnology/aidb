use tauri::{Runtime, State, WebviewWindow};

use crate::adapters::files::{self, DialogRequest};
use crate::domain::connections::ConnectionId;
use crate::domain::export::{EXPORT_MAX_ROWS, ExportFormat, ExportRequest, TableExportOutcome};
use crate::errors::AppResult;
use crate::services::sessions::not_connected;
use crate::services::table_data;
use crate::state::AppState;

/// Asks where to save, then writes every row matching the request (up to [`EXPORT_MAX_ROWS`]) as
/// CSV or JSON. Returns `None` if the user cancelled the dialog. The webview gets only the file
/// name back, never the path.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn export_table<R: Runtime>(
    window: WebviewWindow<R>,
    state: State<'_, AppState>,
    connection_id: ConnectionId,
    request: ExportRequest,
) -> AppResult<Option<TableExportOutcome>> {
    // Fail before showing the dialog when the export could not run anyway.
    request.validate()?;
    if state.sessions().get(connection_id).is_none() {
        return Err(not_connected());
    }
    let suggested = request.file_name();
    let (filter_name, extension) = match request.format {
        ExportFormat::Csv => ("CSV", "csv"),
        ExportFormat::Json => ("JSON", "json"),
    };
    let dialog = DialogRequest {
        title: "Export rows",
        file_name: &suggested,
        filter_name,
        extensions: &[extension],
    };
    let Some(path) = files::pick_save_path(&window, dialog).await? else {
        return Ok(None);
    };
    let summary = table_data::export(
        state.sessions(),
        connection_id,
        &request,
        &path,
        EXPORT_MAX_ROWS,
    )
    .await?;
    Ok(Some(TableExportOutcome {
        file_name: files::file_name(&path),
        rows: summary.rows,
        truncated: summary.truncated,
        max_rows: EXPORT_MAX_ROWS,
    }))
}
