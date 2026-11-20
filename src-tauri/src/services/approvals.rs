//! Approvals for changes: prepared from a risk assessment, approved by the user, used once by
//! the execution layer. The core is the only place an approval exists; every check fails
//! closed, and every failed check and every grant is written to the audit log.
//!
//! An approval is bound to a digest of the exact operation and where it would run
//! ([`Binding`]). Approving and using it both rebuild the binding from the live session and
//! compare: a changed SQL text, parameter, schema, environment, session, estimate or policy
//! invalidates it. Tokens are random, kept only as a digest, single-use and short-lived.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ring::rand::{SecureRandom, SystemRandom};
use secrecy::{ExposeSecret, SecretString};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::time::Instant;

use super::sessions::{Sessions, not_connected};
use super::{audit, policy};
use crate::adapters::database::{DatabaseAdapter, DatabaseSession};
use crate::adapters::local_store::LocalStore;
use crate::domain::audit::{Actor, AuditEvent, AuditStatus};
use crate::domain::connections::{ConnectionId, Environment};
use crate::domain::data_change::{
    DataChange, DataChangePlan, DataChangePreview, PendingChange, SetLimits,
};
use crate::domain::policy::approval::{
    APPROVAL_TTL, ApprovalError, ApprovalId, ApprovalState, Binding, BindingContext,
    MAX_OPEN_APPROVALS, PENDING_TTL, hex, needs_typed_confirmation, sha256_hex,
};
use crate::domain::policy::classify::RiskAssessment;
use crate::domain::policy::facts;
use crate::domain::policy::risk::ApprovalLevel;
use crate::domain::table_data::PageLimits;
use crate::errors::{AppError, AppResult, ErrorCode};

/// A change waiting for the user's decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub id: ApprovalId,
    pub assessment: RiskAssessment,
    pub level: ApprovalLevel,
    pub state: ApprovalState,
    /// When the request lapses, RFC 3339.
    pub expires_at: String,
    /// What the user must type to approve (the database name), for elevated approvals.
    pub confirmation: Option<String>,
}

/// A granted approval. The token is the only way to use it.
#[derive(Debug)]
pub struct ApprovalGrant {
    pub id: ApprovalId,
    pub token: SecretString,
    pub expires_at: String,
}

/// What an approval is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    /// SQL text with bound parameters.
    Sql {
        sql: String,
        parameters: Vec<Option<String>>,
    },
    /// Changes to tables' rows, applied together; their statements are written from the live
    /// tables.
    DataChanges {
        /// Shared, not copied: an import holds many rows.
        changes: Arc<Vec<DataChange>>,
        limits: SetLimits,
    },
    /// A schema migration aidb generated, its statements in the order they run.
    Migration { statements: Arc<Vec<String>> },
}

/// What the execution layer gets for a valid token: the approval to report the outcome to,
/// the operation's digest for its audit record, the assessment it was approved under and, for
/// data changes, the plans they must still match when they run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedOperation {
    pub id: ApprovalId,
    pub digest: String,
    pub assessment: RiskAssessment,
    pub environment: Environment,
    /// One per data change, in order; empty for SQL.
    pub plans: Vec<DataChangePlan>,
}

#[derive(Debug)]
struct Entry {
    connection_id: ConnectionId,
    actor: Actor,
    state: ApprovalState,
    /// An approve or use is checking it right now; nothing else may.
    checking: bool,
    level: ApprovalLevel,
    binding: Binding,
    digest: String,
    subject: Subject,
    assessment: RiskAssessment,
    expires: Instant,
    /// SHA-256 of the token, once approved and until used.
    token_digest: Option<String>,
    /// What the review of data changes shows, kept to list them while they wait.
    preview: Option<DataChangePreview>,
}

/// Open approvals, in memory only: they never outlive the app.
#[derive(Debug, Default)]
pub struct Approvals {
    entries: Mutex<HashMap<ApprovalId, Entry>>,
}

impl Approvals {
    fn lock(&self) -> MutexGuard<'_, HashMap<ApprovalId, Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Drops every approval of a connection that has not started running. Called when the
    /// connection is opened, closed, edited or deleted. Returns how many were dropped.
    pub fn invalidate_connection(&self, connection_id: ConnectionId) -> usize {
        let mut entries = self.lock();
        let before = entries.len();
        entries.retain(|_, entry| {
            entry.connection_id != connection_id
                || matches!(
                    entry.state,
                    ApprovalState::Executing | ApprovalState::Verifying
                )
        });
        let dropped = before - entries.len();
        if dropped > 0 {
            tracing::info!(connection = %connection_id, dropped, "approvals invalidated");
        }
        dropped
    }

