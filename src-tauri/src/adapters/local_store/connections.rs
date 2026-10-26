//! Saved connection profiles. Passwords are never stored; only a credential-store reference.

use sqlx::{FromRow, SqlitePool};

use super::StoreError;
use crate::domain::connections::{ConnectionDraft, ConnectionId, ConnectionProfile};

// Literal SQL only: SQLx 0.9 rejects dynamic strings unless explicitly audited.
macro_rules! select_profiles {
    ($tail:literal) => {
        concat!(
            "SELECT id, engine, name, host, port, database_name, username, ssl_mode, ",
            "ca_cert_path, connect_timeout_secs, environment, read_only, secret_ref, ",
            "created_at, updated_at FROM connection_profiles ",
            $tail
        )
    };
}

/// Repository for `connection_profiles`.
#[derive(Debug, Clone, Copy)]
pub struct ConnectionProfiles<'a> {
    pool: &'a SqlitePool,
}

impl<'a> ConnectionProfiles<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Saves a new profile. The draft must already be validated.
    ///
    /// # Errors
    ///
    /// [`StoreError::Duplicate`] if another profile has the same name (case-insensitive).
    pub async fn insert(&self, draft: &ConnectionDraft) -> Result<ConnectionProfile, StoreError> {
        let id = ConnectionId::new();
        sqlx::query(
            "INSERT INTO connection_profiles (id, name, host, port, database_name, username, \
             ssl_mode, ca_cert_path, connect_timeout_secs, environment, read_only) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(&draft.name)
        .bind(&draft.host)
        .bind(i64::from(draft.port))
        .bind(&draft.database)
        .bind(&draft.username)
        .bind(draft.ssl_mode.as_str())
        .bind(draft.ca_cert_path.as_deref())
        .bind(i64::from(draft.connect_timeout_secs))
        .bind(draft.environment.as_str())
        .bind(draft.read_only)
        .execute(self.pool)
        .await
        .map_err(name_conflict)?;
        self.get(id).await?.ok_or(StoreError::NotFound)
    }

    /// # Errors
    ///
    /// Fails on a query error or a stored row that no longer matches the domain types.
    pub async fn get(&self, id: ConnectionId) -> Result<Option<ConnectionProfile>, StoreError> {
        let row: Option<ProfileRow> = sqlx::query_as(select_profiles!("WHERE id = ?"))
            .bind(id.to_string())
            .fetch_optional(self.pool)
            .await?;
        row.map(ConnectionProfile::try_from).transpose()
    }

    /// All profiles, ordered by name.
    ///
    /// # Errors
    ///
    /// Fails on a query error or a stored row that no longer matches the domain types.
    pub async fn list(&self) -> Result<Vec<ConnectionProfile>, StoreError> {
        let rows: Vec<ProfileRow> =
            sqlx::query_as(select_profiles!("ORDER BY name COLLATE NOCASE, id"))
                .fetch_all(self.pool)
                .await?;
        rows.into_iter().map(ConnectionProfile::try_from).collect()
    }

    /// Replaces the editable fields of a profile. The draft must already be validated.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] if the profile does not exist; [`StoreError::Duplicate`] if the
    /// new name is taken.
    pub async fn update(
        &self,
        id: ConnectionId,
        draft: &ConnectionDraft,
    ) -> Result<ConnectionProfile, StoreError> {
        let result = sqlx::query(
            "UPDATE connection_profiles SET name = ?, host = ?, port = ?, database_name = ?, \
             username = ?, ssl_mode = ?, ca_cert_path = ?, connect_timeout_secs = ?, \
             environment = ?, read_only = ?, \
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
             WHERE id = ?",
        )
        .bind(&draft.name)
        .bind(&draft.host)
        .bind(i64::from(draft.port))
        .bind(&draft.database)
        .bind(&draft.username)
        .bind(draft.ssl_mode.as_str())
        .bind(draft.ca_cert_path.as_deref())
        .bind(i64::from(draft.connect_timeout_secs))
        .bind(draft.environment.as_str())
        .bind(draft.read_only)
        .bind(id.to_string())
        .execute(self.pool)
        .await
        .map_err(name_conflict)?;
        if result.rows_affected() == 0 {
            return Err(StoreError::NotFound);
        }
        self.get(id).await?.ok_or(StoreError::NotFound)
    }

    /// Records (or clears) the credential-store reference for a profile's password.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] if the profile does not exist.
    pub async fn set_secret_ref(
        &self,
        id: ConnectionId,
        secret_ref: Option<&str>,
    ) -> Result<(), StoreError> {
        let result = sqlx::query(
            "UPDATE connection_profiles SET secret_ref = ?, \
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

    /// Deletes a profile and its cached schema metadata. Saved queries and task history stay,
    /// detached from the connection. Returns whether the profile existed.
    ///
    /// # Errors
    ///
    /// Fails on a query error.
    pub async fn delete(&self, id: ConnectionId) -> Result<bool, StoreError> {
        let result = sqlx::query("DELETE FROM connection_profiles WHERE id = ?")
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
struct ProfileRow {
    id: String,
    engine: String,
    name: String,
    host: String,
    port: i64,
    database_name: String,
    username: String,
    ssl_mode: String,
    ca_cert_path: Option<String>,
    connect_timeout_secs: i64,
    environment: String,
    read_only: bool,
    secret_ref: Option<String>,
    created_at: String,
    updated_at: String,
}

impl TryFrom<ProfileRow> for ConnectionProfile {
    type Error = StoreError;

    fn try_from(row: ProfileRow) -> Result<Self, Self::Error> {
        fn corrupt(field: &str, error: impl std::fmt::Display) -> StoreError {
            StoreError::Corrupt(format!("connection profile {field}: {error}"))
        }

        Ok(Self {
            id: row.id.parse().map_err(|e| corrupt("id", e))?,
            engine: row.engine.parse().map_err(|e| corrupt("engine", e))?,
            settings: ConnectionDraft {
                name: row.name,
                host: row.host,
                port: u16::try_from(row.port).map_err(|e| corrupt("port", e))?,
                database: row.database_name,
                username: row.username,
                ssl_mode: row.ssl_mode.parse().map_err(|e| corrupt("ssl_mode", e))?,
                ca_cert_path: row.ca_cert_path,
                connect_timeout_secs: u32::try_from(row.connect_timeout_secs)
                    .map_err(|e| corrupt("connect_timeout_secs", e))?,
                environment: row
                    .environment
                    .parse()
                    .map_err(|e| corrupt("environment", e))?,
                read_only: row.read_only,
            },
            secret_ref: row.secret_ref,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::temp_store;
    use super::*;
    use crate::domain::connections::{Environment, SslMode};

    fn draft(name: &str) -> ConnectionDraft {
        ConnectionDraft::new(name, "db.internal", "orders", "reader")
            .validated()
            .expect("valid draft")
    }

    #[tokio::test]
    async fn inserted_profiles_read_back_with_safe_defaults() {
        let (_dir, store) = temp_store().await;

        let saved = store
            .connections()
            .insert(&draft("Orders"))
            .await
            .expect("insert");
        let loaded = store
            .connections()
            .get(saved.id)
            .await
            .expect("get")
            .expect("exists");

        assert_eq!(loaded, saved);
        assert!(loaded.settings.read_only);
        assert_eq!(loaded.settings.ssl_mode, SslMode::VerifyFull);
        assert_eq!(loaded.secret_ref, None);
        assert!(loaded.created_at.ends_with('Z'));
    }

    #[tokio::test]
    async fn names_are_unique_regardless_of_case() {
        let (_dir, store) = temp_store().await;
        store
            .connections()
            .insert(&draft("Orders"))
            .await
            .expect("first");

        let duplicate = store.connections().insert(&draft("orders")).await;

        assert!(matches!(duplicate, Err(StoreError::Duplicate("name"))));
    }

    #[tokio::test]
    async fn list_is_sorted_by_name() {
        let (_dir, store) = temp_store().await;
        for name in ["zeta", "Alpha", "beta"] {
            store
                .connections()
                .insert(&draft(name))
                .await
                .expect("insert");
        }

        let names: Vec<String> = store
            .connections()
            .list()
            .await
            .expect("list")
            .into_iter()
            .map(|profile| profile.settings.name)
            .collect();

        assert_eq!(names, ["Alpha", "beta", "zeta"]);
    }

    #[tokio::test]
    async fn updates_replace_fields_and_bump_updated_at() {
        let (_dir, store) = temp_store().await;
        let saved = store
            .connections()
            .insert(&draft("Orders"))
            .await
            .expect("insert");
        let mut changes = saved.settings.clone();
        changes.environment = Environment::Production;
        changes.read_only = false;
        changes.port = 6432;

        let updated = store
            .connections()
            .update(saved.id, &changes)
            .await
            .expect("update");

        assert_eq!(updated.settings, changes);
        assert_eq!(updated.created_at, saved.created_at);
        assert!(updated.updated_at >= saved.updated_at);
    }

    #[tokio::test]
    async fn missing_profiles_are_reported() {
        let (_dir, store) = temp_store().await;
        let id = ConnectionId::new();

        assert_eq!(store.connections().get(id).await.expect("get"), None);
        assert!(matches!(
            store.connections().update(id, &draft("x")).await,
            Err(StoreError::NotFound)
        ));
        assert!(matches!(
            store.connections().set_secret_ref(id, Some("ref")).await,
            Err(StoreError::NotFound)
        ));
        assert!(!store.connections().delete(id).await.expect("delete"));
    }

    #[tokio::test]
    async fn secret_refs_are_stored_as_references_only() {
        let (_dir, store) = temp_store().await;
        let saved = store
            .connections()
            .insert(&draft("Orders"))
            .await
            .expect("insert");

        store
            .connections()
            .set_secret_ref(saved.id, Some("aidb/connection/abc"))
            .await
            .expect("set ref");
        let with_ref = store.connections().get(saved.id).await.expect("get");
        store
            .connections()
            .set_secret_ref(saved.id, None)
            .await
            .expect("clear ref");
        let cleared = store.connections().get(saved.id).await.expect("get");

        assert_eq!(
            with_ref.and_then(|p| p.secret_ref).as_deref(),
            Some("aidb/connection/abc")
        );
        assert_eq!(cleared.and_then(|p| p.secret_ref), None);
    }

    #[tokio::test]
    async fn deleting_a_profile_drops_its_cache_and_detaches_history() {
        let (_dir, store) = temp_store().await;
        let saved = store
            .connections()
            .insert(&draft("Orders"))
            .await
            .expect("insert");
        let id = saved.id.to_string();
        sqlx::query(
            "INSERT INTO schema_cache_metadata (connection_id, schema_fingerprint, object_count) VALUES (?, 'f', 3)",
        )
        .bind(&id)
        .execute(store.pool())
        .await
        .expect("cache row");
        sqlx::query(
            "INSERT INTO saved_queries (id, connection_id, name, sql_text) VALUES ('q1', ?, 'Recent', 'select 1')",
        )
        .bind(&id)
        .execute(store.pool())
        .await
        .expect("saved query");

        assert!(store.connections().delete(saved.id).await.expect("delete"));

        let cache_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM schema_cache_metadata")
            .fetch_one(store.pool())
            .await
            .expect("count");
        let detached: Option<String> =
            sqlx::query_scalar("SELECT connection_id FROM saved_queries WHERE id = 'q1'")
                .fetch_one(store.pool())
                .await
                .expect("saved query kept");
        assert_eq!(cache_rows, 0);
        assert_eq!(detached, None);
    }
}
