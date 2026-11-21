//! The assistant's risk limit. The risk classes themselves belong to the policy engine.

pub use crate::domain::policy::risk::{ApprovalLevel, RiskLevel};

/// The highest risk the read-only assistant may plan: bounded reads.
pub const READ_ONLY_AGENT_MAX_RISK: RiskLevel = RiskLevel::R1;
