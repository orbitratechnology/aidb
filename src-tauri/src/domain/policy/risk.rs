//! Risk classes and the approval each requires, as the PRD defines them. Classification is
//! deterministic: it comes from the statement, its target and its estimated impact (see
//! [`super::classify`]), or from a tool and its checked arguments, never from a model's label.

use serde::{Deserialize, Serialize};

/// How much harm an operation could do, from reading structure (R0) to destroying data (R4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum RiskLevel {
    /// Introspection: list schemas, describe tables.
    R0,
    /// A bounded read with row, size and time caps.
    R1,
    /// A small, reversible write in a non-production database.
    R2,
    /// Bulk or production changes, migrations, privilege changes.
    R3,
    /// Destructive or privileged: DROP, TRUNCATE, broad deletes, server administration.
    R4,
}

/// What must happen before an operation of a given risk may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ApprovalLevel {
    /// Runs within the connection's read policy and caps.
    None,
    /// Needs a preview and an explicit Apply.
    Apply,
    /// Needs an approval that shows the SQL, target, impact and risk.
    Elevated,
    /// Never runs from the assistant.
    Blocked,
}

impl RiskLevel {
    /// `R0` to `R4`, as serialized.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::R0 => "R0",
            Self::R1 => "R1",
            Self::R2 => "R2",
            Self::R3 => "R3",
            Self::R4 => "R4",
        }
    }

    /// The approval this risk needs by default. Environment-specific rules (production writes)
    /// tighten this in the policy engine.
    #[must_use]
    pub const fn required_approval(self) -> ApprovalLevel {
        match self {
            Self::R0 | Self::R1 => ApprovalLevel::None,
            Self::R2 => ApprovalLevel::Apply,
            Self::R3 => ApprovalLevel::Elevated,
            Self::R4 => ApprovalLevel::Blocked,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_follows_risk() {
        assert!(RiskLevel::R0 < RiskLevel::R4);
        assert_eq!(RiskLevel::R1.required_approval(), ApprovalLevel::None);
        assert_eq!(RiskLevel::R2.required_approval(), ApprovalLevel::Apply);
        assert_eq!(RiskLevel::R3.required_approval(), ApprovalLevel::Elevated);
        assert_eq!(RiskLevel::R4.required_approval(), ApprovalLevel::Blocked);
        for risk in [RiskLevel::R0, RiskLevel::R1, RiskLevel::R4] {
            assert_eq!(serde_json::to_value(risk).expect("json"), risk.as_str());
        }
    }
}
