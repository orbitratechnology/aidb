//! Audit events: what happened to an operation, without its values. Written by the core only,
//! append-only in the local store, cleared only through Settings › Storage.

use serde::{Deserialize, Serialize};

use crate::domain::connections::{ConnectionId, Environment};
use crate::domain::policy::risk::RiskLevel;

/// Longest detail kept, in characters (the table allows 4,000).
pub const MAX_DETAIL_CHARS: usize = 1_000;

/// Who caused the event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
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

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        [Self::User, Self::Agent, Self::System]
            .into_iter()
            .find(|actor| actor.as_str() == text)
    }
}

/// How the operation ended, as the audit log records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
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

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        [
            Self::Succeeded,
            Self::Failed,
            Self::Partial,
            Self::Cancelled,
            Self::Rejected,
        ]
        .into_iter()
        .find(|status| status.as_str() == text)
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

/// Most events returned by one listing.
pub const MAX_LISTED_EVENTS: u32 = 200;

/// A stored audit event, as the Changes screen shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    #[cfg_attr(test, ts(type = "number"))]
    pub id: i64,
    pub occurred_at: String,
    /// `None` for events not tied to a connection, or one that no longer parses.
    pub connection_id: Option<ConnectionId>,
    pub environment: Option<Environment>,
    pub actor: Actor,
    pub operation: String,
    pub risk: Option<RiskLevel>,
    /// SHA-256 of the operation (64 hex characters), never its text.
    pub fingerprint: Option<String>,
    pub approval_state: Option<String>,
    pub status: AuditStatus,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub rows_affected: Option<u64>,
    pub detail: Option<String>,
}

/// Which events to list: newest first, optionally for one connection, older than an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuditFilter {
    #[serde(default)]
    pub connection_id: Option<ConnectionId>,
    /// Only events older than this one (for the next page).
    #[serde(default)]
    #[cfg_attr(test, ts(type = "number | null"))]
    pub before_id: Option<i64>,
    /// At most [`MAX_LISTED_EVENTS`].
    pub limit: u32,
}