    /// Drops every approval that has not started running.
    pub fn invalidate_all(&self) {
        self.lock().retain(|_, entry| {
            matches!(
                entry.state,
                ApprovalState::Executing | ApprovalState::Verifying
            )
        });
    }

    /// The state of an approval, if it is still open.
    #[must_use]
    pub fn state(&self, id: ApprovalId) -> Option<ApprovalState> {
        self.lock().get(&id).map(|entry| entry.state)
    }

    /// Keeps the preview of data changes waiting for approval, so they can be listed.
    pub fn keep_preview(&self, preview: &DataChangePreview) {
        if let Some(entry) = self.lock().get_mut(&preview.approval_id) {
            entry.preview = Some(preview.clone());
        }
    }

    /// Data changes waiting for the user's decision, oldest first, on one connection or all.
    #[must_use]
    pub fn pending_data_changes(&self, connection_id: Option<ConnectionId>) -> Vec<PendingChange> {
        let now = Instant::now();
        let entries = self.lock();
        let mut pending: Vec<(Instant, PendingChange)> = entries
            .values()
            .filter(|entry| {
                entry.state == ApprovalState::AwaitingApproval
                    && !entry.checking
                    && entry.expires > now
                    && connection_id.is_none_or(|id| id == entry.connection_id)
            })
            .filter_map(|entry| {
                let Subject::DataChanges { changes, .. } = &entry.subject else {
                    return None;
                };
                let preview = entry.preview.clone()?;
                Some((
                    entry.expires,
                    PendingChange {
                        connection_id: entry.connection_id,
                        actor: entry.actor,
                        preview,
                        changes: changes.as_ref().clone(),
                    },
                ))
            })
            .collect();
        pending.sort_by_key(|(expires, _)| *expires);
        pending.into_iter().map(|(_, change)| change).collect()
    }

    /// The data changes an open approval is for, its connection, and who asked for it.
    #[must_use]
    pub fn data_changes(
        &self,
        id: ApprovalId,
    ) -> Option<(ConnectionId, Arc<Vec<DataChange>>, Actor)> {
        self.lock().get(&id).and_then(|entry| match &entry.subject {
            Subject::DataChanges { changes, .. } => {
                Some((entry.connection_id, Arc::clone(changes), entry.actor))
            }
            Subject::Sql { .. } | Subject::Migration { .. } => None,
        })
    }

    /// The statements of an open migration approval and its connection.
    #[must_use]
    pub fn migration(&self, id: ApprovalId) -> Option<(ConnectionId, Arc<Vec<String>>)> {
        self.lock().get(&id).and_then(|entry| match &entry.subject {
            Subject::Migration { statements } => {
                Some((entry.connection_id, Arc::clone(statements)))
            }
            _ => None,
        })
    }

    /// Removes approvals that lapsed, and the oldest ones beyond [`MAX_OPEN_APPROVALS`].
    fn prune(entries: &mut HashMap<ApprovalId, Entry>, now: Instant) {
        entries.retain(|_, entry| {
            entry.expires > now
                || entry.checking
                || matches!(
                    entry.state,
                    ApprovalState::Executing | ApprovalState::Verifying
                )
        });
        while entries.len() >= MAX_OPEN_APPROVALS {
            let oldest = entries
                .iter()
                .filter(|(_, entry)| {
                    !entry.checking
                        && !matches!(
                            entry.state,
                            ApprovalState::Executing | ApprovalState::Verifying
                        )
                })
                .min_by_key(|(_, entry)| entry.expires)
                .map(|(id, _)| *id);
            match oldest {
                Some(id) => {
                    entries.remove(&id);
                }
                None => break,
            }
        }
    }
}

fn rfc3339_in(duration: std::time::Duration) -> String {
    (OffsetDateTime::now_utc() + duration)
        .format(&Rfc3339)
        .unwrap_or_default()
}

fn new_token() -> AppResult<String> {
    let mut bytes = [0_u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| AppError::internal("the system random number generator failed"))?;
    Ok(hex(&bytes))
}

