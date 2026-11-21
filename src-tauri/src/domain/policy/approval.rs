//! Approvals: the lifecycle of a change from draft to result, and the binding that ties an
//! approval to exactly one operation. An approval is valid only for the binding it was given
//! for; anything that changes the binding (the SQL, its parameters, the target, the schema, the
//! environment, the session, the estimate or the policy) makes it fail closed.

use std::fmt;
use std::time::Duration;

use serde::Serialize;

use super::classify::{POLICY_VERSION, RiskAssessment};
use super::risk::ApprovalLevel;
use crate::domain::connections::{ConnectionId, Environment};

/// How long a prepared change waits for the user's decision.
pub const PENDING_TTL: Duration = Duration::from_secs(10 * 60);
/// How long an approval may be used after it is given. Short: it is meant to run at once.
pub const APPROVAL_TTL: Duration = Duration::from_secs(2 * 60);
/// Most prepared or approved changes kept at a time; the oldest go first.
pub const MAX_OPEN_APPROVALS: usize = 32;

/// Identifies a prepared change and its approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ApprovalId(uuid::Uuid);

impl ApprovalId {
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for ApprovalId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ApprovalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for ApprovalId {
    type Err = uuid::Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.parse().map(Self)
    }
}

impl Serialize for ApprovalId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for ApprovalId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// Where a change is in its lifecycle (PRD: draft → validated → preview_ready →
/// awaiting_approval → approved → executing → verifying → succeeded | failed | partial |
/// cancelled), plus the two ways an approval ends without running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    Draft,
    /// The policy accepted it for approval.
    Validated,
    /// Its impact is estimated and its preview can be shown.
    PreviewReady,
    AwaitingApproval,
    Approved,
    Executing,
    Verifying,
    Succeeded,
    Failed,
    Partial,
    Cancelled,
    /// Not approved, or not used, in time.
    Expired,
    /// Something it was bound to changed, or a check failed.
    Invalidated,
}

impl ApprovalState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Validated => "validated",
            Self::PreviewReady => "preview_ready",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Approved => "approved",
            Self::Executing => "executing",
            Self::Verifying => "verifying",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Partial => "partial",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
            Self::Invalidated => "invalidated",
        }
    }

    /// No further transition is possible.
    #[must_use]
    pub const fn is_final(self) -> bool {
        matches!(
            self,
            Self::Succeeded
                | Self::Failed
                | Self::Partial
                | Self::Cancelled
                | Self::Expired
                | Self::Invalidated
        )
    }

    /// Whether the machine allows going from `self` to `next`.
    #[must_use]
    pub const fn allows(self, next: Self) -> bool {
        use ApprovalState::{
            Approved, AwaitingApproval, Cancelled, Draft, Executing, Expired, Failed, Invalidated,
            Partial, PreviewReady, Succeeded, Validated, Verifying,
        };
        match (self, next) {
            (Draft, Validated)
            | (Validated, PreviewReady)
            | (PreviewReady, AwaitingApproval)
            | (AwaitingApproval, Approved)
            | (Approved, Executing)
            | (Executing, Verifying | Succeeded | Failed | Partial | Cancelled)
            | (Verifying, Succeeded | Failed | Partial) => true,
            // Before it runs, a change can always be cancelled, expire or be invalidated.
            (
                Draft | Validated | PreviewReady | AwaitingApproval | Approved,
                Cancelled | Expired | Invalidated,
            ) => true,
            _ => false,
        }
    }

    /// Moves to `next`.
    ///
    /// # Errors
    ///
    /// When the machine does not allow it; the state is unchanged.
    pub fn advance(&mut self, next: Self) -> Result<(), ApprovalError> {
        if self.allows(next) {
            *self = next;
            Ok(())
        } else {
            Err(ApprovalError::BadTransition {
                from: *self,
                to: next,
            })
        }
    }
}

impl fmt::Display for ApprovalState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why an approval cannot be given or used. Every one of these fails closed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApprovalError {
    #[error("This change needs no approval.")]
    NotNeeded,
    #[error("This change cannot run here: {0}")]
    Blocked(String),
    #[error("There is no such approval, or it was already used.")]
    Unknown,
    #[error("The approval expired. Review the change again.")]
    Expired,
    #[error(
        "Something changed since the change was reviewed ({0}). Review it again before approving."
    )]
    Changed(&'static str),
    #[error("The approval belongs to another connection.")]
    WrongConnection,
    #[error("Type the database name, {0}, to approve this change.")]
    ConfirmationNeeded(String),
    #[error("The approval is {0} and cannot be {1}.")]
    WrongState(ApprovalState, &'static str),
    #[error("A change cannot go from {from} to {to}.")]
    BadTransition {
        from: ApprovalState,
        to: ApprovalState,
    },
}

impl ApprovalError {
    /// A short code for the audit log.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotNeeded => "not_needed",
            Self::Blocked(_) => "blocked",
            Self::Unknown => "unknown_approval",
            Self::Expired => "expired",
            Self::Changed(_) => "binding_changed",
            Self::WrongConnection => "wrong_connection",
            Self::ConfirmationNeeded(_) => "confirmation_missing",
            Self::WrongState(..) => "wrong_state",
            Self::BadTransition { .. } => "bad_transition",
        }
    }
}

