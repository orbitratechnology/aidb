//! The append-only audit log. Events carry digests and reasons, never SQL text or values.

use sqlx::SqlitePool;

use super::StoreError;
use crate::domain::audit::{AuditEvent, MAX_DETAIL_CHARS};
use crate::errors::redact;

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
}