fn to_app_error(error: &ApprovalError) -> AppError {
    let code = match error {
        ApprovalError::NotNeeded | ApprovalError::ConfirmationNeeded(_) => ErrorCode::InvalidInput,
        _ => ErrorCode::PermissionDenied,
    };
    AppError::new(code, error.to_string())
}

/// The binding of `subject` on the open connection, built from live state: the session and
/// its serial, the profile's environment and write setting, the schema fingerprint and a fresh
/// assessment. Data changes are planned again (statements written, rows counted); with
/// `preview`, the plans include some of the rows the first changes match.
async fn bind<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    connection_id: ConnectionId,
    subject: &Subject,
    preview: Option<PageLimits>,
) -> AppResult<(Binding, RiskAssessment, Vec<DataChangePlan>)> {
    let (session, serial) = sessions
        .get_with_serial(connection_id)
        .ok_or_else(not_connected)?;
    let (assessment, texts, parameters, plans) = match subject {
        Subject::Sql { sql, parameters } => {
            let assessment = policy::assess(store, sessions, connection_id, sql).await?;
            (
                assessment,
                vec![sql.clone()],
                parameters.clone(),
                Vec::new(),
            )
        }
        Subject::DataChanges { changes, limits } => {
            let plans = session
                .plan_data_changes(changes, preview, limits.max_rows)
                .await
                .map_err(AppError::from)?;
            let statements: Vec<(&str, u64)> = plans
                .iter()
                .map(|plan| (plan.statement.as_str(), plan.rows))
                .collect();
            let assessment =
                policy::assess_change_set(store, sessions, connection_id, &statements).await?;
            let texts = plans.iter().map(|plan| plan.statement.clone()).collect();
            let parameters = plans
                .iter()
                .flat_map(|plan| plan.parameters.iter().cloned())
                .collect();
            (assessment, texts, parameters, plans)
        }
        Subject::Migration { statements } => {
            let assessment =
                policy::assess_migration(store, sessions, connection_id, statements).await?;
            (
                assessment,
                statements.as_ref().clone(),
                Vec::new(),
                Vec::new(),
            )
        }
    };
    let profile = store
        .connections()
        .get(connection_id)
        .await?
        .ok_or_else(|| AppError::not_found("The connection no longer exists."))?;
    let server = session.server_info();
    let schema_fingerprint = session
        .catalog_fingerprint()
        .await
        .map_err(AppError::from)?;
    // Each statement's text exactly as it will run.
    let texts: Vec<String> = texts
        .iter()
        .flat_map(|sql| facts::inspect(sql).statements)
        .map(|statement| statement.text)
        .collect();
    let binding = Binding::new(
        BindingContext {
            connection_id,
            session_serial: serial,
            database: server.database.clone(),
            server_user: server.user.clone(),
            environment: profile.settings.environment,
            writes_allowed: !profile.settings.read_only && !server.in_recovery,
            schema_fingerprint,
        },
        &assessment,
        &texts,
        &parameters,
    );
    Ok((binding, assessment, plans))
}

/// Records a refused check. The audit log gets the digest and the reason, never the SQL.
async fn audit_refusal(
    store: &LocalStore,
    connection_id: Option<ConnectionId>,
    entry: Option<(&Binding, &str, ApprovalState, Actor)>,
    error: &ApprovalError,
) {
    let event = AuditEvent {
        connection_id,
        environment: entry.map(|(binding, ..)| binding.environment),
        actor: entry.map_or(Actor::System, |(.., actor)| actor),
        operation: "approval_check",
        risk: None,
        fingerprint: entry.map(|(_, digest, ..)| digest.to_owned()),
        approval_state: entry.map(|(.., state, _)| state.as_str()),
        status: AuditStatus::Rejected,
        rows_affected: None,
        detail: Some(format!("{}: {error}", error.code())),
    };
    audit::record(store, &event).await;
}

/// Prepares `sql` with `parameters` for approval on the open connection.
///
/// # Errors
///
/// `invalid_input` when the change needs no approval; `permission_denied` when the policy
/// blocks it here; the assessment's errors.
pub async fn request<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    approvals: &Approvals,
    connection_id: ConnectionId,
    sql: &str,
    parameters: Vec<Option<String>>,
    actor: Actor,
) -> AppResult<ApprovalRequest> {
    let subject = Subject::Sql {
        sql: sql.to_owned(),
        parameters,
    };
    request_subject(
        store,
        sessions,
        approvals,
        connection_id,
        subject,
        None,
        actor,
    )
    .await
    .map(|(request, _)| request)
}

