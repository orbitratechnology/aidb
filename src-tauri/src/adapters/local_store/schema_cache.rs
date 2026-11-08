//! When each connection's schema details were last read, and their fingerprint. Only this
//! freshness record is stored; the details themselves stay in memory.

use sqlx::SqlitePool;

use super::StoreError;
use crate::domain::connections::ConnectionId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaCacheRecord {
    pub fingerprint: String,
    pub object_count: u32,
    pub refreshed_at: String,
}

/// Repository for `schema_cache_metadata`.
#[derive(Debug, Clone, Copy)]
pub struct SchemaCache<'a> {
    pool: &'a SqlitePool,
}

impl<'a> SchemaCache<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Records a fresh read of a connection's schema.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] if the profile no longer exists; otherwise a query error.
    pub async fn record(
        &self,
        id: ConnectionId,
        fingerprint: &str,
        object_count: u32,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO schema_cache_metadata (connection_id, schema_fingerprint, object_count) \
             VALUES (?, ?, ?) \
             ON CONFLICT (connection_id) DO UPDATE SET \
                 schema_fingerprint = excluded.schema_fingerprint, \
                 object_count = excluded.object_count, \
                 refreshed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        )
        .bind(id.to_string())
        .bind(fingerprint)
        .bind(i64::from(object_count))
        .execute(self.pool)
        .await
        .map_err(|error| match &error {
            sqlx::Error::Database(db) if db.is_foreign_key_violation() => StoreError::NotFound,
            _ => StoreError::Query(error),
        })?;
        Ok(())
    }

    /// # Errors
    ///
    /// Fails on a query error.
    pub async fn get(&self, id: ConnectionId) -> Result<Option<SchemaCacheRecord>, StoreError> {
        let row: Option<(String, i64, String)> = sqlx::query_as(
            "SELECT schema_fingerprint, object_count, refreshed_at \
             FROM schema_cache_metadata WHERE connection_id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(self.pool)
        .await?;
        Ok(
            row.map(|(fingerprint, count, refreshed_at)| SchemaCacheRecord {
                fingerprint,
                object_count: u32::try_from(count).unwrap_or(u32::MAX),
                refreshed_at,
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::temp_store;
    use super::*;
    use crate::domain::connections::ConnectionDraft;

    #[tokio::test]
    async fn records_are_upserted_per_connection() {
        let (_dir, store) = temp_store().await;
        let profile = store
            .connections()
            .insert(
                &ConnectionDraft::new("Orders", "db", "orders", "reader")
                    .validated()
                    .expect("valid"),
            )
            .await
            .expect("insert");

        store
            .schema_cache()
            .record(profile.id, "a", 3)
            .await
            .expect("first");
        store
            .schema_cache()
            .record(profile.id, "b", 5)
            .await
            .expect("second");

        let record = store
            .schema_cache()
            .get(profile.id)
            .await
            .expect("get")
            .expect("exists");
        assert_eq!((record.fingerprint.as_str(), record.object_count), ("b", 5));
        assert_eq!(
            store
                .maintenance()
                .summary()
                .await
                .expect("summary")
                .schema_cache_entries,
            1
        );
    }

    #[tokio::test]
    async fn unknown_connections_are_reported() {
        let (_dir, store) = temp_store().await;

        let result = store
            .schema_cache()
            .record(ConnectionId::new(), "a", 1)
            .await;

        assert!(matches!(result, Err(StoreError::NotFound)));
    }
}
