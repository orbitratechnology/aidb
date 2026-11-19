//! Risk classification (policy version 1): the risk class of each statement and of the whole
//! submission, with the reasons in words, and what must happen before it may run.
//!
//! The rules are deterministic and depend only on what the statement does ([`StatementFacts`]),
//! what the database says about its effect ([`ImpactFacts`]) and where it would run
//! ([`PolicyContext`]). Risk describes the harm an operation could do; permissions do not lower
//! it, they only block.

use serde::Serialize;

use super::facts::{Operation, SqlFacts, StatementFacts};
use super::risk::{ApprovalLevel, RiskLevel};
use crate::domain::connections::Environment;
use crate::domain::sql::SourceSpan;

/// Bumped whenever a rule changes; approvals record the version they were given under.
pub const POLICY_VERSION: u32 = 1;
/// Writes of at most this many rows count as small (R2); more are bulk writes (R3).
pub const SMALL_WRITE_ROWS: u64 = 100;

/// Where an impact estimate came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpactSource {
    /// Counted from the statement itself (an INSERT's VALUES list).
    Statement,
    /// The planner's estimate (`EXPLAIN`, which plans without running).
    Planner,
    /// Not estimated.
    Unknown,
}

/// What the database says about one statement's effect, gathered before classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImpactFacts {
    /// Rows the statement is expected to write.
    pub rows: Option<u64>,
    pub source: ImpactSource,
    /// Tables a DELETE on the target also deletes from (`ON DELETE CASCADE`).
    pub cascades_to: Vec<String>,
    /// Privileges the session's role lacks, such as "UPDATE on public.orders".
    pub missing_privileges: Vec<String>,
}

impl ImpactFacts {
    /// Nothing known: no estimate, no cascades, no missing privileges.
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            rows: None,
            source: ImpactSource::Unknown,
            cascades_to: Vec::new(),
            missing_privileges: Vec::new(),
        }
    }
}

/// Where the statements would run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicyContext {
    pub environment: Environment,
    /// The profile allows writes and the server accepts them (not read-only, not a standby).
    pub writes_allowed: bool,
}

/// Why a statement has its risk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    Read,
    PlanOnly,
    SmallWrite,
    BulkWrite,
    UnknownImpact,
    ChangesEveryRow,
    CascadingDelete,
    SchemaChange,
    Destructive,
    Comment,
    Maintenance,
    NonTransactional,
    PrivilegeChange,
    DisablesSafeguards,
    ServerAccess,
    ManagedByAidb,
    Production,
    Batch,
    Unreadable,
}

/// One reason, the risk it calls for, and what it means in words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskReason {
    pub code: ReasonCode,
    pub risk: RiskLevel,
    pub message: String,
}

/// The planner's or the statement's estimate of rows written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImpactEstimate {
    pub rows: Option<u64>,
    pub source: ImpactSource,
}

/// The risk of one statement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatementRisk {
    pub span: SourceSpan,
    pub operation: Operation,
    pub targets: Vec<String>,
    pub risk: RiskLevel,
    pub reasons: Vec<RiskReason>,
    /// Absent for statements that write nothing.
    pub impact: Option<ImpactEstimate>,
}

/// The risk of a whole submission and what it needs before it may run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskAssessment {
    pub policy_version: u32,
    /// The highest risk of any statement, raised for batches.
    pub risk: RiskLevel,
    pub required_approval: ApprovalLevel,
    pub statements: Vec<StatementRisk>,
    /// Reasons about the whole submission (a batch, text that could not be split).
    pub reasons: Vec<RiskReason>,
    /// Why it may not run at all here, in words. Non-empty means `required_approval` is
    /// `blocked`.
    pub blockers: Vec<String>,
}

const fn writes(operation: Operation) -> bool {
    !matches!(operation, Operation::Read | Operation::Explain)
}

struct Reasons(Vec<RiskReason>);

impl Reasons {
    fn add(&mut self, code: ReasonCode, risk: RiskLevel, message: impl Into<String>) {
        self.0.push(RiskReason {
            code,
            risk,
            message: message.into(),
        });
    }

    fn risk(&self) -> RiskLevel {
        self.0
            .iter()
            .map(|reason| reason.risk)
            .max()
            .unwrap_or(RiskLevel::R0)
    }
}