/// Prepares a schema migration for approval on the open connection: its statements, in the
/// order they run, assessed together.
///
/// # Errors
///
/// As [`request`].
pub async fn request_migration<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    approvals: &Approvals,
    connection_id: ConnectionId,
    statements: Arc<Vec<String>>,
) -> AppResult<ApprovalRequest> {
    request_subject(
        store,
        sessions,
        approvals,
        connection_id,
        Subject::Migration { statements },
        None,
        Actor::User,
    )
    .await
    .map(|(request, _)| request)
}

/// Data changes applied together, and the limits the set is held to.
#[derive(Debug, Clone)]
pub struct ChangeSet {
    pub changes: Arc<Vec<DataChange>>,
    pub limits: SetLimits,
}

/// Prepares data changes, applied together, for approval on the open connection, and returns
/// their plans with some of the rows the first ones match (read with `preview`), for the user to
/// review.
///
/// # Errors
///
/// As [`request`], and the planning errors: an unknown table or column, a value its column
/// cannot hold, too many matching rows.
pub async fn request_data_changes<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    approvals: &Approvals,
    connection_id: ConnectionId,
    set: ChangeSet,
    preview: PageLimits,
    actor: Actor,
) -> AppResult<(ApprovalRequest, Vec<DataChangePlan>)> {
    request_subject(
        store,
        sessions,
        approvals,
        connection_id,
        Subject::DataChanges {
            changes: set.changes,
            limits: set.limits,
        },
        Some(preview),
        actor,
    )
    .await
}

async fn request_subject<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    approvals: &Approvals,
    connection_id: ConnectionId,
    subject: Subject,
    preview: Option<PageLimits>,
    actor: Actor,
) -> AppResult<(ApprovalRequest, Vec<DataChangePlan>)> {
    let mut state = ApprovalState::Draft;
    let (binding, assessment, plans) =
        bind(store, sessions, connection_id, &subject, preview).await?;
    let advance =
        |state: &mut ApprovalState, next| state.advance(next).map_err(|error| to_app_error(&error));
    if !assessment.blockers.is_empty() {
        return Err(to_app_error(&ApprovalError::Blocked(
            assessment.blockers.join(" "),
        )));
    }
    if assessment.required_approval == ApprovalLevel::None {
        return Err(to_app_error(&ApprovalError::NotNeeded));
    }
    advance(&mut state, ApprovalState::Validated)?;
    // The assessment included the impact estimate, so the preview can be shown.
    advance(&mut state, ApprovalState::PreviewReady)?;
    advance(&mut state, ApprovalState::AwaitingApproval)?;

    let id = ApprovalId::new();
    let level = assessment.required_approval;
    let confirmation = needs_typed_confirmation(level).then(|| binding.database.clone());
    let digest = binding.digest();
    {
        let mut entries = approvals.lock();
        Approvals::prune(&mut entries, Instant::now());
        entries.insert(
            id,
            Entry {
                connection_id,
                actor,
                state,
                checking: false,
                level,
                binding,
                digest,
                subject,
                assessment: assessment.clone(),
                expires: Instant::now() + PENDING_TTL,
                token_digest: None,
                preview: None,
            },
        );
    }
    let request = ApprovalRequest {
        id,
        assessment,
        level,
        state,
        expires_at: rfc3339_in(PENDING_TTL),
        confirmation,
    };
    Ok((request, plans))
}

/// Why an approval could not be checked again, for the user and the audit log.
fn unchecked(error: &AppError) -> &'static str {
    if error.code() == ErrorCode::Unavailable {
        "the connection is no longer open"
    } else {
        "it could not be checked again"
    }
}

/// Marks an entry invalidated and removes it.
fn invalidate(approvals: &Approvals, id: ApprovalId) {
    if let Some(mut entry) = approvals.lock().remove(&id) {
        let _ = entry.state.advance(ApprovalState::Invalidated);
    }
}

