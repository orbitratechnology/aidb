use secrecy::SecretString;
use tauri::State;

use crate::adapters::credentials::CredentialBackend;
use crate::domain::providers::{ProviderDraft, ProviderId};
use crate::errors::AppResult;
use crate::services::providers::{self, ProviderDetails, ProviderTestReport};
use crate::state::AppState;

#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn list_providers(state: State<'_, AppState>) -> AppResult<Vec<ProviderDetails>> {
    providers::list(state.local_store()?, state.credentials()).await
}

#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn create_provider(
    state: State<'_, AppState>,
    provider: ProviderDraft,
) -> AppResult<ProviderDetails> {
    providers::create(state.local_store()?, state.credentials(), provider).await
}

#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn update_provider(
    state: State<'_, AppState>,
    id: ProviderId,
    provider: ProviderDraft,
) -> AppResult<ProviderDetails> {
    providers::update(state.local_store()?, state.credentials(), id, provider).await
}

#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn delete_provider(state: State<'_, AppState>, id: ProviderId) -> AppResult<()> {
    providers::delete(state.local_store()?, state.credentials(), id).await
}

/// Takes the key as a secret, so it is never logged and is zeroed when dropped.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn save_provider_key(
    state: State<'_, AppState>,
    id: ProviderId,
    key: SecretString,
) -> AppResult<CredentialBackend> {
    providers::save_key(state.local_store()?, state.credentials(), id, key).await
}

#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn forget_provider_key(state: State<'_, AppState>, id: ProviderId) -> AppResult<bool> {
    providers::forget_key(state.local_store()?, state.credentials(), id).await
}

/// Sends a few short, fixed prompts to the provider and saves what it supports.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub async fn test_provider(
    state: State<'_, AppState>,
    id: ProviderId,
) -> AppResult<ProviderTestReport> {
    providers::test(state.local_store()?, state.credentials(), id).await
}
