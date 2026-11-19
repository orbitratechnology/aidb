//! Reading and changing app settings.
//!
//! Loading never fails on a bad stored value: an unreadable or out-of-range section falls back to
//! its defaults (logged), so a damaged preference cannot lock the user out of the app.

use serde::de::DeserializeOwned;

use crate::adapters::local_store::{LocalStore, StoreError};
use crate::domain::settings::{AiSettings, AppSettings, SafetySettings, SettingsPatch, keys};
use crate::errors::{AppError, AppResult};

/// Current settings, with defaults for anything never saved.
///
/// # Errors
///
/// `storage` if the local database cannot be read at all.
pub async fn load(store: &LocalStore) -> AppResult<AppSettings> {
    let appearance = section(store, keys::APPEARANCE).await?;
    let safety: SafetySettings = section(store, keys::SAFETY).await?;
    let safety = match safety.validate() {
        Ok(()) => safety,
        Err(error) => {
            tracing::warn!(error = %error, "stored safety settings out of range; using defaults");
            SafetySettings::default()
        }
    };
    let ai: AiSettings = section(store, keys::AI).await?;
    // A provider deleted outside the app (or a damaged row) turns AI off rather than failing.
    let ai = match ai.provider_id {
        Some(id) if store.providers().get(id).await?.is_none() => {
            tracing::warn!("the chosen AI provider no longer exists; AI is off");
            AiSettings::default()
        }
        _ => ai,
    };
    Ok(AppSettings {
        appearance,
        safety,
        ai,
    })
}

/// Turns AI off and forgets the chosen provider if it is `id`. Called when a provider is deleted.
///
/// # Errors
///
/// `storage` if the settings cannot be read or saved.
pub async fn forget_provider(
    store: &LocalStore,
    id: crate::domain::providers::ProviderId,
) -> AppResult<()> {
    let ai: AiSettings = section(store, keys::AI).await?;
    if ai.provider_id == Some(id) {
        store
            .settings()
            .set(keys::AI, &AiSettings::default())
            .await?;
        tracing::info!("the chosen AI provider was deleted; AI is off");
    }
    Ok(())
}

/// Replaces the sections present in `patch`, after validating them, and returns the result.
///
/// # Errors
///
/// `invalid_input` if a value is out of range (nothing is saved), `storage` if saving fails.
pub async fn update(store: &LocalStore, patch: SettingsPatch) -> AppResult<AppSettings> {
    if let Some(safety) = &patch.safety {
        safety.validate()?;
    }
    if let Some(ai) = &patch.ai {
        match ai.provider_id {
            None if ai.enabled => {
                return Err(AppError::invalid_input(
                    "Choose a provider before turning on AI.",
                ));
            }
            Some(id) if store.providers().get(id).await?.is_none() => {
                return Err(AppError::not_found("That provider no longer exists."));
            }
            _ => {}
        }
    }
    if let Some(appearance) = &patch.appearance {
        store.settings().set(keys::APPEARANCE, appearance).await?;
    }
    if let Some(safety) = &patch.safety {
        store.settings().set(keys::SAFETY, safety).await?;
    }
    if let Some(ai) = &patch.ai {
        store.settings().set(keys::AI, ai).await?;
        tracing::info!(enabled = ai.enabled, "AI settings saved");
    }
    load(store).await
}