/// One statement as the approval binds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundStatement {
    /// Exactly as it will run.
    pub text: String,
    pub operation: &'static str,
    pub targets: Vec<String>,
    pub risk: &'static str,
    pub estimated_rows: Option<u64>,
}

/// Everything an approval is bound to. Its digest identifies the operation; equal digests mean
/// the same SQL and parameters, on the same session of the same connection, environment and
/// schema, with the same impact, under the same policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    pub policy_version: u32,
    pub connection_id: ConnectionId,
    /// Changes whenever the connection is reopened.
    pub session_serial: u64,
    pub database: String,
    pub server_user: String,
    pub environment: Environment,
    pub writes_allowed: bool,
    pub schema_fingerprint: String,
    /// Digest of the bound parameter values, in order; the values themselves are not kept.
    pub parameters_digest: String,
    pub statements: Vec<BoundStatement>,
    pub risk: &'static str,
    pub required_approval: ApprovalLevel,
}

/// What the binding is built from, apart from the assessment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingContext {
    pub connection_id: ConnectionId,
    pub session_serial: u64,
    pub database: String,
    pub server_user: String,
    pub environment: Environment,
    pub writes_allowed: bool,
    pub schema_fingerprint: String,
}

/// SHA-256 of `bytes`, as lowercase hex.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    hex(digest.as_ref())
}

/// Lowercase hex.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// Digest of parameter values: each value's length then the value, so `["ab", "c"]` and
/// `["a", "bc"]` differ.
#[must_use]
pub fn parameters_digest(parameters: &[Option<String>]) -> String {
    let mut bytes = Vec::new();
    for parameter in parameters {
        match parameter {
            None => bytes.extend_from_slice(b"N;"),
            Some(value) => {
                bytes.extend_from_slice(format!("V{}:", value.len()).as_bytes());
                bytes.extend_from_slice(value.as_bytes());
                bytes.push(b';');
            }
        }
    }
    sha256_hex(&bytes)
}

impl Binding {
    /// The binding for `assessment` of `statements` (their exact texts, in order) with
    /// `parameters`, in `context`.
    #[must_use]
    pub fn new(
        context: BindingContext,
        assessment: &RiskAssessment,
        statements: &[String],
        parameters: &[Option<String>],
    ) -> Self {
        Self {
            policy_version: POLICY_VERSION,
            connection_id: context.connection_id,
            session_serial: context.session_serial,
            database: context.database,
            server_user: context.server_user,
            environment: context.environment,
            writes_allowed: context.writes_allowed,
            schema_fingerprint: context.schema_fingerprint,
            parameters_digest: parameters_digest(parameters),
            statements: assessment
                .statements
                .iter()
                .zip(statements)
                .map(|(statement, text)| BoundStatement {
                    text: text.clone(),
                    operation: statement.operation.as_str(),
                    targets: statement.targets.clone(),
                    risk: statement.risk.as_str(),
                    estimated_rows: statement.impact.and_then(|impact| impact.rows),
                })
                .collect(),
            risk: assessment.risk.as_str(),
            required_approval: assessment.required_approval,
        }
    }

    /// SHA-256 of the binding's canonical JSON (fields in declaration order).
    #[must_use]
    pub fn digest(&self) -> String {
        // Serializing these plain types cannot fail; an empty input would still be a digest
        // nothing else shares, so the binding fails closed either way.
        let json = serde_json::to_vec(self).unwrap_or_default();
        sha256_hex(&json)
    }

    /// The first part that differs from `other`, for the user and the audit log.
    #[must_use]
    pub fn first_difference(&self, other: &Self) -> &'static str {
        if self.connection_id != other.connection_id || self.session_serial != other.session_serial
        {
            "the connection was reopened or switched"
        } else if self.environment != other.environment
            || self.writes_allowed != other.writes_allowed
        {
            "the connection's environment or write setting"
        } else if self.database != other.database || self.server_user != other.server_user {
            "the database or role"
        } else if self.schema_fingerprint != other.schema_fingerprint {
            "the database's structure"
        } else if self.parameters_digest != other.parameters_digest {
            "the parameter values"
        } else if self
            .statements
            .iter()
            .map(|s| &s.text)
            .ne(other.statements.iter().map(|s| &s.text))
        {
            "the SQL"
        } else if self.statements != other.statements {
            "the estimated impact"
        } else if self.policy_version != other.policy_version {
            "the policy"
        } else {
            "the risk or required approval"
        }
    }
}

