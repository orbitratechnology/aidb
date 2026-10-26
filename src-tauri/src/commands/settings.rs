use serde::Serialize;
use tauri::State;

use crate::adapters::local_store::{ClearTarget, StorageSummary};
use crate::domain::settings::{AppSettings, SettingsPatch};
use crate::errors::AppResult;
use crate::services::settings;
use crate::state::AppState;

#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn get_settings(state: State<'_, AppState>) -> AppResult<AppSettings> {
    settings::load(state.local_store()?).await
}

#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn update_settings(
    state: State<'_, AppState>,
    patch: SettingsPatch,
) -> AppResult<AppSettings> {
    settings::update(state.local_store()?, patch).await
}

#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn storage_summary(state: State<'_, AppState>) -> AppResult<StorageSummary> {
    Ok(state.local_store()?.maintenance().summary().await?)
}

/// How many records a clear removed.
#[derive(Debug, Clone, Copy, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ClearResult {
    pub removed: u32,
}

#[tauri::command]
#[tracing::instrument(skip_all, fields(target = ?target), err(Debug))]
pub async fn clear_local_data(
    state: State<'_, AppState>,
    target: ClearTarget,
) -> AppResult<ClearResult> {
    let removed = state.local_store()?.maintenance().clear(target).await?;
    tracing::info!(?target, removed, "local data cleared by the user");
    Ok(ClearResult { removed })
}
