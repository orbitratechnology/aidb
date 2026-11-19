use serde::Serialize;
use tauri::State;

use crate::adapters::credentials::CredentialBackend;
use crate::errors::AppResult;
use crate::state::{AppState, LocalStoreState};

/// Where the app keeps its files and whether its local database is healthy.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsInfo {
    pub data_dir: String,
    pub log_dir: String,
    pub local_store: LocalStoreStatus,
    pub credential_store: CredentialStoreStatus,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct CredentialStoreStatus {
    pub backend: CredentialBackend,
    /// Why secrets are session-only, when they are. Technical; shown as detail only.
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct LocalStoreStatus {
    pub state: LocalStoreHealth,
    /// Applied schema version when the store is ready.
    pub schema_version: Option<u32>,
    /// User-facing reason when the store is unavailable.
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum LocalStoreHealth {
    Ready,
    Unavailable,
}

#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn diagnostics(state: State<'_, AppState>) -> AppResult<DiagnosticsInfo> {
    let local_store = match state.local_store_state() {
        LocalStoreState::Ready(store) => match store
            .schema_version()
            .await
            .map(|version| u32::try_from(version).unwrap_or(u32::MAX))
        {
            Ok(version) => LocalStoreStatus {
                state: LocalStoreHealth::Ready,
                schema_version: Some(version),
                message: None,
            },
            Err(error) => {
                tracing::warn!(error = ?crate::errors::AppError::from(error), "schema version unreadable");
                LocalStoreStatus {
                    state: LocalStoreHealth::Unavailable,
                    schema_version: None,
                    message: Some("The local database could not be read.".to_owned()),
                }
            }
        },
        LocalStoreState::Unavailable { reason } => LocalStoreStatus {
            state: LocalStoreHealth::Unavailable,
            schema_version: None,
            message: Some(reason.clone()),
        },
    };

    let credentials = state.credentials();
    Ok(DiagnosticsInfo {
        data_dir: state.paths().data_dir().display().to_string(),
        log_dir: state.paths().log_dir().display().to_string(),
        local_store,
        credential_store: CredentialStoreStatus {
            backend: credentials.backend(),
            reason: credentials
                .fallback_reason()
                .map(crate::errors::redact)
                .map(Into::into),
        },
    })
}