/// What the user must do to approve at `level`, for the confirmation step.
#[must_use]
pub fn needs_typed_confirmation(level: ApprovalLevel) -> bool {
    level == ApprovalLevel::Elevated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::policy::classify::{self, ImpactFacts, ImpactSource, PolicyContext};
    use crate::domain::policy::facts::inspect;

    #[test]
    fn the_machine_follows_the_prd() {
        use ApprovalState::*;
        let mut state = Draft;
        for next in [
            Validated,
            PreviewReady,
            AwaitingApproval,
            Approved,
            Executing,
            Verifying,
            Succeeded,
        ] {
            state.advance(next).expect("allowed");
        }
        assert!(state.is_final());
        assert!(state.advance(Executing).is_err());

        // No skipping approval, and nothing after an approval is used but its outcome.
        let mut pending = AwaitingApproval;
        assert_eq!(
            pending.advance(Executing),
            Err(ApprovalError::BadTransition {
                from: AwaitingApproval,
                to: Executing
            })
        );
        assert_eq!(pending, AwaitingApproval);
        assert!(!Executing.allows(Invalidated));
        assert!(!Executing.allows(Expired));
        assert!(Approved.allows(Invalidated));
        assert!(AwaitingApproval.allows(Expired));
        assert!(!Invalidated.allows(Approved));
    }

    fn context() -> BindingContext {
        BindingContext {
            connection_id: ConnectionId::new(),
            session_serial: 1,
            database: "shop".into(),
            server_user: "writer".into(),
            environment: Environment::Development,
            writes_allowed: true,
            schema_fingerprint: "f1".into(),
        }
    }

    fn binding(
        context: BindingContext,
        sql: &str,
        rows: u64,
        parameters: &[Option<String>],
    ) -> Binding {
        let facts = inspect(sql);
        let impact = ImpactFacts {
            rows: Some(rows),
            source: ImpactSource::Planner,
            ..ImpactFacts::unknown()
        };
        let assessment = classify::assess(
            &facts,
            &[impact],
            PolicyContext {
                environment: context.environment,
                writes_allowed: context.writes_allowed,
            },
        );
        let texts: Vec<String> = facts.statements.iter().map(|s| s.text.clone()).collect();
        Binding::new(context, &assessment, &texts, parameters)
    }

    #[test]
    fn every_bound_part_changes_the_digest() {
        let base = context();
        let sql = "UPDATE orders SET status = $1 WHERE id = $2";
        let params = [Some("shipped".to_owned()), Some("7".to_owned())];
        let original = binding(base.clone(), sql, 1, &params);
        assert_eq!(
            original.digest(),
            binding(base.clone(), sql, 1, &params).digest()
        );
        assert_eq!(original.digest().len(), 64);

        let changes: Vec<(Binding, &str)> = vec![
            (
                binding(
                    BindingContext {
                        session_serial: 2,
                        ..base.clone()
                    },
                    sql,
                    1,
                    &params,
                ),
                "the connection was reopened or switched",
            ),
            (
                binding(
                    BindingContext {
                        environment: Environment::Production,
                        ..base.clone()
                    },
                    sql,
                    1,
                    &params,
                ),
                "the connection's environment or write setting",
            ),
            (
                binding(
                    BindingContext {
                        schema_fingerprint: "f2".into(),
                        ..base.clone()
                    },
                    sql,
                    1,
                    &params,
                ),
                "the database's structure",
            ),
            (
                binding(
                    base.clone(),
                    sql,
                    1,
                    &[Some("shipped".into()), Some("8".into())],
                ),
                "the parameter values",
            ),
            (
                binding(
                    base.clone(),
                    "UPDATE orders SET status = $1 WHERE id >= $2",
                    1,
                    &params,
                ),
                "the SQL",
            ),
            (
                binding(base.clone(), sql, 2, &params),
                "the estimated impact",
            ),
        ];
        for (changed, reason) in changes {
            assert_ne!(changed.digest(), original.digest(), "{reason}");
            assert_eq!(original.first_difference(&changed), reason);
        }
    }

    #[test]
    fn parameter_digests_keep_boundaries_and_nulls() {
        assert_ne!(
            parameters_digest(&[Some("ab".into()), Some("c".into())]),
            parameters_digest(&[Some("a".into()), Some("bc".into())])
        );
        assert_ne!(
            parameters_digest(&[None]),
            parameters_digest(&[Some(String::new())])
        );
        assert_eq!(parameters_digest(&[]), sha256_hex(b""));
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
