//! The task plan: what the assistant is doing for one request, step by step, and against which
//! database. Every field that matters for safety is set by Rust: the target comes from the open
//! session, each step's risk from the tool registry, and the plan's risk and required approval
//! are computed from its steps whenever they are read, never stored or taken from the model.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::risk::{ApprovalLevel, RiskLevel};
use super::tools::{TOOLS_VERSION, ToolInvocation, ToolName};
use crate::domain::connections::{ConnectionId, Environment};

/// Version of the plan format. Plans of other versions are not loaded.
pub const PLAN_VERSION: u32 = 1;
/// Most steps one task may take; a model that loops is stopped here.
pub const MAX_STEPS: usize = 25;
/// Longest stated intent, in characters.
pub const MAX_INTENT_CHARS: usize = 2_000;
/// Most assumptions recorded for a task.
pub const MAX_ASSUMPTIONS: usize = 10;

/// Stable identifier of an assistant task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, type = "string"))]
pub struct TaskId(Uuid);

impl TaskId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.hyphenated().fmt(f)
    }
}

impl FromStr for TaskId {
    type Err = uuid::Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(text).map(Self)
    }
}

impl Serialize for TaskId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for TaskId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// The database a plan runs against, taken from the open session, never from the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct PlanTarget {
    pub connection_id: ConnectionId,
    /// The database name the server reported.
    pub database: String,
    pub environment: Environment,
    /// The session rejects writes.
    pub read_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Planned,
    Running,
    Succeeded,
    Failed,
    /// Not started because the task was cancelled or stopped earlier.
    Skipped,
    Cancelled,
}

/// One tool call the assistant made or will make.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub tool: ToolName,
    /// What the step does, for the user.
    pub summary: String,
    /// The checked arguments.
    #[cfg_attr(test, ts(type = "unknown"))]
    pub input: Value,
    pub risk: RiskLevel,
    pub status: StepStatus,
    /// What happened, for the user: a row count, or why it failed.
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    NotRun,
    Passed,
    Failed,
}

/// A check that the task did what it says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct VerificationStep {
    pub description: String,
    pub status: VerificationStatus,
    pub summary: Option<String>,
}

/// One task of the assistant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TaskPlan {
    pub version: u32,
    pub tools_version: u32,
    pub task_id: TaskId,
    pub target: PlanTarget,
    /// The user's request, as they wrote it.
    pub intent: String,
    /// What the assistant assumed where the request was ambiguous.
    pub assumptions: Vec<String>,
    pub steps: Vec<PlanStep>,
    pub verification: Vec<VerificationStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("The request is empty.")]
    EmptyIntent,
    #[error("The request is too long ({0} characters at most).")]
    IntentTooLong(usize),
    #[error("The task reached its limit of {MAX_STEPS} steps.")]
    TooManySteps,
    #[error("`{tool}` is {risk:?}, above what this task may do ({max:?}).")]
    RiskTooHigh {
        tool: ToolName,
        risk: RiskLevel,
        max: RiskLevel,
    },
    #[error("The saved plan has version {0}, which this version of aidb cannot read.")]
    UnsupportedVersion(u32),
    #[error("The saved plan is unreadable: {0}")]
    Unreadable(String),
}

/// A plan as it is stored and shown: the plan plus its computed risk and approval.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, rename = "TaskPlan"))]
#[serde(rename_all = "camelCase")]
pub struct PlanView<'a> {
    #[serde(flatten)]
    #[cfg_attr(test, ts(flatten))]
    plan: &'a TaskPlan,
    risk: RiskLevel,
    required_approval: ApprovalLevel,
}

impl TaskPlan {
    /// A new, empty plan for `intent` against `target`.
    ///
    /// # Errors
    ///
    /// When the intent is empty or too long.
    pub fn new(target: PlanTarget, intent: &str) -> Result<Self, PlanError> {
        let intent = intent.trim();
        if intent.is_empty() {
            return Err(PlanError::EmptyIntent);
        }
        if intent.chars().count() > MAX_INTENT_CHARS {
            return Err(PlanError::IntentTooLong(MAX_INTENT_CHARS));
        }
        Ok(Self {
            version: PLAN_VERSION,
            tools_version: TOOLS_VERSION,
            task_id: TaskId::new(),
            target,
            intent: intent.to_owned(),
            assumptions: Vec::new(),
            steps: Vec::new(),
            verification: Vec::new(),
        })
    }

