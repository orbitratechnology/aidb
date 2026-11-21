//! Schema comparison: `.sql` files read through the native dialog, comparisons, and saving a
//! comparison's migration script.

use secrecy::SecretString;
use tauri::{Runtime, State, WebviewWindow};

use crate::adapters::files::{self, DialogRequest};
use crate::domain::connections::ConnectionId;
use crate::domain::schema_diff::{CompareRequest, MAX_SCHEMA_FILE_BYTES, SchemaDiff, SchemaFile};
use crate::errors::AppResult;
use crate::services::schema_diff;
use crate::state::AppState;

/// Asks for a `.sql` file and reads it for comparison. `None` when the user cancels.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn open_schema_file<R: Runtime>(
    window: WebviewWindow<R>,
    state: State<'_, AppState>,
) -> AppResult<Option<SchemaFile>> {
    let request = DialogRequest {
        title: "Compare with a SQL file",
        file_name: "",
        filter_name: "SQL",
        extensions: &["sql"],
    };
    let Some((file_name, bytes)) =
        files::open_via_dialog(&window, request, MAX_SCHEMA_FILE_BYTES).await?
    else {
        return Ok(None);
    };
    schema_diff::open_file(state.schema_diffs(), file_name, bytes).map(Some)
}

/// Forgets a file read for comparison.
#[tauri::command]
pub fn close_schema_file(state: State<'_, AppState>, file_id: uuid::Uuid) {
    schema_diff::close_file(state.schema_diffs(), file_id);
}

/// Compares a schema on the open connection with another schema or a file, and returns the
/// steps that make it match. Nothing is changed.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn compare_schemas(
    state: State<'_, AppState>,
    connection_id: ConnectionId,
    request: CompareRequest,
    password: Option<SecretString>,
) -> AppResult<SchemaDiff> {
    let store = state.local_store()?;
    schema_diff::compare(
        store,
        state.credentials(),
        state.sessions(),
        state.schema_diffs(),
        connection_id,
        request,
        password,
    )
    .await
}

/// Saves a comparison's migration script where the user chooses. Returns the file name, or
/// `None` when the user cancels.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn save_schema_script<R: Runtime>(
    window: WebviewWindow<R>,
    state: State<'_, AppState>,
    diff_id: uuid::Uuid,
) -> AppResult<Option<String>> {
    let (file_name, script) = schema_diff::script(state.schema_diffs(), diff_id)?;
    let request = DialogRequest {
        title: "Save migration script",
        file_name: &file_name,
        filter_name: "SQL",
        extensions: &["sql"],
    };
    files::save_via_dialog(&window, request, script.into_bytes()).await
}
