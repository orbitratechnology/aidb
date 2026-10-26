//! Reading and changing app settings.
//!
//! Loading never fails on a bad stored value: an unreadable or out-of-range section falls back to
//! its defaults (logged), so a damaged preference cannot lock the user out of the app.

use serde::de::DeserializeOwned;

use crate::adapters::local_store::{LocalStore, StoreError};
use crate::domain::settings::{AppSettings, SafetySettings, SettingsPatch, keys};
use crate::errors::AppResult;

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
    Ok(AppSettings { appearance, safety })
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
    if let Some(appearance) = &patch.appearance {
        store.settings().set(keys::APPEARANCE, appearance).await?;
    }
    if let Some(safety) = &patch.safety {
        store.settings().set(keys::SAFETY, safety).await?;
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
}
