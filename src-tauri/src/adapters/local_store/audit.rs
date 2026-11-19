//! The append-only audit log. Events carry digests and reasons, never SQL text or values.

use sqlx::SqlitePool;

use super::StoreError;
use crate::domain::audit::{
    Actor, AuditEntry, AuditEvent, AuditFilter, AuditStatus, MAX_DETAIL_CHARS, MAX_LISTED_EVENTS,
};
use crate::domain::policy::risk::RiskLevel;
use crate::errors::redact;

type AuditRow = (
    i64,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
    Option<i64>,
    Option<String>,
);

/// Repository for `operation_audit`.
#[derive(Debug, Clone, Copy)]
pub struct AuditLog<'a> {
    pool: &'a SqlitePool,
}

impl<'a> AuditLog<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Appends an event. The detail is redacted and cut to [`MAX_DETAIL_CHARS`].
    ///
    /// # Errors
    ///
    /// A query error.
    pub async fn record(&self, event: &AuditEvent) -> Result<i64, StoreError> {
        let detail = event.detail.as_deref().map(|detail| {
            redact(detail)
                .chars()
                .take(MAX_DETAIL_CHARS)
                .collect::<String>()
        });
        let rows = event
            .rows_affected
            .and_then(|rows| i64::try_from(rows).ok());
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO operation_audit \
             (connection_id, environment, actor, operation, risk, fingerprint, approval_state, \
              status, rows_affected, detail) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(event.connection_id.map(|id| id.to_string()))
        .bind(event.environment.map(|environment| environment.as_str()))
        .bind(event.actor.as_str())
        .bind(event.operation)
        .bind(event.risk.map(|risk| risk.as_str()))
        .bind(event.fingerprint.as_deref())
        .bind(event.approval_state)
        .bind(event.status.as_str())
        .bind(rows)
        .bind(detail)
        .fetch_one(self.pool)
        .await?;
        Ok(id)
    }
}

impl AuditLog<'_> {
    /// Events newest first, as `filter` asks. Rows a later version wrote with values this one
    /// does not know are skipped.
    ///
    /// # Errors
    ///
    /// A query error.
    pub async fn list(&self, filter: &AuditFilter) -> Result<Vec<AuditEntry>, StoreError> {
        let rows: Vec<AuditRow> = sqlx::query_as(
            "SELECT id, occurred_at, connection_id, environment, actor, operation, risk, \
             fingerprint, approval_state, status, rows_affected, detail \
             FROM operation_audit \
             WHERE (?1 IS NULL OR connection_id = ?1) AND (?2 IS NULL OR id < ?2) \
             ORDER BY id DESC LIMIT ?3",
        )
        .bind(filter.connection_id.map(|id| id.to_string()))
        .bind(filter.before_id)
        .bind(i64::from(filter.limit.clamp(1, MAX_LISTED_EVENTS)))
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().filter_map(entry).collect())
    }
}

fn entry(row: AuditRow) -> Option<AuditEntry> {
    let (
        id,
        occurred_at,
        connection,
        environment,
        actor,
        operation,
        risk,
        fingerprint,
        state,
        status,
        rows,
        detail,
    ) = row;
    Some(AuditEntry {
        id,
        occurred_at,
        connection_id: connection.and_then(|id| id.parse().ok()),
        environment: environment.and_then(|environment| environment.parse().ok()),
        actor: Actor::parse(&actor)?,
        operation,
        risk: risk.as_deref().and_then(RiskLevel::parse),
        fingerprint,
        approval_state: state,
        status: AuditStatus::parse(&status)?,
        rows_affected: rows.and_then(|rows| u64::try_from(rows).ok()),
        detail,
    })
}

#[cfg(test)]
mod tests {
    use super::super::test_support::temp_store;
    use crate::domain::audit::{Actor, AuditEvent, AuditStatus};
    use crate::domain::connections::{ConnectionId, Environment};
    use crate::domain::policy::risk::RiskLevel;

    #[tokio::test]
    async fn events_are_appended_redacted_and_never_edited() {
        let (_dir, store) = temp_store().await;
        let id = store
            .audit()
            .record(&AuditEvent {
                connection_id: Some(ConnectionId::new()),
                environment: Some(Environment::Production),
                actor: Actor::User,
                operation: "approval_check",
                risk: Some(RiskLevel::R3),
                fingerprint: Some("ab".repeat(32)),
                approval_state: Some("approved"),
                status: AuditStatus::Rejected,
                rows_affected: None,
                detail: Some(format!(
                    "password=hunter2 {}",
                    "x".repeat(super::MAX_DETAIL_CHARS * 2)
                )),
            })
            .await
            .expect("record");
        let (status, detail): (String, String) =
            sqlx::query_as("SELECT status, detail FROM operation_audit WHERE id = ?")
                .bind(id)
                .fetch_one(store.pool())
                .await
                .expect("read");
        assert_eq!(status, "rejected");
        assert!(!detail.contains("hunter2"), "{detail:.80}");
        assert!(detail.chars().count() <= super::MAX_DETAIL_CHARS);
        let edit = sqlx::query("UPDATE operation_audit SET status = 'succeeded' WHERE id = ?")
            .bind(id)
            .execute(store.pool())
            .await;
        assert!(edit.is_err(), "the audit log is append-only");
    }

    #[tokio::test]
    async fn events_are_listed_newest_first_by_connection() {
        use crate::domain::audit::AuditFilter;

        let (_dir, store) = temp_store().await;
        let shop = ConnectionId::new();
        for (connection, operation) in [
            (Some(shop), "approve"),
            (None, "tool_call"),
            (Some(shop), "approval_check"),
            (Some(ConnectionId::new()), "sql_refused"),
        ] {
            store
                .audit()
                .record(&AuditEvent {
                    connection_id: connection,
                    environment: Some(Environment::Staging),
                    actor: Actor::Agent,
                    operation,
                    risk: Some(RiskLevel::R2),
                    fingerprint: None,
                    approval_state: None,
                    status: AuditStatus::Succeeded,
                    rows_affected: Some(3),
                    detail: None,
                })
                .await
                .expect("record");
        }
        let all = store
            .audit()
            .list(&AuditFilter {
                connection_id: None,
                before_id: None,
                limit: 10,
            })
            .await
            .expect("list");
        assert_eq!(
            all.iter().map(|e| e.operation.as_str()).collect::<Vec<_>>(),
            ["sql_refused", "approval_check", "tool_call", "approve"]
        );
        assert_eq!(all[0].environment, Some(Environment::Staging));
        assert_eq!(all[0].risk, Some(RiskLevel::R2));
        assert_eq!(all[0].rows_affected, Some(3));

        let filter = AuditFilter {
            connection_id: Some(shop),
            before_id: None,
            limit: 1,
        };
        let first = store.audit().list(&filter).await.expect("page");
        assert_eq!(first[0].operation, "approval_check");
        let next = store
            .audit()
            .list(&AuditFilter {
                before_id: Some(first[0].id),
                ..filter
            })
            .await
            .expect("next page");
        assert_eq!(next[0].operation, "approve");
        assert_eq!(next[0].connection_id, Some(shop));
    }
}
