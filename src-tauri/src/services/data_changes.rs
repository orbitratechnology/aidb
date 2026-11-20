//! Data changes from preview to result: a set of changes is planned against the live tables and
//! held for approval; applying it approves and uses that approval inside the core (no token is
//! ever issued), then runs the changes in one checked transaction and records the outcome.

use super::approvals::{self, Approvals};
use super::audit;
use super::sessions::{Sessions, not_connected};
use crate::adapters::database::{
    ApplyError, DatabaseAdapter, DatabaseSession, DbError, DbErrorKind,
};
use crate::adapters::local_store::LocalStore;
use crate::domain::audit::{Actor, AuditEvent, AuditStatus};
use crate::domain::connections::ConnectionId;
use crate::domain::data_change::{
    ApplyStatus, ChangeKind, DataChange, DataChangePreview, DataChangeResult, PREVIEW_BYTES,
    PREVIEW_ROWS, PendingChange, PlannedChange, ValueSample, validate_set,
};
use crate::domain::policy::approval::{ApprovalError, ApprovalId, ApprovalState, sha256_hex};
use crate::domain::policy::classify::RiskAssessment;
use crate::domain::table_data::PageLimits;
use crate::errors::{AppError, AppResult, ErrorCode};

const PREVIEW_LIMITS: PageLimits = PageLimits {
    max_rows: PREVIEW_ROWS,
    max_bytes: PREVIEW_BYTES,
};

/// What a set of changes does, for the audit log: "Update of public.orders", or "3 changes to
/// public.orders: 2 updates, 1 delete".
fn describe(changes: &[DataChange]) -> String {
    let name = |change: &DataChange| {
        let table = change.table();
        format!("{}.{}", table.schema, table.name)
    };
    if let [change] = changes {
        return format!("{} of {}", change.kind().label(), name(change));
    }
    let mut tables: Vec<String> = changes.iter().map(name).collect();
    tables.sort();
    tables.dedup();
    let parts: Vec<String> = [
        (ChangeKind::Update, "update", "updates"),
        (ChangeKind::Delete, "delete", "deletes"),
        (ChangeKind::Insert, "insert", "inserts"),
    ]
    .into_iter()
    .filter_map(|(kind, one, many)| {
        let count = changes
            .iter()
            .filter(|change| change.kind() == kind)
            .count();
        match count {
            0 => None,
            1 => Some(format!("1 {one}")),
            _ => Some(format!("{count} {many}")),
        }
    })
    .collect();
    format!(
        "{} changes to {}: {}",
        changes.len(),
        tables.join(", "),
        parts.join(", ")
    )
}

/// ", proposed by the assistant", for changes the user did not write.
fn proposed_by(actor: Actor) -> &'static str {
    match actor {
        Actor::Agent => ", proposed by the assistant",
        Actor::User | Actor::System => "",
    }
}

fn reasons(assessment: &RiskAssessment) -> Vec<String> {
    let mut reasons: Vec<String> = Vec::new();
    for reason in assessment
        .statements
        .iter()
        .flat_map(|statement| &statement.reasons)
        .chain(&assessment.reasons)
    {
        // Each edit of a set says the same thing; once is enough.
        if !reasons.contains(&reason.message) {
            reasons.push(reason.message.clone());
        }
    }
    reasons
}

/// Plans `changes`, applied together in this order, on the open connection, and holds them for
/// the user's approval.
///
/// # Errors
///
/// `invalid_input` for a malformed change, an unknown column, a value its column cannot hold or
/// too many matching rows (naming the change); `permission_denied` when the policy or the
/// role's privileges refuse them here (audited); `not_found` for an unknown table;
/// `unavailable` when not connected.
pub async fn preview<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    approvals: &Approvals,
    connection_id: ConnectionId,
    changes: Vec<DataChange>,
    actor: Actor,
) -> AppResult<DataChangePreview> {
    validate_set(&changes)?;
    let fingerprint = serde_json::to_vec(&changes)
        .map(|json| sha256_hex(&json))
        .ok();
    let description = describe(&changes);
    let requested = approvals::request_data_changes(
        store,
        sessions,
        approvals,
        connection_id,
        changes.clone(),
        PREVIEW_LIMITS,
        actor,
    )
    .await;
    let (request, plans) = match requested {
        Ok(requested) => requested,
        Err(error) => {
            if error.code() == ErrorCode::PermissionDenied {
                let environment = store
                    .connections()
                    .get(connection_id)
                    .await
                    .ok()
                    .flatten()
                    .map(|profile| profile.settings.environment);
                let event = AuditEvent {
                    connection_id: Some(connection_id),
                    environment,
                    actor,
                    operation: "data_change",
                    risk: None,
                    fingerprint,
                    approval_state: None,
                    status: AuditStatus::Rejected,
                    rows_affected: None,
                    detail: Some(format!("{description} refused: {}", error.message())),
                };
                audit::record(store, &event).await;
            }
            return Err(error);
        }
    };
    let planned: Vec<PlannedChange> = changes
        .iter()
        .zip(plans)
        .map(|(change, plan)| PlannedChange {
            kind: change.kind(),
            table: change.table().clone(),
            after: (change.kind() != ChangeKind::Delete)
                .then(|| ValueSample::of(&plan.value_columns, &plan.values)),
            statement: plan.statement,
            rows: plan.rows,
            before: plan.before,
        })
        .collect();
    let preview = DataChangePreview {
        approval_id: request.id,
        rows: planned.iter().map(|change| change.rows).sum(),
        changes: planned,
        risk: request.assessment.risk,
        approval: request.level,
        reasons: reasons(&request.assessment),
        confirmation: request.confirmation,
        expires_at: request.expires_at,
    };
    approvals.keep_preview(&preview);
    Ok(preview)
}