async fn section<T: DeserializeOwned + Default>(store: &LocalStore, key: &str) -> AppResult<T> {
    match store.settings().get::<T>(key).await {
        Ok(value) => Ok(value.unwrap_or_default()),
        Err(StoreError::Corrupt(detail)) => {
            tracing::warn!(key, detail, "stored setting unreadable; using defaults");
            Ok(T::default())
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::local_store::test_support::temp_store;
    use crate::domain::settings::{AppearanceSettings, ThemePreference};
    use crate::errors::ErrorCode;

    #[tokio::test]
    async fn a_fresh_store_returns_defaults() {
        let (_dir, store) = temp_store().await;

        assert_eq!(load(&store).await.expect("load"), AppSettings::default());
    }

    #[tokio::test]
    async fn patches_replace_only_the_sections_they_contain() {
        let (_dir, store) = temp_store().await;
        let safety = SafetySettings {
            max_rows_per_page: 1000,
            ..SafetySettings::default()
        };
        update(
            &store,
            SettingsPatch {
                safety: Some(safety.clone()),
                ..SettingsPatch::default()
            },
        )
        .await
        .expect("save safety");

        let updated = update(
            &store,
            SettingsPatch {
                appearance: Some(AppearanceSettings {
                    theme: ThemePreference::Dark,
                }),
                ..SettingsPatch::default()
            },
        )
        .await
        .expect("save appearance");

        assert_eq!(updated.appearance.theme, ThemePreference::Dark);
        assert_eq!(updated.safety, safety);
        assert_eq!(load(&store).await.expect("reload"), updated);
    }

    #[tokio::test]
    async fn invalid_patches_save_nothing() {
        let (_dir, store) = temp_store().await;
        let patch = SettingsPatch {
            appearance: Some(AppearanceSettings {
                theme: ThemePreference::Light,
            }),
            safety: Some(SafetySettings {
                statement_timeout_secs: 0,
                ..SafetySettings::default()
            }),
            ai: None,
        };

        let error = update(&store, patch).await.expect_err("rejected");

        assert_eq!(error.code(), ErrorCode::InvalidInput);
        assert_eq!(load(&store).await.expect("load"), AppSettings::default());
    }

    #[tokio::test]
    async fn damaged_stored_values_fall_back_to_defaults() {
        let (_dir, store) = temp_store().await;
        store
            .settings()
            .set(keys::APPEARANCE, &"not an object")
            .await
            .expect("store junk");
        store
            .settings()
            .set(
                keys::SAFETY,
                &serde_json::json!({ "maxRowsPerPage": 99_999_999 }),
            )
            .await
            .expect("store out-of-range value");

        assert_eq!(load(&store).await.expect("load"), AppSettings::default());
    }

    #[tokio::test]
    async fn ai_needs_an_existing_provider_and_turns_off_without_it() {
        use crate::domain::providers::{ProviderDraft, ProviderId, ProviderPreset};

        let (_dir, store) = temp_store().await;
        assert_eq!(load(&store).await.expect("load").ai, AiSettings::default());

        let on_without_provider = SettingsPatch {
            ai: Some(AiSettings {
                enabled: true,
                provider_id: None,
            }),
            ..SettingsPatch::default()
        };
        let error = update(&store, on_without_provider)
            .await
            .expect_err("needs a provider");
        assert_eq!(error.code(), ErrorCode::InvalidInput);
        let unknown = SettingsPatch {
            ai: Some(AiSettings {
                enabled: true,
                provider_id: Some(ProviderId::new()),
            }),
            ..SettingsPatch::default()
        };
        assert_eq!(
            update(&store, unknown).await.expect_err("unknown").code(),
            ErrorCode::NotFound
        );

        let provider = store
            .providers()
            .insert(
                &ProviderDraft::new("Local", ProviderPreset::Ollama, "qwen2.5")
                    .validated()
                    .expect("valid"),
            )
            .await
            .expect("provider");
        let on = AiSettings {
            enabled: true,
            provider_id: Some(provider.id),
        };
        let saved = update(
            &store,
            SettingsPatch {
                ai: Some(on),
                ..SettingsPatch::default()
            },
        )
        .await
        .expect("on");
        assert_eq!(saved.ai, on);

        // Deleting the chosen provider turns AI off.
        store.providers().delete(provider.id).await.expect("delete");
        assert_eq!(load(&store).await.expect("load").ai, AiSettings::default());
        forget_provider(&store, provider.id).await.expect("forget");
        let stored: Option<AiSettings> = store.settings().get(keys::AI).await.expect("get");
        assert_eq!(stored, Some(AiSettings::default()));
    }
}