/// The user approves a prepared change. Elevated approvals need the database name typed in
/// `confirmation`.
///
/// # Errors
///
/// `permission_denied` when the approval is unknown, not awaiting a decision, expired, or
/// anything it was bound to changed (it is then invalidated); `invalid_input` when the typed
/// confirmation is missing or wrong (it stays open).
pub async fn approve<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    approvals: &Approvals,
    id: ApprovalId,
    confirmation: Option<&str>,
) -> AppResult<ApprovalGrant> {
    match decide(store, sessions, approvals, id, confirmation, None).await? {
        Decision::Granted(grant) => Ok(grant),
        Decision::Running(_) => Err(AppError::internal("an approval ran without being used")),
    }
}

/// The user approves a prepared change that the core runs at once, on `connection_id`: as
/// [`approve`] followed by [`consume_subject`], with one check of the binding and no token
/// issued. On success the approval is executing; report the outcome with [`finish`].
///
/// # Errors
///
/// As [`approve`], and `permission_denied` when the approval belongs to another connection.
pub async fn approve_and_use<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    approvals: &Approvals,
    connection_id: ConnectionId,
    id: ApprovalId,
    confirmation: Option<&str>,
) -> AppResult<ApprovedOperation> {
    match decide(
        store,
        sessions,
        approvals,
        id,
        confirmation,
        Some(connection_id),
    )
    .await?
    {
        Decision::Running(operation) => Ok(operation),
        Decision::Granted(_) => Err(AppError::internal("an approval was granted, not used")),
    }
}

enum Decision {
    Granted(ApprovalGrant),
    Running(ApprovedOperation),
}

/// Approves `id`; with `use_on`, also starts running it there instead of issuing a token.
async fn decide<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    approvals: &Approvals,
    id: ApprovalId,
    confirmation: Option<&str>,
    use_on: Option<ConnectionId>,
) -> AppResult<Decision> {
    // Claim it, so nothing else approves or uses it while it is checked.
    let claimed = {
        let mut entries = approvals.lock();
        match entries.get_mut(&id) {
            None => Err((ApprovalError::Unknown, None)),
            Some(entry) if entry.checking || entry.state != ApprovalState::AwaitingApproval => {
                Err((
                    ApprovalError::WrongState(entry.state, "approved"),
                    Some((
                        entry.connection_id,
                        entry.binding.clone(),
                        entry.digest.clone(),
                        entry.state,
                        entry.actor,
                    )),
                ))
            }
            Some(entry) if entry.expires <= Instant::now() => {
                let _ = entry.state.advance(ApprovalState::Expired);
                let snapshot = (
                    entry.connection_id,
                    entry.binding.clone(),
                    entry.digest.clone(),
                    entry.state,
                    entry.actor,
                );
                entries.remove(&id);
                Err((ApprovalError::Expired, Some(snapshot)))
            }
            Some(entry) => {
                entry.checking = true;
                Ok((
                    entry.connection_id,
                    entry.binding.clone(),
                    entry.digest.clone(),
                    entry.subject.clone(),
                    entry.level,
                    entry.actor,
                ))
            }
        }
    };
    let (connection_id, binding, digest, subject, level, actor) = match claimed {
        Ok(claimed) => claimed,
        Err((error, snapshot)) => {
            let entry = snapshot.as_ref().map(|(_, binding, digest, state, actor)| {
                (binding, digest.as_str(), *state, *actor)
            });
            audit_refusal(store, snapshot.as_ref().map(|s| s.0), entry, &error).await;
            return Err(to_app_error(&error));
        }
    };
    let release = || {
        if let Some(entry) = approvals.lock().get_mut(&id) {
            entry.checking = false;
        }
    };

    if use_on.is_some_and(|expected| expected != connection_id) {
        invalidate(approvals, id);
        let error = ApprovalError::WrongConnection;
        audit_refusal(
            store,
            use_on,
            Some((&binding, &digest, ApprovalState::Invalidated, actor)),
            &error,
        )
        .await;
        return Err(to_app_error(&error));
    }

    if needs_typed_confirmation(level)
        && confirmation.map(str::trim) != Some(binding.database.as_str())
    {
        release();
        let error = ApprovalError::ConfirmationNeeded(binding.database.clone());
        audit_refusal(
            store,
            Some(connection_id),
            Some((&binding, &digest, ApprovalState::AwaitingApproval, actor)),
            &error,
        )
        .await;
        return Err(to_app_error(&error));
    }

    let fresh = bind(store, sessions, connection_id, &subject, None).await;
    let checked = match fresh {
        Ok((current, assessment, plans)) if current.digest() == digest => Ok((assessment, plans)),
        Ok((current, ..)) => Err(ApprovalError::Changed(binding.first_difference(&current))),
        Err(error) => Err(ApprovalError::Changed(unchecked(&error))),
    };
    let (assessment, plans) = match checked {
        Ok(checked) => checked,
        Err(error) => {
            invalidate(approvals, id);
            audit_refusal(
                store,
                Some(connection_id),
                Some((&binding, &digest, ApprovalState::Invalidated, actor)),
                &error,
            )
            .await;
            return Err(to_app_error(&error));
        }
    };

    let approved_event = |risk| AuditEvent {
        connection_id: Some(connection_id),
        environment: Some(binding.environment),
        actor: Actor::User,
        operation: "approve",
        risk: Some(risk),
        fingerprint: Some(digest.clone()),
        approval_state: Some(ApprovalState::Approved.as_str()),
        status: AuditStatus::Succeeded,
        rows_affected: None,
        detail: None,
    };
    if use_on.is_some() {
        let started = {
            let mut entries = approvals.lock();
            entries.get_mut(&id).is_some_and(|entry| {
                entry.checking = false;
                entry.state.advance(ApprovalState::Approved).is_ok()
                    && entry.state.advance(ApprovalState::Executing).is_ok()
            })
        };
        // Invalidated while it was being checked (the connection closed, say).
        if !started {
            return Err(to_app_error(&ApprovalError::Unknown));
        }
        audit::record(store, &approved_event(assessment.risk)).await;
        return Ok(Decision::Running(ApprovedOperation {
            id,
            digest,
            assessment,
            environment: binding.environment,
            plans,
        }));
    }

    let token = new_token()?;
    let granted = {
        let mut entries = approvals.lock();
        entries.get_mut(&id).and_then(|entry| {
            entry.checking = false;
            entry.state.advance(ApprovalState::Approved).ok()?;
            entry.token_digest = Some(sha256_hex(token.as_bytes()));
            entry.expires = Instant::now() + APPROVAL_TTL;
            Some(entry.assessment.risk)
        })
    };
    // Invalidated while it was being checked (the connection closed, say).
    let Some(risk) = granted else {
        return Err(to_app_error(&ApprovalError::Unknown));
    };
    audit::record(store, &approved_event(risk)).await;
    Ok(Decision::Granted(ApprovalGrant {
        id,
        token: SecretString::from(token),
        expires_at: rfc3339_in(APPROVAL_TTL),
    }))
}