fn rows_text(rows: u64) -> String {
    if rows == 1 {
        "1 row".to_owned()
    } else {
        format!("{rows} rows")
    }
}

/// Reasons from the kind of operation.
fn operation_reasons(facts: &StatementFacts, impact: &ImpactFacts, reasons: &mut Reasons) {
    use RiskLevel::{R1, R2, R3, R4};
    match facts.operation {
        Operation::Read => reasons.add(
            ReasonCode::Read,
            R1,
            "Reads rows within the session's row, size and time limits.",
        ),
        Operation::Explain => reasons.add(
            ReasonCode::PlanOnly,
            R1,
            "Plans the statement without running it.",
        ),
        Operation::Insert
        | Operation::Update
        | Operation::Delete
        | Operation::Merge
        | Operation::Copy
        | Operation::OtherWrite => write_reasons(facts, impact, reasons),
        Operation::CreateObject | Operation::AlterObject => reasons.add(
            ReasonCode::SchemaChange,
            R3,
            "Changes the database's structure.",
        ),
        Operation::DropObject => reasons.add(
            ReasonCode::Destructive,
            R4,
            "Drops objects and the data in them.",
        ),
        Operation::Truncate => reasons.add(
            ReasonCode::Destructive,
            R4,
            "Removes every row of the table at once.",
        ),
        Operation::Comment => reasons.add(ReasonCode::Comment, R2, "Changes an object's comment."),
        Operation::Maintenance => reasons.add(
            ReasonCode::Maintenance,
            R3,
            "Runs maintenance that can lock tables for a long time.",
        ),
        Operation::PrivilegeChange => {
            reasons.add(ReasonCode::PrivilegeChange, R3, "Changes who may do what.")
        }
        Operation::ServerAdministration => reasons.add(
            ReasonCode::ServerAccess,
            R4,
            "Administers roles, extensions, the server or runs code on it.",
        ),
        Operation::SessionControl => reasons.add(
            ReasonCode::ManagedByAidb,
            R4,
            "Changes session settings, which aidb sets itself for safety.",
        ),
        Operation::TransactionControl => reasons.add(
            ReasonCode::ManagedByAidb,
            R4,
            "Controls transactions, which aidb manages for each change.",
        ),
        Operation::Unknown => reasons.add(
            ReasonCode::Unreadable,
            R4,
            "aidb cannot tell what this statement does.",
        ),
    }
}

fn write_reasons(facts: &StatementFacts, impact: &ImpactFacts, reasons: &mut Reasons) {
    use RiskLevel::{R2, R3, R4};
    if facts.filtered == Some(false) {
        reasons.add(
            ReasonCode::ChangesEveryRow,
            R4,
            "Has no WHERE clause that filters rows, so it would change every row.",
        );
    }
    if facts.operation == Operation::OtherWrite {
        reasons.add(
            ReasonCode::BulkWrite,
            R3,
            "Locks rows or creates a table from a query.",
        );
        return;
    }
    match impact.rows {
        Some(rows) if rows <= SMALL_WRITE_ROWS => reasons.add(
            ReasonCode::SmallWrite,
            R2,
            match impact.source {
                ImpactSource::Statement => format!("Writes {}.", rows_text(rows)),
                _ => format!("Writes about {} (estimated).", rows_text(rows)),
            },
        ),
        Some(rows) => reasons.add(
            ReasonCode::BulkWrite,
            R3,
            format!(
                "Writes about {} (estimated), more than {SMALL_WRITE_ROWS}.",
                rows_text(rows)
            ),
        ),
        None => reasons.add(
            ReasonCode::UnknownImpact,
            R3,
            "aidb cannot estimate how many rows this writes.",
        ),
    }
    if !impact.cascades_to.is_empty() {
        let broad = impact.rows.is_none_or(|rows| rows > SMALL_WRITE_ROWS);
        reasons.add(
            ReasonCode::CascadingDelete,
            if broad { R4 } else { R3 },
            format!(
                "Also deletes matching rows in {} (ON DELETE CASCADE){}.",
                impact.cascades_to.join(", "),
                if broad {
                    ", from a broad or unestimated delete"
                } else {
                    ""
                }
            ),
        );
    }
}

