//! App preferences stored as JSON values under short dotted keys (for example `ui.theme`).

use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::SqlitePool;

use super::StoreError;

const KEY_MAX: usize = 64;

/// Repository for `app_settings`.
#[derive(Debug, Clone, Copy)]
pub struct Settings<'a> {
    pool: &'a SqlitePool,
}

impl<'a> Settings<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Reads and decodes a setting, or `None` if it was never set.
    ///
    /// # Errors
    ///
    /// Fails on an invalid key, a query error, or a stored value of a different shape.
    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, StoreError> {
        check_key(key)?;
        let raw: Option<String> =
            sqlx::query_scalar("SELECT value_json FROM app_settings WHERE key = ?")
                .bind(key)
                .fetch_optional(self.pool)
                .await?;
        raw.map(|json| {
            serde_json::from_str(&json)
                .map_err(|error| StoreError::Corrupt(format!("setting `{key}`: {error}")))
        })
        .transpose()
    }

    /// Creates or replaces a setting.
    ///
    /// # Errors
    ///
    /// Fails on an invalid key, a value that cannot be encoded, or a query error.
    pub async fn set<T: Serialize + ?Sized>(&self, key: &str, value: &T) -> Result<(), StoreError> {
        check_key(key)?;
        let json = serde_json::to_string(value)
            .map_err(|error| StoreError::Corrupt(format!("setting `{key}`: {error}")))?;
        sqlx::query(
            "INSERT INTO app_settings (key, value_json) VALUES (?, ?)
             ON CONFLICT (key) DO UPDATE SET
                 value_json = excluded.value_json,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )
        .bind(key)
        .bind(json)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Deletes a setting. Returns whether it existed.
    ///
    /// # Errors
    ///
    /// Fails on an invalid key or a query error.
    pub async fn remove(&self, key: &str) -> Result<bool, StoreError> {
        check_key(key)?;
        let result = sqlx::query("DELETE FROM app_settings WHERE key = ?")
            .bind(key)
            .execute(self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

/// Keys are lowercase ASCII words separated by dots or underscores, starting with a letter.
fn check_key(key: &str) -> Result<(), StoreError> {
    let valid = key.len() <= KEY_MAX
        && key.starts_with(|c: char| c.is_ascii_lowercase())
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_');
    if valid {
        Ok(())
    } else {
        Err(StoreError::InvalidKey(key.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::super::test_support::temp_store;
    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Limits {
        max_rows: u32,
        timeout_secs: u32,
    }

    #[tokio::test]
    async fn settings_round_trip_typed_values() {
        let (_dir, store) = temp_store().await;
        let settings = store.settings();
        let limits = Limits {
            max_rows: 500,
            timeout_secs: 30,
        };

        settings
            .set("safety.read_limits", &limits)
            .await
            .expect("set");

        assert_eq!(
            settings
                .get::<Limits>("safety.read_limits")
                .await
                .expect("get"),
            Some(limits)
        );
        assert_eq!(
            settings.get::<Limits>("safety.missing").await.expect("get"),
            None
        );
    }

    #[tokio::test]
    async fn setting_a_key_again_replaces_the_value() {
        let (_dir, store) = temp_store().await;
        let settings = store.settings();

        settings.set("ui.theme", "light").await.expect("first set");
        settings.set("ui.theme", "dark").await.expect("second set");

        assert_eq!(
            settings
                .get::<String>("ui.theme")
                .await
                .expect("get")
                .as_deref(),
            Some("dark")
        );
    }

    #[tokio::test]
    async fn removing_reports_whether_the_key_existed() {
        let (_dir, store) = temp_store().await;
        let settings = store.settings();
        settings.set("ui.theme", "dark").await.expect("set");

        assert!(settings.remove("ui.theme").await.expect("remove"));
        assert!(!settings.remove("ui.theme").await.expect("remove again"));
    }

    #[tokio::test]
    async fn malformed_keys_are_rejected() {
        let (_dir, store) = temp_store().await;

        for key in [
            "",
            "UI.theme",
            "1theme",
            "ui theme",
            "ui/theme",
            &"k".repeat(65),
        ] {
            assert!(
                matches!(
                    store.settings().set(key, &true).await,
                    Err(StoreError::InvalidKey(_))
                ),
                "{key:?} must be rejected"
            );
        }
    }

    #[tokio::test]
    async fn a_value_of_the_wrong_shape_is_reported_as_corrupt() {
        let (_dir, store) = temp_store().await;
        store
            .settings()
            .set("safety.read_limits", "not an object")
            .await
            .expect("set");

        let result = store.settings().get::<Limits>("safety.read_limits").await;

        assert!(matches!(result, Err(StoreError::Corrupt(_))));
    }
}