    /// The highest risk of any step; R0 for a plan with none.
    #[must_use]
    pub fn risk(&self) -> RiskLevel {
        self.steps
            .iter()
            .map(|step| step.risk)
            .max()
            .unwrap_or(RiskLevel::R0)
    }

    #[must_use]
    pub fn required_approval(&self) -> ApprovalLevel {
        self.risk().required_approval()
    }

    /// Adds a checked tool call as a planned step and returns its index. Its risk comes from the
    /// registry; a step above `max_risk` is refused.
    ///
    /// # Errors
    ///
    /// [`PlanError::RiskTooHigh`] or [`PlanError::TooManySteps`].
    pub fn add_step(
        &mut self,
        invocation: &ToolInvocation,
        max_risk: RiskLevel,
    ) -> Result<usize, PlanError> {
        let risk = invocation.risk();
        if risk > max_risk {
            return Err(PlanError::RiskTooHigh {
                tool: invocation.tool(),
                risk,
                max: max_risk,
            });
        }
        if self.steps.len() >= MAX_STEPS {
            return Err(PlanError::TooManySteps);
        }
        self.steps.push(PlanStep {
            tool: invocation.tool(),
            summary: invocation.summary(),
            input: invocation.input_json(),
            risk,
            status: StepStatus::Planned,
            outcome: None,
        });
        Ok(self.steps.len() - 1)
    }

    /// Records an assumption, up to [`MAX_ASSUMPTIONS`].
    pub fn assume(&mut self, assumption: &str) {
        let assumption = assumption.trim();
        if !assumption.is_empty() && self.assumptions.len() < MAX_ASSUMPTIONS {
            self.assumptions
                .push(assumption.chars().take(500).collect());
        }
    }

    /// Marks the steps that never started as skipped (after a cancel or a stop).
    pub fn skip_unstarted(&mut self) {
        for step in &mut self.steps {
            if step.status == StepStatus::Planned {
                step.status = StepStatus::Skipped;
            }
        }
    }