/// Data changes waiting for the user's decision, on one connection or all.
#[must_use]
pub fn pending(approvals: &Approvals, connection_id: Option<ConnectionId>) -> Vec<PendingChange> {
    approvals.pending_data_changes(connection_id)
}

fn no_such_change() -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        ApprovalError::Unknown.to_string(),
    )
}

/// The user approves previewed changes, which then run at once, together. Elevated changes need
/// the database name typed in `confirmation`.
///
/// Refusals before anything runs are errors. Once the changes run, the result says whether they
/// were applied, rolled back (nothing written) or are uncertain (the connection was lost while
/// committing); every outcome is audited.
///
/// # Errors
///
/// `permission_denied` for an unknown, expired, used or foreign approval, or when anything it
/// was bound to changed (it is invalidated); `invalid_input` when the typed confirmation is
/// missing or wrong (it stays open).
pub async fn apply<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    approvals: &Approvals,
    connection_id: ConnectionId,
    approval_id: ApprovalId,
    confirmation: Option<&str>,
) -> AppResult<DataChangeResult> {
    let Some((_, changes, proposer)) = approvals.data_changes(approval_id) else {
        return Err(no_such_change());
    };
    let operation = approvals::approve_and_use(
        store,
        sessions,
        approvals,
        connection_id,
        approval_id,
        confirmation,
    )
    .await?;

    let outcome = match sessions.get(connection_id) {
        Some(_) if operation.plans.len() != changes.len() => Err(ApplyError::RolledBack(
            DbError::new(DbErrorKind::Other, "The changes were not planned."),
        )),
        Some(session) => session.apply_data_changes(&changes, &operation.plans).await,
        None => {
            let _ = approvals::finish(approvals, operation.id, ApprovalState::Failed);
            return Err(not_connected());
        }
    };
    let (result, state, status) = match outcome {
        Ok(rows) => (
            DataChangeResult {
                status: ApplyStatus::Applied,
                rows_affected: Some(rows),
                message: None,
            },
            ApprovalState::Succeeded,
            AuditStatus::Succeeded,
        ),
        Err(ApplyError::RolledBack(error)) => (
            DataChangeResult {
                status: ApplyStatus::RolledBack,
                rows_affected: None,
                message: Some(error.message().to_owned()),
            },
            ApprovalState::Failed,
            AuditStatus::Failed,
        ),
        Err(ApplyError::Uncertain(error)) => {
            tracing::warn!(error = %error, "the commit of a data change is uncertain");
            (
                DataChangeResult {
                    status: ApplyStatus::Uncertain,
                    rows_affected: None,
                    message: Some(
                        "The connection was lost while the changes were being committed, so they \
                         may or may not have been written. Check the table before trying again."
                            .to_owned(),
                    ),
                },
                ApprovalState::Failed,
                AuditStatus::Failed,
            )
        }
    };
    if let Err(error) = approvals::finish(approvals, operation.id, state) {
        tracing::warn!(error = %error, "could not record how a data change ended");
    }
    let outcome_text = match result.status {
        ApplyStatus::Applied => "applied",
        ApplyStatus::RolledBack => "rolled back, nothing written",
        ApplyStatus::Uncertain => "uncertain, the connection was lost while committing",
    };
    let event = AuditEvent {
        connection_id: Some(connection_id),
        environment: Some(operation.environment),
        actor: Actor::User,
        operation: "data_change",
        risk: Some(operation.assessment.risk),
        fingerprint: Some(operation.digest),
        approval_state: Some(state.as_str()),
        status,
        rows_affected: result.rows_affected,
        detail: Some(format!(
            "{}{}: {outcome_text}.",
            describe(&changes),
            proposed_by(proposer)
        )),
    };
    audit::record(store, &event).await;
    Ok(result)
}

/// The user declines previewed changes.
///
/// # Errors
///
/// `permission_denied` for an unknown approval, one that is not for data changes, or one that
/// is already running.
pub async fn cancel(
    store: &LocalStore,
    approvals: &Approvals,
    approval_id: ApprovalId,
) -> AppResult<()> {
    let Some((connection_id, changes, proposer)) = approvals.data_changes(approval_id) else {
        return Err(no_such_change());
    };
    approvals::cancel(approvals, approval_id)?;
    let environment = store
        .connections()
        .get(connection_id)
        .await
        .ok()
        .flatten()
        .map(|profile| profile.settings.environment);
    let event = AuditEvent {
        connection_id: Some(connection_id),
        environment,
        actor: Actor::User,
        operation: "data_change",
        risk: None,
        fingerprint: None,
        approval_state: Some(ApprovalState::Cancelled.as_str()),
        status: AuditStatus::Cancelled,
        rows_affected: None,
        detail: Some(format!(
            "{}{}: declined.",
            describe(&changes),
            proposed_by(proposer)
        )),
    };
    audit::record(store, &event).await;
    Ok(())
}

#[cfg(test)]
mod tests;
