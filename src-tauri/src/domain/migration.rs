//! Applying a schema migration: what the user reviews before approving it, what happened, and
//! the record kept of it.

use serde::Serialize;

use crate::domain::connections::{ConnectionId, Environment};
use crate::domain::policy::risk::{ApprovalLevel, RiskLevel};

/// The longest detail kept in a record, in characters.
pub const MAX_RECORD_DETAIL_CHARS: usize = 4_000;

/// Records listed at a time.
pub const MAX_LISTED_RECORDS: u32 = 100;

/// A comparison prepared for applying: held by the core for approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct MigrationPreview {
    #[cfg_attr(test, ts(type = "string"))]
    pub approval_id: crate::domain::policy::approval::ApprovalId,
    #[cfg_attr(test, ts(type = "string"))]
    pub diff_id: uuid::Uuid,
    pub steps: usize,
    /// Summaries of the steps that delete data; applying needs them acknowledged.
    pub data_loss: Vec<String>,
    /// Steps that run after the transaction.
    pub outside_transaction: usize,
    pub risk: RiskLevel,
    pub approval: ApprovalLevel,
    /// Why it has its risk, in words.
    pub reasons: Vec<String>,
    /// The database name to type, for elevated approvals.
    pub confirmation: Option<String>,
    /// When the approval lapses, RFC 3339.
    pub expires_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum MigrationStatus {
    Running,
    /// Every step ran.
    Succeeded,
    /// A step in the transaction failed: everything was rolled back.
    Failed,
    /// The transaction committed but a step after it failed.
    Partial,
    /// The commit was sent and the connection lost: whether it committed is unknown.
    Uncertain,
}

impl MigrationStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Partial => "partial",
            Self::Uncertain => "uncertain",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        [
            Self::Running,
            Self::Succeeded,
            Self::Failed,
            Self::Partial,
            Self::Uncertain,
        ]
        .into_iter()
        .find(|status| status.as_str() == text)
    }
}

/// Whether the schema matched the source after the migration ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum Verification {
    Passed,
    Failed,
    NotRun,
}

impl Verification {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::NotRun => "not_run",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        [Self::Passed, Self::Failed, Self::NotRun]
            .into_iter()
            .find(|verification| verification.as_str() == text)
    }
}

/// The step that failed, counted from 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct FailedStep {
    pub number: usize,
    pub summary: String,
    pub message: String,
}

/// What applying a migration did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct MigrationResult {
    #[cfg_attr(test, ts(type = "string"))]
    pub record_id: uuid::Uuid,
    pub status: MigrationStatus,
    pub applied_steps: usize,
    pub total_steps: usize,
    pub failed_step: Option<FailedStep>,
    pub verification: Verification,
    /// Differences left after it ran, when verified.
    pub remaining: usize,
    /// What happened, in words.
    pub message: String,
}

/// A migration as recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct MigrationRecord {
    #[cfg_attr(test, ts(type = "string"))]
    pub id: uuid::Uuid,
    pub connection_id: Option<ConnectionId>,
    pub environment: Option<Environment>,
    pub target_schema: String,
    /// What it was compared with: "staging on Dev", "shop.sql (public)".
    pub source: String,
    pub script_sha256: String,
    pub steps: u32,
    pub applied_steps: u32,
    pub risk: RiskLevel,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: MigrationStatus,
    pub verification: Verification,
    pub detail: Option<String>,
}