    /// The plan with its computed risk and approval, for storage and display.
    #[must_use]
    pub fn view(&self) -> PlanView<'_> {
        PlanView {
            plan: self,
            risk: self.risk(),
            required_approval: self.required_approval(),
        }
    }

    /// The plan as stored.
    ///
    /// # Errors
    ///
    /// Only if serialization fails, which these types do not.
    pub fn to_json(&self) -> Result<String, PlanError> {
        serde_json::to_string(&self.view()).map_err(|e| PlanError::Unreadable(e.to_string()))
    }

    /// Reads a stored plan. Stored risk and approval are ignored and each step's risk is taken
    /// from the registry again, so a tampered record cannot lower them.
    ///
    /// # Errors
    ///
    /// [`PlanError::UnsupportedVersion`] or [`PlanError::Unreadable`].
    pub fn from_json(json: &str) -> Result<Self, PlanError> {
        let value: Value =
            serde_json::from_str(json).map_err(|e| PlanError::Unreadable(e.to_string()))?;
        let version = value.get("version").and_then(Value::as_u64).unwrap_or(0);
        if version != u64::from(PLAN_VERSION) {
            return Err(PlanError::UnsupportedVersion(
                u32::try_from(version).unwrap_or(u32::MAX),
            ));
        }
        let mut plan: Self =
            serde_json::from_value(value).map_err(|e| PlanError::Unreadable(e.to_string()))?;
        for step in &mut plan.steps {
            step.risk = super::tools::definition(step.tool).risk;
        }
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::domain::agent::risk::READ_ONLY_AGENT_MAX_RISK;
    use crate::domain::agent::tools::ReadQueryInput;
    use crate::domain::catalog::ObjectName;

    fn target() -> PlanTarget {
        PlanTarget {
            connection_id: ConnectionId::new(),
            database: "shop".into(),
            environment: Environment::Production,
            read_only: true,
        }
    }

    fn read_query() -> ToolInvocation {
        ToolInvocation::RunReadQuery(ReadQueryInput {
            sql: "SELECT count(*) FROM orders".into(),
            purpose: "Count orders".into(),
        })
    }

    #[test]
    fn plans_compute_their_risk_from_their_steps() {
        let mut plan = TaskPlan::new(target(), "  How many orders?  ").expect("plan");
        assert_eq!(plan.intent, "How many orders?");
        assert_eq!(plan.risk(), RiskLevel::R0);

        plan.add_step(
            &ToolInvocation::DescribeTable(ObjectName {
                schema: "public".into(),
                name: "orders".into(),
            }),
            READ_ONLY_AGENT_MAX_RISK,
        )
        .expect("describe");
        assert_eq!(plan.risk(), RiskLevel::R0);
        plan.add_step(&read_query(), READ_ONLY_AGENT_MAX_RISK)
            .expect("read");
        assert_eq!(plan.risk(), RiskLevel::R1);
        assert_eq!(plan.required_approval(), ApprovalLevel::None);
        assert_eq!(plan.steps[1].summary, "Run a read query: Count orders");
        assert_eq!(plan.steps[1].input["sql"], "SELECT count(*) FROM orders");
    }

    #[test]
    fn steps_above_the_allowed_risk_and_runaway_plans_are_refused() {
        let mut plan = TaskPlan::new(target(), "Count").expect("plan");
        assert_eq!(
            plan.add_step(&read_query(), RiskLevel::R0),
            Err(PlanError::RiskTooHigh {
                tool: ToolName::RunReadQuery,
                risk: RiskLevel::R1,
                max: RiskLevel::R0,
            })
        );
        for _ in 0..MAX_STEPS {
            plan.add_step(&ToolInvocation::ListSchemas, READ_ONLY_AGENT_MAX_RISK)
                .expect("step");
        }
        assert_eq!(
            plan.add_step(&ToolInvocation::ListSchemas, READ_ONLY_AGENT_MAX_RISK),
            Err(PlanError::TooManySteps)
        );
        assert_eq!(TaskPlan::new(target(), " "), Err(PlanError::EmptyIntent));
        assert!(TaskPlan::new(target(), &"x".repeat(MAX_INTENT_CHARS + 1)).is_err());
    }

    #[test]
    fn stored_plans_round_trip_and_cannot_lower_their_risk() {
        let mut plan = TaskPlan::new(target(), "Count orders").expect("plan");
        plan.add_step(&read_query(), READ_ONLY_AGENT_MAX_RISK)
            .expect("read");
        plan.assume("Orders means rows of public.orders");
        let json = plan.to_json().expect("json");
        let stored: Value = serde_json::from_str(&json).expect("value");
        assert_eq!(stored["risk"], "R1");
        assert_eq!(stored["requiredApproval"], "none");
        assert_eq!(stored["target"]["environment"], "production");
        assert_eq!(TaskPlan::from_json(&json).expect("load"), plan);

        // A tampered record: lower risks, a fake approval. They are recomputed.
        let mut tampered = stored.clone();
        tampered["risk"] = json!("R0");
        tampered["steps"][0]["risk"] = json!("R0");
        let loaded = TaskPlan::from_json(&tampered.to_string()).expect("load");
        assert_eq!(loaded.steps[0].risk, RiskLevel::R1);
        assert_eq!(loaded.risk(), RiskLevel::R1);

        let mut future = stored;
        future["version"] = json!(2);
        assert_eq!(
            TaskPlan::from_json(&future.to_string()),
            Err(PlanError::UnsupportedVersion(2))
        );
        assert!(matches!(
            TaskPlan::from_json("{"),
            Err(PlanError::Unreadable(_))
        ));
    }

    #[test]
    fn unstarted_steps_are_skipped_on_stop() {
        let mut plan = TaskPlan::new(target(), "Count").expect("plan");
        plan.add_step(&ToolInvocation::ListSchemas, READ_ONLY_AGENT_MAX_RISK)
            .expect("step");
        plan.add_step(&read_query(), READ_ONLY_AGENT_MAX_RISK)
            .expect("step");
        plan.steps[0].status = StepStatus::Succeeded;
        plan.skip_unstarted();
        assert_eq!(
            plan.steps.iter().map(|s| s.status).collect::<Vec<_>>(),
            [StepStatus::Succeeded, StepStatus::Skipped]
        );
    }
}
