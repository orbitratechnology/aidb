//! Saved AI provider profiles. API keys are never stored; only a credential-store reference.

use sqlx::{FromRow, SqlitePool};

use super::StoreError;
use crate::domain::providers::{CheckedFeatures, ProviderDraft, ProviderId, ProviderProfile};

// Literal SQL only: SQLx 0.9 rejects dynamic strings unless explicitly audited.
macro_rules! select_providers {
    ($tail:literal) => {
        concat!(
            "SELECT id, name, kind, endpoint, model, secret_ref, request_timeout_secs, ",
            "context_budget_tokens, allow_insecure_http, capabilities_json, created_at, ",
            "updated_at FROM provider_profiles ",
            $tail
        )
    };
}

/// Repository for `provider_profiles`.
#[derive(Debug, Clone, Copy)]
pub struct ProviderProfiles<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ProviderProfiles<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Saves a new profile. The draft must already be validated.
    ///
    /// # Errors
    ///
    /// [`StoreError::Duplicate`] if another profile has the same name (case-insensitive).
    pub async fn insert(&self, draft: &ProviderDraft) -> Result<ProviderProfile, StoreError> {
        let id = ProviderId::new();
        sqlx::query(
            "INSERT INTO provider_profiles (id, name, kind, endpoint, model, \
             request_timeout_secs, context_budget_tokens, allow_insecure_http) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(&draft.name)
        .bind(draft.kind.as_str())
        .bind(draft.endpoint.as_deref())
        .bind(&draft.model)
        .bind(i64::from(draft.request_timeout_secs))
        .bind(draft.context_budget_tokens.map(i64::from))
        .bind(draft.allow_insecure_http)
        .execute(self.pool)
        .await
        .map_err(name_conflict)?;
        self.get(id).await?.ok_or(StoreError::NotFound)
    }

    /// # Errors
    ///
    /// Fails on a query error or a stored row that no longer matches the domain types.
    pub async fn get(&self, id: ProviderId) -> Result<Option<ProviderProfile>, StoreError> {
        let row: Option<ProviderRow> = sqlx::query_as(select_providers!("WHERE id = ?"))
            .bind(id.to_string())
            .fetch_optional(self.pool)
            .await?;
        row.map(ProviderProfile::try_from).transpose()
    }

    /// All profiles, ordered by name.
    ///
    /// # Errors
    ///
    /// Fails on a query error or a stored row that no longer matches the domain types.
    pub async fn list(&self) -> Result<Vec<ProviderProfile>, StoreError> {
        let rows: Vec<ProviderRow> =
            sqlx::query_as(select_providers!("ORDER BY name COLLATE NOCASE, id"))
                .fetch_all(self.pool)
                .await?;
        rows.into_iter().map(ProviderProfile::try_from).collect()
    }

    /// Replaces the editable fields of a profile. The draft must already be validated. The last
    /// check's results are dropped when the kind, endpoint or model changes, since they described
    /// another service.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] if the profile does not exist; [`StoreError::Duplicate`] if the
    /// new name is taken.
    pub async fn update(
        &self,
        id: ProviderId,
        draft: &ProviderDraft,
    ) -> Result<ProviderProfile, StoreError> {
        // In an UPDATE, column names on the right-hand side read the row's old values.
        let result = sqlx::query(
            "UPDATE provider_profiles SET \
             capabilities_json = CASE WHEN kind = ?1 AND endpoint IS ?2 AND model = ?3 \
                 THEN capabilities_json ELSE NULL END, \
             name = ?4, kind = ?1, endpoint = ?2, model = ?3, request_timeout_secs = ?5, \
             context_budget_tokens = ?6, allow_insecure_http = ?7, \
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
             WHERE id = ?8",
        )
        .bind(draft.kind.as_str())
        .bind(draft.endpoint.as_deref())
        .bind(&draft.model)
        .bind(&draft.name)
        .bind(i64::from(draft.request_timeout_secs))
        .bind(draft.context_budget_tokens.map(i64::from))
        .bind(draft.allow_insecure_http)
        .bind(id.to_string())
        .execute(self.pool)
        .await
        .map_err(name_conflict)?;
        if result.rows_affected() == 0 {
            return Err(StoreError::NotFound);
        }
        self.get(id).await?.ok_or(StoreError::NotFound)
    }

    /// Records (or clears) the credential-store reference for a profile's API key.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] if the profile does not exist.
    pub async fn set_secret_ref(
        &self,
        id: ProviderId,
        secret_ref: Option<&str>,
    ) -> Result<(), StoreError> {
        let result = sqlx::query(
            "UPDATE provider_profiles SET secret_ref = ?, \
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        )
        .bind(secret_ref)
        .bind(id.to_string())
        .execute(self.pool)
        .await?;
        if result.rows_affected() == 0 {
            Err(StoreError::NotFound)
        } else {
            Ok(())
        }
    }

    /// Records what "Test provider" found.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] if the profile does not exist.
    pub async fn set_checked(
        &self,
        id: ProviderId,
        checked: &CheckedFeatures,
    ) -> Result<(), StoreError> {
        let json = serde_json::to_string(checked)
            .map_err(|e| StoreError::Corrupt(format!("provider capabilities: {e}")))?;
        let result = sqlx::query("UPDATE provider_profiles SET capabilities_json = ? WHERE id = ?")
            .bind(json)
            .bind(id.to_string())
            .execute(self.pool)
            .await?;
        if result.rows_affected() == 0 {
            Err(StoreError::NotFound)
        } else {
            Ok(())
        }
    }

    /// Deletes a profile. Returns whether it existed.
    ///
    /// # Errors
    ///
    /// Fails on a query error.
    pub async fn delete(&self, id: ProviderId) -> Result<bool, StoreError> {
        let result = sqlx::query("DELETE FROM provider_profiles WHERE id = ?")
            .bind(id.to_string())
            .execute(self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}

fn name_conflict(error: sqlx::Error) -> StoreError {
    match &error {
        sqlx::Error::Database(db) if db.is_unique_violation() => StoreError::Duplicate("name"),
        _ => StoreError::Query(error),
    }
}

#[derive(FromRow)]
struct ProviderRow {
    id: String,
    name: String,
    kind: String,
    endpoint: Option<String>,
    model: String,
    secret_ref: Option<String>,
    request_timeout_secs: i64,
    context_budget_tokens: Option<i64>,
    allow_insecure_http: bool,
    capabilities_json: Option<String>,
    created_at: String,
    updated_at: String,
}

impl TryFrom<ProviderRow> for ProviderProfile {
    type Error = StoreError;

    fn try_from(row: ProviderRow) -> Result<Self, Self::Error> {
        fn corrupt(field: &str, error: impl std::fmt::Display) -> StoreError {
            StoreError::Corrupt(format!("provider profile {field}: {error}"))
        }

        // A capability record that no longer parses is only a cache: drop it.
        let checked = row
            .capabilities_json
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok());
        Ok(Self {
            id: row.id.parse().map_err(|e| corrupt("id", e))?,
            settings: ProviderDraft {
                name: row.name,
                kind: row.kind.parse().map_err(|e| corrupt("kind", e))?,
                endpoint: row.endpoint,
                model: row.model,
                request_timeout_secs: u32::try_from(row.request_timeout_secs)
                    .map_err(|e| corrupt("request_timeout_secs", e))?,
                context_budget_tokens: row
                    .context_budget_tokens
                    .map(u32::try_from)
                    .transpose()
                    .map_err(|e| corrupt("context_budget_tokens", e))?,
                allow_insecure_http: row.allow_insecure_http,
            },
            secret_ref: row.secret_ref,
            checked,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::temp_store;
    use super::*;
    use crate::domain::ai::StructuredOutput;
    use crate::domain::providers::{ProviderFeatures, ProviderPreset};

    fn draft(name: &str) -> ProviderDraft {
        ProviderDraft::new(name, ProviderPreset::Ollama, "qwen2.5")
            .validated()
            .expect("valid draft")
    }

    fn checked() -> CheckedFeatures {
        CheckedFeatures {
            features: ProviderFeatures {
                tool_calls: false,
                structured_output: StructuredOutput::JsonObject,
            },
            checked_at: "2026-11-17T10:00:00Z".into(),
        }
    }

    #[tokio::test]
    async fn profiles_round_trip() {
        let (_dir, store) = temp_store().await;
        let saved = store
            .providers()
            .insert(&draft("Local"))
            .await
            .expect("insert");
        let loaded = store
            .providers()
            .get(saved.id)
            .await
            .expect("get")
            .expect("exists");
        assert_eq!(loaded, saved);
        assert_eq!(loaded.settings.kind, ProviderPreset::Ollama);
        assert_eq!((loaded.secret_ref, loaded.checked), (None, None));

        store
            .providers()
            .set_secret_ref(saved.id, Some("provider:x"))
            .await
            .expect("secret ref");
        store
            .providers()
            .set_checked(saved.id, &checked())
            .await
            .expect("checked");
        let loaded = store
            .providers()
            .get(saved.id)
            .await
            .expect("get")
            .expect("exists");
        assert_eq!(loaded.secret_ref.as_deref(), Some("provider:x"));
        assert_eq!(loaded.checked, Some(checked()));
    }

    #[tokio::test]
    async fn names_are_unique_and_lists_sorted() {
        let (_dir, store) = temp_store().await;
        for name in ["zeta", "Alpha"] {
            store
                .providers()
                .insert(&draft(name))
                .await
                .expect("insert");
        }
        assert!(matches!(
            store.providers().insert(&draft("ALPHA")).await,
            Err(StoreError::Duplicate("name"))
        ));
        let names: Vec<String> = store
            .providers()
            .list()
            .await
            .expect("list")
            .into_iter()
            .map(|p| p.settings.name)
            .collect();
        assert_eq!(names, ["Alpha", "zeta"]);
    }

    #[tokio::test]
    async fn checks_survive_renames_but_not_a_new_model() {
        let (_dir, store) = temp_store().await;
        let saved = store
            .providers()
            .insert(&draft("Local"))
            .await
            .expect("insert");
        store
            .providers()
            .set_checked(saved.id, &checked())
            .await
            .expect("checked");

        let renamed = store
            .providers()
            .update(saved.id, &draft("Local Ollama"))
            .await
            .expect("rename");
        assert_eq!(renamed.checked, Some(checked()));

        let mut other_model = draft("Local Ollama");
        other_model.model = "llama3.2".into();
        let changed = store
            .providers()
            .update(saved.id, &other_model)
            .await
            .expect("update");
        assert_eq!(changed.checked, None);

        assert!(matches!(
            store
                .providers()
                .update(ProviderId::new(), &draft("x"))
                .await,
            Err(StoreError::NotFound)
        ));
        assert!(store.providers().delete(saved.id).await.expect("delete"));
        assert!(!store.providers().delete(saved.id).await.expect("delete"));
    }

    #[tokio::test]
    async fn a_damaged_capability_record_is_ignored() {
        let (_dir, store) = temp_store().await;
        let saved = store
            .providers()
            .insert(&draft("Local"))
            .await
            .expect("insert");
        sqlx::query("UPDATE provider_profiles SET capabilities_json = '{\"x\":1}' WHERE id = ?")
            .bind(saved.id.to_string())
            .execute(store.pool())
            .await
            .expect("damage");
        let loaded = store
            .providers()
            .get(saved.id)
            .await
            .expect("get")
            .expect("exists");
        assert_eq!(loaded.checked, None);
    }
}
