use tauri::State;

use crate::domain::catalog::{ObjectDetails, ObjectName};
use crate::domain::connections::ConnectionId;
use crate::errors::AppResult;
use crate::services::catalog::{self, CatalogSnapshot};
use crate::state::AppState;

/// Schemas, tables and views of the open connection, from the cache when still current.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn catalog_overview(
    state: State<'_, AppState>,
    connection_id: ConnectionId,
) -> AppResult<CatalogSnapshot> {
    let snapshot = catalog::overview(
        state.local_store()?,
        state.sessions(),
        state.catalog(),
        connection_id,
        false,
    )
    .await?;
    Ok((*snapshot).clone())
}

/// Reloads the open connection's schema details from the database.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn refresh_catalog(
    state: State<'_, AppState>,
    connection_id: ConnectionId,
) -> AppResult<CatalogSnapshot> {
    let snapshot = catalog::overview(
        state.local_store()?,
        state.sessions(),
        state.catalog(),
        connection_id,
        true,
    )
    .await?;
    Ok((*snapshot).clone())
}

#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn describe_object(
    state: State<'_, AppState>,
    connection_id: ConnectionId,
    object: ObjectName,
) -> AppResult<ObjectDetails> {
    let details = catalog::describe(
        state.local_store()?,
        state.sessions(),
        state.catalog(),
        connection_id,
        object,
    )
    .await?;
    Ok((*details).clone())
}
