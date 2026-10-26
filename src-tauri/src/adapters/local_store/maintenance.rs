//! What the local database holds, and the user's controls for clearing it.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::StoreError;

/// Row counts per kind of local data. Never row contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct StorageSummary {
    pub connections: u32,
    pub saved_queries: u32,
    pub agent_tasks: u32,
    pub audit_records: u32,
    pub schema_cache_entries: u32,
}

/// Local data the user can clear without touching connections or saved queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ClearTarget {
    TaskHistory,
    AuditLog,
    SchemaCache,
}

impl ClearTarget {
    const fn delete_sql(self) -> &'static str {
        match self {
            Self::TaskHistory => "DELETE FROM agent_tasks",
            Self::AuditLog => "DELETE FROM operation_audit",
            Self::SchemaCache => "DELETE FROM schema_cache_metadata",
        }
    }
}

/// Repository for summaries and clearing.
#[derive(Debug, Clone, Copy)]
pub struct Maintenance<'a> {
    pool: &'a SqlitePool,
}

impl<'a> Maintenance<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// # Errors
    ///
    /// Fails on a query error.
    pub async fn summary(&self) -> Result<StorageSummary, StoreError> {
        let (connections, saved_queries, agent_tasks, audit_records, schema_cache_entries): (
            i64,
            i64,
            i64,
            i64,
            i64,
        ) = sqlx::query_as(
            "SELECT \
                 (SELECT COUNT(*) FROM connection_profiles), \
                 (SELECT COUNT(*) FROM saved_queries), \
                 (SELECT COUNT(*) FROM agent_tasks), \
                 (SELECT COUNT(*) FROM operation_audit), \
                 (SELECT COUNT(*) FROM schema_cache_metadata)",
        )
        .fetch_one(self.pool)
        .await?;
        Ok(StorageSummary {
            connections: count(connections),
            saved_queries: count(saved_queries),
            agent_tasks: count(agent_tasks),
            audit_records: count(audit_records),
            schema_cache_entries: count(schema_cache_entries),
        })
    }

    /// Deletes every row of one kind. Returns how many were removed.
    ///
    /// # Errors
    ///
    /// Fails on a query error.
    pub async fn clear(&self, target: ClearTarget) -> Result<u32, StoreError> {
        let result = sqlx::query(target.delete_sql()).execute(self.pool).await?;
        Ok(u32::try_from(result.rows_affected()).unwrap_or(u32::MAX))
    }
}

fn count(value: i64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::temp_store;
    use super::*;
    use crate::domain::connections::ConnectionDraft;

    async fn seed(store: &super::super::LocalStore) {
        let profile = store
            .connections()
            .insert(
                &ConnectionDraft::new("Orders", "db", "orders", "reader")
                    .validated()
                    .expect("valid"),
            )
            .await
            .expect("insert");
        let id = profile.id.to_string();
        for sql in [
            "INSERT INTO saved_queries (id, connection_id, name, sql_text) VALUES ('q', ?1, 'n', 'select 1')",
            "INSERT INTO agent_tasks (id, connection_id, title, status) VALUES ('t1', ?1, 'a', 'succeeded')",
            "INSERT INTO agent_tasks (id, connection_id, title, status) VALUES ('t2', ?1, 'b', 'failed')",
            "INSERT INTO operation_audit (connection_id, actor, operation, status) VALUES (?1, 'user', 'read', 'succeeded')",
            "INSERT INTO schema_cache_metadata (connection_id, schema_fingerprint, object_count) VALUES (?1, 'f', 2)",
        ] {
            sqlx::query(sql)
                .bind(&id)
                .execute(store.pool())
                .await
                .expect("seed");
        }
    }

    #[tokio::test]
    async fn summary_counts_each_kind_of_data() {
        let (_dir, store) = temp_store().await;
        seed(&store).await;

        let summary = store.maintenance().summary().await.expect("summary");

        assert_eq!(
            summary,
            StorageSummary {
                connections: 1,
                saved_queries: 1,
                agent_tasks: 2,
                audit_records: 1,
                schema_cache_entries: 1,
            }
        );
    }

    #[tokio::test]
    async fn clearing_removes_only_the_chosen_kind() {
        let (_dir, store) = temp_store().await;
        seed(&store).await;

        let removed_tasks = store
            .maintenance()
            .clear(ClearTarget::TaskHistory)
            .await
            .expect("clear tasks");
        let removed_audit = store
            .maintenance()
            .clear(ClearTarget::AuditLog)
            .await
            .expect("clear audit");
        let removed_cache = store
            .maintenance()
            .clear(ClearTarget::SchemaCache)
            .await
            .expect("clear cache");

        let summary = store.maintenance().summary().await.expect("summary");
        assert_eq!((removed_tasks, removed_audit, removed_cache), (2, 1, 1));
        assert_eq!(summary.connections, 1);
        assert_eq!(summary.saved_queries, 1);
        assert_eq!(
            (
                summary.agent_tasks,
                summary.audit_records,
                summary.schema_cache_entries
            ),
            (0, 0, 0)
        );
    }

    #[test]
    fn clear_targets_use_their_wire_names() {
        assert_eq!(
            serde_json::to_value(ClearTarget::TaskHistory).expect("serializes"),
            "task_history"
        );
        assert_eq!(
            serde_json::from_str::<ClearTarget>(r#""schema_cache""#).expect("parses"),
            ClearTarget::SchemaCache
        );
        assert!(serde_json::from_str::<ClearTarget>(r#""connections""#).is_err());
    }
}
