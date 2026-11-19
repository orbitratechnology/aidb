//! Audit events: what happened to an operation, without its values. Written by the core only,
//! append-only in the local store, cleared only through Settings › Storage.

use crate::domain::connections::{ConnectionId, Environment};
use crate::domain::policy::risk::RiskLevel;

/// Longest detail kept, in characters (the table allows 4,000).
pub const MAX_DETAIL_CHARS: usize = 1_000;

/// Who caused the event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    User,
    Agent,
    System,
}

impl Actor {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
            Self::System => "system",
        }
    }
}

/// How the operation ended, as the audit log records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditStatus {
    Succeeded,
    Failed,
    Partial,
    Cancelled,
    /// A check refused it; nothing ran.
    Rejected,
}

impl AuditStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Partial => "partial",
            Self::Cancelled => "cancelled",
            Self::Rejected => "rejected",
        }
    }
}

/// One audit record. `fingerprint` is a digest of the operation, never its text or values;
/// `detail` is redacted and cut before it is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvent {
    pub connection_id: Option<ConnectionId>,
    pub environment: Option<Environment>,
    pub actor: Actor,
    /// A short name, such as `approve` or `approval_check`.
    pub operation: &'static str,
    pub risk: Option<RiskLevel>,
    pub fingerprint: Option<String>,
    pub approval_state: Option<&'static str>,
    pub status: AuditStatus,
    pub rows_affected: Option<u64>,
    pub detail: Option<String>,
}
