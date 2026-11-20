//! Applying a schema migration the user reviewed: a held comparison is prepared for approval,
//! approved with the typed database name (and, when steps delete data, an explicit
//! acknowledgement), applied in one transaction, checked by comparing again, and recorded.

use std::sync::Arc;

use super::approvals::{self, Approvals};
use super::audit;
use super::schema_diff::{HeldDiff, SchemaDiffs};
use super::sessions::{Sessions, not_connected};
use crate::adapters::database::{
    DatabaseAdapter, DatabaseSession, MigrationRun, MigrationStatement,
};
use crate::adapters::local_store::{LocalStore, NewMigrationRecord};
use crate::domain::audit::{Actor, AuditEvent, AuditStatus};
use crate::domain::connections::ConnectionId;
use crate::domain::migration::{
    FailedStep, MigrationPreview, MigrationRecord, MigrationResult, MigrationStatus, Verification,
};
use crate::domain::policy::approval::{ApprovalId, ApprovalState, sha256_hex};
use crate::domain::schema_diff::generate::plan;
use crate::domain::schema_diff::{NoteKind, SchemaDiff};
use crate::errors::{AppError, AppResult};

fn no_such_comparison() -> AppError {
    AppError::not_found("That comparison is no longer open. Compare again.")
}

/// Summaries of the steps that delete data.
fn data_loss(diff: &SchemaDiff) -> Vec<String> {
    diff.steps
        .iter()
        .filter(|step| step.notes.iter().any(|n| n.kind == NoteKind::DataLoss))
        .map(|step| step.summary.clone())
        .collect()
}

fn held_for(connection_id: ConnectionId, held: Option<HeldDiff>) -> AppResult<HeldDiff> {
    held.filter(|held| held.connection_id == connection_id)
        .ok_or_else(no_such_comparison)
}

/// Why the migration has its risk: the set's reasons, then each different statement reason
/// once.
fn reasons(assessment: &crate::domain::policy::classify::RiskAssessment) -> Vec<String> {
    let mut reasons: Vec<String> = Vec::new();
    let all = assessment
        .reasons
        .iter()
        .chain(assessment.statements.iter().flat_map(|s| s.reasons.iter()));
    for reason in all {
        if !reasons.contains(&reason.message) {
            reasons.push(reason.message.clone());
        }
    }
    reasons
}

/// Prepares a held comparison's migration for approval on the open connection.
///
/// # Errors
///
/// `not_found` when the comparison is no longer held; `invalid_input` when there is nothing to
/// apply; `permission_denied` when the policy refuses it here (a read-only connection, a step it
/// does not allow); `unavailable` when not connected.
pub async fn prepare<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    approvals: &Approvals,
    diffs: &SchemaDiffs,
    connection_id: ConnectionId,
    diff_id: uuid::Uuid,
) -> AppResult<MigrationPreview> {
    let held = held_for(connection_id, diffs.get(diff_id))?;
    let diff = &held.diff;
    if diff.steps.is_empty() {
        return Err(AppError::invalid_input(
            "The schemas match: there is nothing to apply.",
        ));
    }
    let statements = Arc::new(diff.steps.iter().map(|step| step.sql.clone()).collect());
    let request =
        approvals::request_migration(store, sessions, approvals, connection_id, statements).await?;
    diffs.prepare(request.id, diff_id);
    Ok(MigrationPreview {
        approval_id: request.id,
        diff_id,
        steps: diff.steps.len(),
        data_loss: data_loss(diff),
        outside_transaction: diff.steps.iter().filter(|s| !s.transactional).count(),
        risk: request.assessment.risk,
        approval: request.level,
        reasons: reasons(&request.assessment),
        confirmation: request.confirmation,
        expires_at: request.expires_at,
    })
}

/// The user's decision on a prepared migration.
#[derive(Debug, Clone, Copy)]
pub struct Decision<'a> {
    pub approval_id: ApprovalId,
    /// The database name, for elevated approvals.
    pub confirmation: Option<&'a str>,
    /// Required when steps delete data.
    pub accept_data_loss: bool,
}