/// The execution layer presents a token for `sql` with `parameters` on `connection_id`. The
/// token is used up whatever the outcome; on success the approval is executing and the caller
/// must report the result with [`finish`].
///
/// # Errors
///
/// `permission_denied` for an unknown, used, expired or foreign token, or when anything bound
/// changed. Every refusal is audited.
pub async fn consume<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    approvals: &Approvals,
    connection_id: ConnectionId,
    token: &SecretString,
    sql: &str,
    parameters: &[Option<String>],
) -> AppResult<ApprovedOperation> {
    let subject = Subject::Sql {
        sql: sql.to_owned(),
        parameters: parameters.to_vec(),
    };
    consume_subject(store, sessions, approvals, connection_id, token, &subject).await
}

/// As [`consume`], for any subject. For a data change, the operation carries the fresh plan
/// the change must still match when it runs.
///
/// # Errors
///
/// As [`consume`].
pub async fn consume_subject<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    approvals: &Approvals,
    connection_id: ConnectionId,
    token: &SecretString,
    subject: &Subject,
) -> AppResult<ApprovedOperation> {
    let token_digest = sha256_hex(token.expose_secret().as_bytes());
    // Claim it and take the token away first: a second use fails even if it arrives now.
    let claimed = {
        let mut entries = approvals.lock();
        let found = entries
            .iter()
            .find(|(_, entry)| entry.token_digest.as_deref() == Some(token_digest.as_str()))
            .map(|(id, _)| *id);
        match found.and_then(|id| entries.get_mut(&id).map(|entry| (id, entry))) {
            None => Err((ApprovalError::Unknown, None)),
            Some((id, entry)) => {
                entry.token_digest = None;
                let snapshot = (id, entry.binding.clone(), entry.digest.clone(), entry.actor);
                if entry.connection_id != connection_id {
                    Err((ApprovalError::WrongConnection, Some(snapshot)))
                } else if entry.checking || entry.state != ApprovalState::Approved {
                    Err((
                        ApprovalError::WrongState(entry.state, "used"),
                        Some(snapshot),
                    ))
                } else if entry.expires <= Instant::now() {
                    Err((ApprovalError::Expired, Some(snapshot)))
                } else {
                    entry.checking = true;
                    Ok(snapshot)
                }
            }
        }
    };
    let (id, binding, digest, actor) = match claimed {
        Ok(claimed) => claimed,
        Err((error, snapshot)) => {
            if let Some((id, ..)) = &snapshot {
                invalidate(approvals, *id);
            }
            let entry = snapshot.as_ref().map(|(_, binding, digest, actor)| {
                (binding, digest.as_str(), ApprovalState::Invalidated, *actor)
            });
            audit_refusal(store, Some(connection_id), entry, &error).await;
            return Err(to_app_error(&error));
        }
    };

    let fresh = bind(store, sessions, connection_id, subject, None).await;
    let checked = match fresh {
        Ok((current, assessment, plans)) if current.digest() == digest => Ok((assessment, plans)),
        Ok((current, ..)) => Err(ApprovalError::Changed(binding.first_difference(&current))),
        Err(error) => Err(ApprovalError::Changed(unchecked(&error))),
    };
    let (assessment, plans) = match checked {
        Ok(checked) => checked,
        Err(error) => {
            invalidate(approvals, id);
            audit_refusal(
                store,
                Some(connection_id),
                Some((&binding, &digest, ApprovalState::Invalidated, actor)),
                &error,
            )
            .await;
            return Err(to_app_error(&error));
        }
    };
    let started = {
        let mut entries = approvals.lock();
        entries.get_mut(&id).is_some_and(|entry| {
            entry.checking = false;
            entry.state.advance(ApprovalState::Executing).is_ok()
        })
    };
    if !started {
        let error = ApprovalError::Unknown;
        audit_refusal(
            store,
            Some(connection_id),
            Some((&binding, &digest, ApprovalState::Invalidated, actor)),
            &error,
        )
        .await;
        return Err(to_app_error(&error));
    }
    Ok(ApprovedOperation {
        id,
        digest,
        assessment,
        environment: binding.environment,
        plans,
    })
}