/// The risk of one statement where it would run.
#[must_use]
pub fn classify_statement(
    facts: &StatementFacts,
    impact: &ImpactFacts,
    environment: Environment,
) -> StatementRisk {
    use RiskLevel::{R2, R3, R4};
    let mut reasons = Reasons(Vec::new());
    operation_reasons(facts, impact, &mut reasons);

    if let Some(problem) = &facts.unreadable
        && writes(facts.operation)
    {
        reasons.add(
            ReasonCode::Unreadable,
            R4,
            format!("aidb could not read the whole statement ({problem})."),
        );
    }
    if facts.destructive && facts.operation != Operation::DropObject {
        reasons.add(
            ReasonCode::Destructive,
            R4,
            "Removes columns, constraints or dependent objects (CASCADE).",
        );
    }
    if facts.disables_safeguards {
        reasons.add(
            ReasonCode::DisablesSafeguards,
            R4,
            "Turns off a safeguard: triggers, row-level security, rules or read-only mode.",
        );
    }
    if let Some(function) = &facts.blocked_function {
        reasons.add(
            ReasonCode::ServerAccess,
            R4,
            format!("Calls {function}(), which acts outside the transaction or on the server."),
        );
    }
    if facts.server_file_access {
        reasons.add(
            ReasonCode::ServerAccess,
            R4,
            "Reads or writes a file or program on the database server.",
        );
    }
    if facts.non_transactional {
        reasons.add(
            ReasonCode::NonTransactional,
            R3,
            "Cannot run inside a transaction, so it cannot be rolled back.",
        );
    }
    if environment == Environment::Production && reasons.risk() >= R2 {
        reasons.add(ReasonCode::Production, R3, "Changes a production database.");
    }

    let risk = reasons.risk();
    StatementRisk {
        span: facts.span,
        operation: facts.operation,
        targets: facts.targets.clone(),
        risk,
        reasons: reasons.0,
        impact: writes(facts.operation).then_some(ImpactEstimate {
            rows: impact.rows,
            source: impact.source,
        }),
    }
}

/// The risk of a whole submission, and what it needs before it may run. `impacts` lines up with
/// `sql.statements`; missing entries count as unknown.
#[must_use]
pub fn assess(sql: &SqlFacts, impacts: &[ImpactFacts], context: PolicyContext) -> RiskAssessment {
    use RiskLevel::{R2, R3, R4};
    let unknown = ImpactFacts::unknown();
    let statements: Vec<StatementRisk> = sql
        .statements
        .iter()
        .enumerate()
        .map(|(index, facts)| {
            classify_statement(
                facts,
                impacts.get(index).unwrap_or(&unknown),
                context.environment,
            )
        })
        .collect();

    let mut reasons = Reasons(Vec::new());
    if let Some(problem) = &sql.problem {
        reasons.add(
            ReasonCode::Unreadable,
            R4,
            format!(
                "aidb could not split the SQL into statements: {}",
                problem.message
            ),
        );
    }
    let changes = statements
        .iter()
        .filter(|statement| writes(statement.operation))
        .count();
    if statements.len() > 1 && changes > 0 {
        reasons.add(
            ReasonCode::Batch,
            R3,
            format!(
                "Runs {} statements together, {changes} of them changing the database.",
                statements.len()
            ),
        );
    }
    let risk = statements
        .iter()
        .map(|statement| statement.risk)
        .chain(std::iter::once(reasons.risk()))
        .max()
        .unwrap_or(RiskLevel::R0);

    let mut blockers = Vec::new();
    if statements.is_empty() && sql.problem.is_none() {
        blockers.push("There is no statement to run.".to_owned());
    }
    if risk == R4 {
        blockers.push(
            "aidb does not run destructive, privileged or unreadable statements, nor ones that \
             change every row."
                .to_owned(),
        );
    }
    if risk >= R2 && !context.writes_allowed {
        blockers
            .push("The connection is read-only. Allow writes in its settings first.".to_owned());
    }
    for (index, statement) in statements.iter().enumerate() {
        if !writes(statement.operation) {
            continue;
        }
        if let Some(impact) = impacts.get(index) {
            for missing in &impact.missing_privileges {
                blockers.push(format!("The database role lacks {missing}."));
            }
        }
    }

    let required_approval = if blockers.is_empty() {
        risk.required_approval()
    } else {
        ApprovalLevel::Blocked
    };
    RiskAssessment {
        policy_version: POLICY_VERSION,
        risk,
        required_approval,
        statements,
        reasons: reasons.0,
        blockers,
    }
}

#[cfg(test)]
mod tests;
