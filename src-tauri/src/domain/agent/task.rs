//! An assistant task as it runs and as it is remembered: its status, the events the planner
//! reports while it works, and the outcome.

use serde::{Deserialize, Serialize};

use super::context::ContextItem;
use super::plan::{PlanStep, TaskId, TaskPlan};

/// Longest answer kept with a finished task, in characters.
pub const MAX_ANSWER_CHARS: usize = 20_000;
/// Earlier questions and answers a follow-up may build on.
pub const MAX_HISTORY: usize = 6;

/// The lifecycle of a task, as stored. The values are a subset of what the local store allows;
/// the rest belong to tasks that change data (later phases).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Executing,
    Verifying,
    /// Answered, and every check passed.
    Succeeded,
    /// Answered, but something it relied on failed or was cut short.
    Partial,
    Failed,
    Cancelled,
}

impl TaskStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Executing => "executing",
            Self::Verifying => "verifying",
            Self::Succeeded => "succeeded",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// # Errors
    ///
    /// For a status this version does not know (a damaged row or a later phase's task).
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "executing" => Ok(Self::Executing),
            "verifying" => Ok(Self::Verifying),
            "succeeded" => Ok(Self::Succeeded),
            "partial" => Ok(Self::Partial),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(format!("unknown task status `{other}`")),
        }
    }

    #[must_use]
    pub const fn is_finished(self) -> bool {
        !matches!(self, Self::Executing | Self::Verifying)
    }
}

/// An earlier question and its answer, for follow-ups.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Exchange {
    pub question: String,
    pub answer: String,
}

/// What the planner reports while a task runs.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TaskEvent {
    /// The task began; `items` is what was sent as context.
    Started {
        task_id: TaskId,
        items: Vec<ContextItem>,
    },
    /// The model is asked again (`round` counts from 0). Text that follows belongs to a new
    /// message; the outcome's answer is the last one.
    RoundStarted {
        #[cfg_attr(test, ts(type = "number"))]
        round: usize,
    },
    /// More answer text.
    Text { text: String },
    /// A step was accepted and is about to run.
    StepStarted {
        #[cfg_attr(test, ts(type = "number"))]
        index: usize,
        step: PlanStep,
    },
    /// A step finished, failed or was skipped.
    StepFinished {
        #[cfg_attr(test, ts(type = "number"))]
        index: usize,
        step: PlanStep,
    },
    /// The model asked for something that was not accepted; nothing ran.
    CallRejected { tool: String, reason: String },
}

/// How a task ended.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskOutcome {
    pub task_id: TaskId,
    pub status: TaskStatus,
    /// The final answer, as the model wrote it.
    pub answer: String,
    /// Why the task stopped short, when it did.
    pub notice: Option<String>,
    pub plan: TaskPlan,
}

/// A remembered task, for the list of past tasks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct TaskSummary {
    pub id: TaskId,
    pub connection_id: Option<crate::domain::connections::ConnectionId>,
    /// The question, shortened.
    pub title: String,
    pub status: TaskStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_round_trip() {
        for status in [
            TaskStatus::Executing,
            TaskStatus::Verifying,
            TaskStatus::Succeeded,
            TaskStatus::Partial,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ] {
            assert_eq!(TaskStatus::parse(status.as_str()), Ok(status));
            assert_eq!(serde_json::to_value(status).expect("json"), status.as_str());
        }
        assert!(TaskStatus::parse("awaiting_approval").is_err());
        assert!(TaskStatus::Partial.is_finished());
        assert!(!TaskStatus::Executing.is_finished());
    }
}
