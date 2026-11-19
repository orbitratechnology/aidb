use tauri::State;
use tauri::ipc::Channel;

use crate::domain::agent::ask::{AskRequest, ContextPreview};
use crate::domain::agent::plan::TaskId;
use crate::domain::agent::task::{TaskEvent, TaskOutcome};
use crate::domain::connections::ConnectionId;
use crate::errors::AppResult;
use crate::services::assistant::ask::{self, Workspace};
use crate::state::AppState;

/// What a question would send to the provider, for review. Nothing is sent.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn preview_assistant_context(
    state: State<'_, AppState>,
    connection_id: ConnectionId,
    request: AskRequest,
) -> AppResult<ContextPreview> {
    ask::preview(
        state.local_store()?,
        state.sessions(),
        state.catalog(),
        connection_id,
        request,
    )
    .await
}

/// Answers a question on the open connection, streaming progress to `on_event`.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn ask_assistant(
    state: State<'_, AppState>,
    connection_id: ConnectionId,
    request: AskRequest,
    on_event: Channel<TaskEvent>,
) -> AppResult<TaskOutcome> {
    let workspace = Workspace {
        store: state.local_store()?,
        credentials: state.credentials(),
        sessions: state.sessions(),
        cache: state.catalog(),
        running: state.assistant(),
    };
    ask::ask(workspace, connection_id, request, &mut |event| {
        // The window may have reloaded meanwhile; the task still finishes and is saved.
        if let Err(error) = on_event.send(event) {
            tracing::debug!(error = %error, "assistant event not delivered");
        }
    })
    .await
}

/// Stops a running question. Returns whether it was running.
#[tauri::command]
#[tracing::instrument(skip_all)]
pub fn cancel_assistant_task(state: State<'_, AppState>, task_id: TaskId) -> bool {
    state.assistant().cancel(task_id)
}
