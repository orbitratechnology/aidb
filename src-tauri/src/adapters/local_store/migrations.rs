//! Records of applied schema migrations: a script digest and how it went, never SQL text.

use sqlx::SqlitePool;

use super::StoreError;
use crate::domain::connections::{ConnectionId, Environment};
use crate::domain::migration::{
    MAX_LISTED_RECORDS, MAX_RECORD_DETAIL_CHARS, MigrationRecord, MigrationStatus, Verification,
};
use crate::domain::policy::risk::RiskLevel;
use crate::errors::redact;

type RecordRow = (
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    String,
    i64,
    i64,
    String,
    String,
    Option<String>,
    String,
    String,
    Option<String>,
);

/// A migration about to run.
#[derive(Debug, Clone)]
pub struct NewMigrationRecord<'a> {
    pub id: uuid::Uuid,
    pub connection_id: ConnectionId,
    pub environment: Environment,
    pub target_schema: &'a str,
    pub source: &'a str,
    pub script_sha256: &'a str,
    pub steps: usize,
    pub risk: RiskLevel,
}

/// Repository for `migration_records`.
#[derive(Debug, Clone, Copy)]
pub struct MigrationRecords<'a> {
    pool: &'a SqlitePool,
}

fn detail(text: &str) -> String {
    redact(text).chars().take(MAX_RECORD_DETAIL_CHARS).collect()
}

/// `source` cut to the column's limit.
fn source(text: &str) -> String {
    text.chars().take(400).collect()
}

impl<'a> MigrationRecords<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Records a migration as running, before its first statement.
    ///
    /// # Errors
    ///
    /// A query error.
    pub async fn start(&self, record: &NewMigrationRecord<'_>) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO migration_records \
             (id, connection_id, environment, target_schema, source, script_sha256, steps, risk, \
              status) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'running')",
        )
        .bind(record.id.to_string())
        .bind(record.connection_id.to_string())
        .bind(record.environment.as_str())
        .bind(record.target_schema)
        .bind(source(record.source))
        .bind(record.script_sha256)
        .bind(i64::try_from(record.steps).unwrap_or(i64::MAX))
        .bind(record.risk.as_str())
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Records how a migration ended. The detail is redacted and cut.
    ///
    /// # Errors
    ///
    /// A query error.
    pub async fn finish(
        &self,
        id: uuid::Uuid,
        status: MigrationStatus,
        applied_steps: usize,
        verification: Verification,
        message: Option<&str>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE migration_records SET status = ?, applied_steps = ?, verification = ?, \
             detail = ?, finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(i64::try_from(applied_steps).unwrap_or(i64::MAX))
        .bind(verification.as_str())
        .bind(message.map(detail))
        .bind(id.to_string())
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Records newest first, for one connection or all, at most [`MAX_LISTED_RECORDS`]. Rows
    /// with values this version does not know are skipped.
    ///
    /// # Errors
    ///
    /// A query error.
    pub async fn list(
        &self,
        connection_id: Option<ConnectionId>,
    ) -> Result<Vec<MigrationRecord>, StoreError> {
        let rows: Vec<RecordRow> = sqlx::query_as(
            "SELECT id, connection_id, environment, target_schema, source, script_sha256, steps, \
             applied_steps, risk, started_at, finished_at, status, verification, detail \
             FROM migration_records WHERE ?1 IS NULL OR connection_id = ?1 \
             ORDER BY started_at DESC, rowid DESC LIMIT ?2",
        )
        .bind(connection_id.map(|id| id.to_string()))
        .bind(i64::from(MAX_LISTED_RECORDS))
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().filter_map(record).collect())
    }
}

fn record(row: RecordRow) -> Option<MigrationRecord> {
    let (
        id,
        connection_id,
        environment,
        target_schema,
        source,
        script_sha256,
        steps,
        applied_steps,
        risk,
        started_at,
        finished_at,
        status,
        verification,
        detail,
    ) = row;
    Some(MigrationRecord {
        id: id.parse().ok()?,
        connection_id: match connection_id {
            Some(id) => Some(id.parse().ok()?),
            None => None,
        },
        environment: match environment {
            Some(environment) => Some(environment.parse().ok()?),
            None => None,
        },
        target_schema,
        source,
        script_sha256,
        steps: u32::try_from(steps).ok()?,
        applied_steps: u32::try_from(applied_steps).ok()?,
        risk: RiskLevel::parse(&risk)?,
        started_at,
        finished_at,
        status: MigrationStatus::parse(&status)?,
        verification: Verification::parse(&verification)?,
        detail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::local_store::test_support::temp_store;
    use crate::domain::connections::ConnectionDraft;

    #[tokio::test]
    async fn records_start_running_and_finish_kept_after_the_connection_goes() {
        let (_dir, store) = temp_store().await;
        let profile = store
            .connections()
            .insert(
                &ConnectionDraft::new("Shop", "db", "orders", "owner")
                    .validated()
                    .expect("valid"),
            )
            .await
            .expect("insert");
        let records = store.migrations();
        let id = uuid::Uuid::new_v4();
        records
            .start(&NewMigrationRecord {
                id,
                connection_id: profile.id,
                environment: Environment::Staging,
                target_schema: "public",
                source: "staging on Dev",
                script_sha256: &"a".repeat(64),
                steps: 3,
                risk: RiskLevel::R4,
            })
            .await
            .expect("start");
        let running = records.list(Some(profile.id)).await.expect("list");
        assert_eq!(running[0].status, MigrationStatus::Running);
        assert_eq!(running[0].verification, Verification::NotRun);

        records
            .finish(
                id,
                MigrationStatus::Failed,
                0,
                Verification::NotRun,
                Some("Step 2 of 3: column \"x\" does not exist (password=hunter2)"),
            )
            .await
            .expect("finish");
        store
            .connections()
            .delete(profile.id)
            .await
            .expect("delete");
        let all = records.list(None).await.expect("list");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].status, MigrationStatus::Failed);
        assert!(all[0].finished_at.is_some());
        let detail = all[0].detail.as_deref().unwrap_or_default();
        assert!(!detail.contains("hunter2"), "{detail}");
        // More applied steps than the migration has is refused by the table.
        let refused = records
            .finish(
                id,
                MigrationStatus::Succeeded,
                9,
                Verification::Passed,
                None,
            )
            .await;
        assert!(refused.is_err());
    }
}