/// The user's approval of a prepared migration, which then runs: the steps that can, in one
/// transaction, then the others. `accept_data_loss` must be set when steps delete data.
///
/// # Errors
///
/// `not_found` for an unknown approval or comparison; `invalid_input` when data loss is not
/// acknowledged (the approval stays open); the approval's errors (wrong name, expired, the
/// schema changed). A migration that fails is not an error: the result says how it ended.
pub async fn apply<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    approvals: &Approvals,
    diffs: &SchemaDiffs,
    connection_id: ConnectionId,
    decision: Decision<'_>,
) -> AppResult<MigrationResult> {
    let Decision {
        approval_id,
        confirmation,
        accept_data_loss,
    } = decision;
    let held = held_for(connection_id, diffs.prepared(approval_id))?;
    let Some((_, statements)) = approvals.migration(approval_id) else {
        return Err(no_such_comparison());
    };
    let diff = Arc::clone(&held.diff);
    if !data_loss(&diff).is_empty() && !accept_data_loss {
        return Err(AppError::invalid_input(
            "Some steps delete data. Confirm that you accept this before applying.",
        ));
    }
    let operation = approvals::approve_and_use(
        store,
        sessions,
        approvals,
        connection_id,
        approval_id,
        confirmation,
    )
    .await?;
    let Some(session) = sessions.get(connection_id) else {
        let _ = approvals::finish(approvals, operation.id, ApprovalState::Failed);
        return Err(not_connected());
    };
    let fingerprint = match session.catalog_fingerprint().await {
        Ok(fingerprint) => fingerprint,
        Err(error) => {
            let _ = approvals::finish(approvals, operation.id, ApprovalState::Failed);
            return Err(error.into());
        }
    };

    let record_id = uuid::Uuid::new_v4();
    let records = store.migrations();
    records
        .start(&NewMigrationRecord {
            id: record_id,
            connection_id,
            environment: operation.environment,
            target_schema: &diff.target_schema,
            source: &diff.source,
            script_sha256: &sha256_hex(diff.script.as_bytes()),
            steps: diff.steps.len(),
            risk: operation.assessment.risk,
        })
        .await?;

    let run_statements: Vec<MigrationStatement> = diff
        .steps
        .iter()
        .zip(statements.iter())
        .map(|(step, sql)| MigrationStatement {
            sql: sql.clone(),
            transactional: step.transactional,
        })
        .collect();
    let run = session
        .apply_migration(&diff.target_schema, &run_statements, &fingerprint)
        .await;
    let total = diff.steps.len();
    let failed = |step: usize, message: &str| FailedStep {
        number: step + 1,
        summary: diff.steps[step].summary.clone(),
        message: message.to_owned(),
    };
    let (status, applied, failed_step, message) = match &run {
        MigrationRun::Applied => (
            MigrationStatus::Succeeded,
            total,
            None,
            format!(
                "Applied {}.",
                if total == 1 {
                    "1 step".to_owned()
                } else {
                    format!("{total} steps")
                }
            ),
        ),
        MigrationRun::RolledBack {
            step: Some(step),
            error,
        } => (
            MigrationStatus::Failed,
            0,
            Some(failed(*step, error.message())),
            format!(
                "Step {} of {total} failed, so the transaction was rolled back and nothing \
                 changed: {}",
                step + 1,
                error.message()
            ),
        ),
        MigrationRun::RolledBack { step: None, error } => (
            MigrationStatus::Failed,
            0,
            None,
            format!("Nothing changed: {}", error.message()),
        ),
        MigrationRun::Partial {
            applied,
            step,
            error,
        } => (
            MigrationStatus::Partial,
            *applied,
            Some(failed(*step, error.message())),
            format!(
                "The transaction committed, but step {} of {total}, which runs after it, failed: \
                 {} The steps after it did not run.",
                step + 1,
                error.message()
            ),
        ),
        MigrationRun::Uncertain(error) => {
            tracing::warn!(error = %error, "the commit of a migration is uncertain");
            (
                MigrationStatus::Uncertain,
                0,
                None,
                "The connection was lost while the migration was being committed, so it may or \
                 may not have been applied. Compare again to see where the schema stands."
                    .to_owned(),
            )
        }
    };

    // Compare again: the migration did its job when nothing is left to do.
    let (verification, remaining) = match status {
        MigrationStatus::Succeeded | MigrationStatus::Partial => {
            match session.read_schema(&diff.target_schema).await {
                Ok(target) => {
                    let left = plan(&held.source, &target, held.options, operation.environment)
                        .steps
                        .len();
                    if left == 0 {
                        (Verification::Passed, 0)
                    } else {
                        (Verification::Failed, left)
                    }
                }
                Err(error) => {
                    tracing::warn!(error = %error, "could not compare after a migration");
                    (Verification::NotRun, 0)
                }
            }
        }
        _ => (Verification::NotRun, 0),
    };
    let message = match verification {
        Verification::Failed => format!(
            "{message} Compared again, {remaining} {} still differ.",
            if remaining == 1 { "thing" } else { "things" }
        ),
        Verification::Passed => format!("{message} Compared again, the schemas now match."),
        Verification::NotRun => message,
    };

    let state = if status == MigrationStatus::Succeeded {
        ApprovalState::Succeeded
    } else {
        ApprovalState::Failed
    };
    if let Err(error) = approvals::finish(approvals, operation.id, state) {
        tracing::warn!(error = %error, "could not record how a migration ended");
    }
    if let Err(error) = records
        .finish(record_id, status, applied, verification, Some(&message))
        .await
    {
        tracing::warn!(error = %error, "could not record how a migration ended");
    }
    let event = AuditEvent {
        connection_id: Some(connection_id),
        environment: Some(operation.environment),
        actor: Actor::User,
        operation: "migration",
        risk: Some(operation.assessment.risk),
        fingerprint: Some(operation.digest),
        approval_state: Some(state.as_str()),
        status: match status {
            MigrationStatus::Succeeded => AuditStatus::Succeeded,
            _ => AuditStatus::Failed,
        },
        rows_affected: None,
        detail: Some(format!(
            "Migration of {} to match {}: {} ({applied} of {total} steps).",
            diff.target,
            diff.source,
            status.as_str()
        )),
    };
    audit::record(store, &event).await;
    // Anything but a clean rollback changed the schema the comparison describes.
    if status != MigrationStatus::Failed {
        diffs.forget(diff.diff_id);
    }
    Ok(MigrationResult {
        record_id,
        status,
        applied_steps: applied,
        total_steps: total,
        failed_step,
        verification,
        remaining,
        message,
    })
}

/// Declines a prepared migration.
///
/// # Errors
///
/// `not_found` for an unknown approval.
pub fn cancel(approvals: &Approvals, approval_id: ApprovalId) -> AppResult<()> {
    if approvals.migration(approval_id).is_none() {
        return Err(no_such_comparison());
    }
    approvals::cancel(approvals, approval_id)
}

/// Applied migrations, newest first, for one connection or all.
///
/// # Errors
///
/// A local store error.
pub async fn list(
    store: &LocalStore,
    connection_id: Option<ConnectionId>,
) -> AppResult<Vec<MigrationRecord>> {
    Ok(store.migrations().list(connection_id).await?)
}

#[cfg(test)]
mod tests;
