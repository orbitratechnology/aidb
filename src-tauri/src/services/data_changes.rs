//! Data changes from preview to result: a change is planned against the live table and held
//! for approval; applying it approves and uses that approval inside the core (the token never
//! leaves it), then runs the change in one checked transaction and records the outcome.

use secrecy::SecretString;

use super::approvals::{self, Approvals, Subject};
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
    PREVIEW_ROWS, ValueSample,
};
use crate::domain::policy::approval::{ApprovalError, ApprovalId, ApprovalState, sha256_hex};
use crate::domain::policy::classify::RiskAssessment;
use crate::domain::table_data::PageLimits;
use crate::errors::{AppError, AppResult, ErrorCode};

const PREVIEW_LIMITS: PageLimits = PageLimits {
    max_rows: PREVIEW_ROWS,
    max_bytes: PREVIEW_BYTES,
};

/// "Update of public.orders", for the audit log.
fn describe(change: &DataChange) -> String {
    let table = change.table();
    format!(
        "{} of {}.{}",
        change.kind().label(),
        table.schema,
        table.name
    )
}

fn reasons(assessment: &RiskAssessment) -> Vec<String> {
    assessment
        .statements
        .iter()
        .flat_map(|statement| &statement.reasons)
        .chain(&assessment.reasons)
        .map(|reason| reason.message.clone())
        .collect()
}

/// Plans `change` on the open connection and holds it for the user's approval.
///
/// # Errors
///
/// `invalid_input` for a malformed change, an unknown column, a value its column cannot hold or
/// too many matching rows; `permission_denied` when the policy or the role's privileges refuse
/// it here (audited); `not_found` for an unknown table; `unavailable` when not connected.
pub async fn preview<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    approvals: &Approvals,
    connection_id: ConnectionId,
    change: DataChange,
    actor: Actor,
) -> AppResult<DataChangePreview> {
    change.validate()?;
    let kind = change.kind();
    let table = change.table().clone();
    let fingerprint = serde_json::to_vec(&change)
        .map(|json| sha256_hex(&json))
        .ok();
    let description = describe(&change);
    let requested = approvals::request_data_change(
        store,
        sessions,
        approvals,
        connection_id,
        change,
        PREVIEW_LIMITS,
        actor,
    )
    .await;
    let (request, plan) = match requested {
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
    let after =
        (kind != ChangeKind::Delete).then(|| ValueSample::of(&plan.value_columns, &plan.values));
    Ok(DataChangePreview {
        approval_id: request.id,
        kind,
        table,
        statement: plan.statement,
        rows: plan.rows,
        before: plan.before,
        after,
        risk: request.assessment.risk,
        approval: request.level,
        reasons: reasons(&request.assessment),
        confirmation: request.confirmation,
        expires_at: request.expires_at,
    })
}

fn no_such_change() -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        ApprovalError::Unknown.to_string(),
    )
}

/// The user approves a previewed change, which then runs at once. Elevated changes need the
/// database name typed in `confirmation`.
///
/// Refusals before anything runs are errors. Once the change runs, the result says whether it
/// was applied, rolled back (nothing written) or is uncertain (the connection was lost while
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
    let Some((owner, change)) = approvals.data_change(approval_id) else {
        return Err(no_such_change());
    };
    if owner != connection_id {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            ApprovalError::WrongConnection.to_string(),
        ));
    }
    let grant = approvals::approve(store, sessions, approvals, approval_id, confirmation).await?;
    let token: SecretString = grant.token;
    let subject = Subject::DataChange(change.clone());
    let operation =
        approvals::consume_subject(store, sessions, approvals, connection_id, &token, &subject)
            .await?;
    drop(token);

    let outcome = match (&operation.plan, sessions.get(connection_id)) {
        (Some(plan), Some(session)) => session.apply_data_change(&change, plan).await,
        (None, _) => Err(ApplyError::RolledBack(DbError::new(
            DbErrorKind::Other,
            "The change was not planned.",
        ))),
        (_, None) => {
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
                        "The connection was lost while the change was being committed, so it \
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
        detail: Some(format!("{}: {outcome_text}.", describe(&change))),
    };
    audit::record(store, &event).await;
    Ok(result)
}

/// The user declines a previewed change.
///
/// # Errors
///
/// `permission_denied` for an unknown approval, one that is not a data change, or one that is
/// already running.
pub async fn cancel(
    store: &LocalStore,
    approvals: &Approvals,
    approval_id: ApprovalId,
) -> AppResult<()> {
    let Some((connection_id, change)) = approvals.data_change(approval_id) else {
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
        detail: Some(format!("{}: declined.", describe(&change))),
    };
    audit::record(store, &event).await;
    Ok(())
}

#[cfg(test)]
mod tests;