/// Reports how an executing approval ended (or that it is being verified). Final states remove
/// it.
///
/// # Errors
///
/// `permission_denied` for an unknown approval or a transition the machine does not allow.
pub fn finish(approvals: &Approvals, id: ApprovalId, next: ApprovalState) -> AppResult<()> {
    let mut entries = approvals.lock();
    let entry = entries
        .get_mut(&id)
        .ok_or_else(|| to_app_error(&ApprovalError::Unknown))?;
    if !matches!(
        entry.state,
        ApprovalState::Executing | ApprovalState::Verifying
    ) {
        return Err(to_app_error(&ApprovalError::WrongState(
            entry.state,
            "finished",
        )));
    }
    entry
        .state
        .advance(next)
        .map_err(|error| to_app_error(&error))?;
    if entry.state.is_final() {
        entries.remove(&id);
    }
    Ok(())
}

/// The user declines a prepared or approved change.
///
/// # Errors
///
/// `permission_denied` for an unknown approval or one already running.
pub fn cancel(approvals: &Approvals, id: ApprovalId) -> AppResult<()> {
    let mut entries = approvals.lock();
    let entry = entries
        .get_mut(&id)
        .ok_or_else(|| to_app_error(&ApprovalError::Unknown))?;
    // Once running, only the execution layer reports how it ended (see `finish`).
    if entry.checking
        || matches!(
            entry.state,
            ApprovalState::Executing | ApprovalState::Verifying
        )
    {
        return Err(to_app_error(&ApprovalError::WrongState(
            entry.state,
            "declined",
        )));
    }
    entry
        .state
        .advance(ApprovalState::Cancelled)
        .map_err(|error| to_app_error(&error))?;
    entries.remove(&id);
    Ok(())
}

#[cfg(test)]
mod tests;
