//! The planner loop: one assistant task, from the question to a verified answer.
//!
//! Each round sends the conversation to the model with the tool registry. Every tool call is
//! parsed by the registry, added to the plan only within the read-only risk limit, held back when
//! it would send data values to a hosted provider without consent, and only then run. Results go
//! back framed as untrusted. The loop ends when the model answers, is cancelled, or reaches a
//! limit; deterministic checks then say what actually succeeded, and the task is saved.

mod runner;

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::watch;

use super::ai;
use crate::adapters::ai_providers::{AiProvider, ProviderErrorKind};
use crate::adapters::local_store::LocalStore;
use crate::domain::agent::context::{self, BuiltContext};
use crate::domain::agent::plan::{
    PlanError, StepStatus, TaskId, TaskPlan, VerificationStatus, VerificationStep,
};
use crate::domain::agent::risk::READ_ONLY_AGENT_MAX_RISK;
use crate::domain::agent::task::{Exchange, MAX_HISTORY, TaskEvent, TaskOutcome, TaskStatus};
use crate::domain::agent::tools::{self, ToolInvocation, ToolName};
use crate::domain::ai::{
    ChatEvent, ChatRequest, Message, OutputFormat, Part, Role, StopReason, ToolResult,
};
use crate::domain::database::OperationId;

pub use runner::SessionTools;

/// Most model calls in one task.
pub const MAX_ROUNDS: usize = 10;
/// Calls the registry refuses before the task stops: a model that keeps inventing tools will
/// not get better.
pub const MAX_REJECTED_CALLS: usize = 5;
/// Longest one model call may take.
pub const MODEL_CALL_LIMIT: Duration = Duration::from_secs(600);

/// What a tool returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    /// For the model; framed as untrusted before it is sent.
    pub text: String,
    /// For the user: what came of the step, such as "12 rows".
    pub outcome: String,
    /// The result was cut (more rows than shown).
    pub incomplete: bool,
}

/// Runs checked tool calls against the open connection.
pub trait ToolRunner: Send + Sync {
    /// # Errors
    ///
    /// A message for the model and the user, such as the database's error.
    fn run(
        &self,
        invocation: &ToolInvocation,
        operation: OperationId,
    ) -> impl Future<Output = Result<ToolOutput, String>> + Send;

    /// Stops a running operation in the database.
    fn cancel(&self, operation: OperationId) -> impl Future<Output = ()> + Send;
}

/// Tells a running task to stop.
#[derive(Debug, Clone)]
pub struct CancelSignal(watch::Receiver<bool>);

impl CancelSignal {
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }

    /// Completes when the task is cancelled; never, if it is not.
    pub async fn cancelled(&self) {
        let mut receiver = self.0.clone();
        // An error means the sender is gone: nobody can cancel any more.
        if receiver.wait_for(|cancelled| *cancelled).await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// The tasks running now, so they can be cancelled by id.
#[derive(Debug, Default)]
pub struct RunningTasks {
    tasks: Mutex<HashMap<TaskId, watch::Sender<bool>>>,
}

impl RunningTasks {
    /// Registers a task and returns its cancel signal.
    pub fn start(&self, id: TaskId) -> CancelSignal {
        let (sender, receiver) = watch::channel(false);
        self.tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, sender);
        CancelSignal(receiver)
    }

    /// Cancels a running task. Returns whether it was running.
    pub fn cancel(&self, id: TaskId) -> bool {
        self.tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id)
            .is_some_and(|sender| sender.send(true).is_ok())
    }

    pub fn finish(&self, id: TaskId) {
        self.tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
    }
}

/// Everything one task needs, prepared by the caller from the session and the profile.
#[derive(Debug, Clone)]
pub struct TaskInput {
    pub context: BuiltContext,
    pub plan: TaskPlan,
    pub history: Vec<Exchange>,
    pub model: String,
    /// Tool results with data values may go to the model: the provider is on this computer,
    /// or the user allowed it for this task.
    pub data_allowed: bool,
}

/// Why a call that the registry accepted was held back.
const DATA_CONSENT_MESSAGE: &str = "This result would contain values from the database, and the \
user has not allowed sending data values to this provider. Answer from the database structure \
alone, or tell the user that answering needs their permission to send query results.";

/// Runs a task to its end and saves it. Never fails: problems end the task with a status and a
/// notice.
pub async fn run<P: AiProvider, R: ToolRunner>(
    store: &LocalStore,
    provider: &P,
    runner: &R,
    input: TaskInput,
    cancel: &CancelSignal,
    on_event: &mut (dyn FnMut(TaskEvent) + Send),
) -> TaskOutcome {
    let TaskInput {
        context,
        mut plan,
        history,
        model,
        data_allowed,
    } = input;
    on_event(TaskEvent::Started {
        task_id: plan.task_id,
        items: context.items.clone(),
    });
    if let Err(error) = store.agent_tasks().start(&plan).await {
        tracing::warn!(task = %plan.task_id, error = %error, "could not record the task");
    }

    let mut messages = conversation(&history, &context);
    let mut state = LoopState::default();
    let mut ending: Option<(TaskStatus, Option<String>)> = None;

    for round in 0..MAX_ROUNDS {
        if cancel.is_cancelled() {
            ending = Some((TaskStatus::Cancelled, None));
            break;
        }
        on_event(TaskEvent::RoundStarted { round });
        let request = ChatRequest {
            model: model.clone(),
            system: Some(context.system.clone()),
            messages: messages.clone(),
            tools: tools::specs(),
            output: OutputFormat::Text,
            max_output_tokens: None,
        };
        let response = ai::chat(
            provider,
            &request,
            MODEL_CALL_LIMIT,
            cancel.cancelled(),
            &mut |event| {
                if let ChatEvent::Text(text) = event {
                    on_event(TaskEvent::Text { text });
                }
            },
        )
        .await;
        let response = match response {
            Ok(response) => response,
            Err(error) if error.kind == ProviderErrorKind::Cancelled => {
                ending = Some((TaskStatus::Cancelled, None));
                break;
            }
            Err(error) => {
                ending = Some((TaskStatus::Failed, Some(error.message)));
                break;
            }
        };
        let text = response.message.text();
        if !text.trim().is_empty() {
            state.answer.clone_from(&text);
        }
        messages.push(response.message.clone());
        match response.stop_reason {
            StopReason::Refusal => {
                ending = Some((
                    TaskStatus::Failed,
                    Some("The model declined to answer this request.".into()),
                ));
                break;
            }
            StopReason::MaxTokens => {
                ending = Some((
                    TaskStatus::Partial,
                    Some("The answer was cut off at the model's output limit.".into()),
                ));
                break;
            }
            _ => {}
        }
        let calls: Vec<_> = response.runnable_tool_calls().cloned().collect();
        if calls.is_empty() {
            // Models (small ones especially) sometimes write the query instead of running it.
            // They get one reminder per task; after that the verification says so.
            if data_allowed && !state.nudged && shows_sql(&text) && !ran_a_read(&plan) {
                state.nudged = true;
                messages.push(Message::user(UNRUN_SQL_REMINDER));
                continue;
            }
            break;
        }

        let mut results = Vec::with_capacity(calls.len());
        for call in &calls {
            let (content, is_error) = handle_call(
                call,
                &mut plan,
                &mut state,
                runner,
                cancel,
                data_allowed,
                &context.tag,
                on_event,
            )
            .await;
            results.push(Part::ToolResult(ToolResult {
                call_id: call.id.clone(),
                content,
                is_error,
            }));
            if cancel.is_cancelled() {
                break;
            }
        }
        if cancel.is_cancelled() {
            ending = Some((TaskStatus::Cancelled, None));
            break;
        }
        messages.push(Message {
            role: Role::User,
            parts: results,
        });
        if state.rejected >= MAX_REJECTED_CALLS {
            ending = Some((
                TaskStatus::Failed,
                Some("The model kept asking for tools or arguments that are not allowed.".into()),
            ));
            break;
        }
        if state.step_limit {
            ending = Some((
                TaskStatus::Partial,
                Some(format!(
                    "The task stopped at its limit of {} steps.",
                    crate::domain::agent::plan::MAX_STEPS
                )),
            ));
            break;
        }
        if round + 1 == MAX_ROUNDS {
            ending = Some((
                TaskStatus::Partial,
                Some(format!(
                    "The task stopped after {MAX_ROUNDS} rounds without a final answer."
                )),
            ));
        }
        if let Err(error) = store
            .agent_tasks()
            .update(&plan, TaskStatus::Executing, None)
            .await
        {
            tracing::debug!(error = %error, "could not save the task's progress");
        }
    }

    plan.skip_unstarted();
    let (status, notice) = finish(&mut plan, &state, ending);
    let answer = state.answer;
    if let Err(error) = store
        .agent_tasks()
        .update(
            &plan,
            status,
            Some(&answer).filter(|a| !a.is_empty()).map(String::as_str),
        )
        .await
    {
        tracing::warn!(task = %plan.task_id, error = %error, "could not save the finished task");
    }
    tracing::info!(task = %plan.task_id, status = status.as_str(), steps = plan.steps.len(), "task finished");
    TaskOutcome {
        task_id: plan.task_id,
        status,
        answer,
        notice,
        plan,
    }
}

#[derive(Debug, Default)]
struct LoopState {
    answer: String,
    rejected: usize,
    step_limit: bool,
    /// A result had more rows than were shown.
    incomplete: bool,
    /// The model was reminded to run the SQL it wrote.
    nudged: bool,
}

/// Sent when the model ends its turn showing SQL that it has not run.
const UNRUN_SQL_REMINDER: &str = "You showed SQL but did not run it, so the answer is not based \
on the database. Run it with run_read_query and answer from its result. If it cannot run, say \
why.";

/// The text shows a query in a ```sql block.
fn shows_sql(text: &str) -> bool {
    text.to_ascii_lowercase().contains("```sql")
}

/// A query or plan ran in this task.
fn ran_a_read(plan: &TaskPlan) -> bool {
    plan.steps.iter().any(|step| {
        matches!(step.tool, ToolName::RunReadQuery | ToolName::ExplainQuery)
            && step.status == StepStatus::Succeeded
    })
}

/// Earlier exchanges (most recent last), then the context and question.
fn conversation(history: &[Exchange], context: &BuiltContext) -> Vec<Message> {
    let skip = history.len().saturating_sub(MAX_HISTORY);
    let mut messages = Vec::new();
    for exchange in &history[skip..] {
        messages.push(Message::user(exchange.question.clone()));
        messages.push(Message {
            role: Role::Assistant,
            parts: vec![Part::Text(exchange.answer.clone())],
        });
    }
    messages.push(Message::user(context.message.clone()));
    messages
}

/// Checks and runs one call. Returns the tool result for the model and whether it is an error.
#[allow(clippy::too_many_arguments)]
async fn handle_call<R: ToolRunner>(
    call: &crate::domain::ai::ToolCall,
    plan: &mut TaskPlan,
    state: &mut LoopState,
    runner: &R,
    cancel: &CancelSignal,
    data_allowed: bool,
    tag: &str,
    on_event: &mut (dyn FnMut(TaskEvent) + Send),
) -> (String, bool) {
    let invocation = match tools::parse(call) {
        Ok(invocation) => invocation,
        Err(error) => {
            state.rejected += 1;
            on_event(TaskEvent::CallRejected {
                tool: call.name.chars().take(64).collect(),
                reason: error.to_string(),
            });
            return (error.to_string(), true);
        }
    };
    let index = match plan.add_step(&invocation, READ_ONLY_AGENT_MAX_RISK) {
        Ok(index) => index,
        Err(PlanError::TooManySteps) => {
            state.step_limit = true;
            return (
                "The task reached its step limit. Answer with what you have found.".into(),
                true,
            );
        }
        Err(error) => {
            state.rejected += 1;
            on_event(TaskEvent::CallRejected {
                tool: call.name.clone(),
                reason: error.to_string(),
            });
            return (error.to_string(), true);
        }
    };
    if tools::definition(invocation.tool()).returns_data && !data_allowed {
        let step = &mut plan.steps[index];
        step.status = StepStatus::Skipped;
        step.outcome = Some("Needs your permission to send data values to this provider.".into());
        on_event(TaskEvent::StepFinished {
            index,
            step: step.clone(),
        });
        return (DATA_CONSENT_MESSAGE.into(), true);
    }

    plan.steps[index].status = StepStatus::Running;
    on_event(TaskEvent::StepStarted {
        index,
        step: plan.steps[index].clone(),
    });
    let operation = OperationId::new();
    let result = tokio::select! {
        biased;
        () = cancel.cancelled() => {
            runner.cancel(operation).await;
            None
        }
        result = runner.run(&invocation, operation) => Some(result),
    };
    let step = &mut plan.steps[index];
    let reply = match result {
        None => {
            step.status = StepStatus::Cancelled;
            step.outcome = Some("Cancelled.".into());
            ("The task was cancelled.".to_owned(), true)
        }
        Some(Ok(output)) => {
            step.status = StepStatus::Succeeded;
            step.outcome = Some(output.outcome);
            state.incomplete |= output.incomplete;
            (
                context::tool_result(tag, invocation.tool().as_str(), &output.text),
                false,
            )
        }
        Some(Err(message)) => {
            step.status = StepStatus::Failed;
            step.outcome = Some(short(&message, 300));
            // Database errors can quote data, so they are framed like any result.
            (
                context::tool_result(tag, invocation.tool().as_str(), &message),
                true,
            )
        }
    };
    on_event(TaskEvent::StepFinished {
        index,
        step: step.clone(),
    });
    reply
}

fn short(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_owned(),
    }
}

/// Deterministic checks of what actually succeeded, and the task's final status.
fn finish(
    plan: &mut TaskPlan,
    state: &LoopState,
    ending: Option<(TaskStatus, Option<String>)>,
) -> (TaskStatus, Option<String>) {
    let mut checks = vec![check(
        "The assistant gave an answer.",
        !state.answer.trim().is_empty(),
        None,
    )];
    let reads: Vec<_> = plan
        .steps
        .iter()
        .filter(|step| matches!(step.tool, ToolName::RunReadQuery | ToolName::ExplainQuery))
        .collect();
    if let Some(last) = reads.last() {
        checks.push(check(
            "The last query it relied on ran successfully.",
            last.status == StepStatus::Succeeded,
            last.outcome.clone(),
        ));
        if reads
            .iter()
            .any(|step| step.status == StepStatus::Succeeded)
        {
            checks.push(check(
                "Every result it saw was complete.",
                !state.incomplete,
                state.incomplete.then(|| {
                    format!(
                        "At least one result had more rows than the {} shown.",
                        tools::MAX_RESULT_ROWS
                    )
                }),
            ));
        }
    }
    if shows_sql(&state.answer) {
        let ran = ran_a_read(plan);
        checks.push(check(
            "The SQL it shows was run.",
            ran,
            (!ran).then(|| "The answer shows SQL, but no query ran in this task.".to_owned()),
        ));
    }
    let all_passed = checks
        .iter()
        .all(|c| c.status == VerificationStatus::Passed);
    plan.verification = checks;

    match ending {
        Some(ending) => ending,
        None if state.answer.trim().is_empty() => {
            (TaskStatus::Failed, Some("The model gave no answer.".into()))
        }
        None if all_passed => (TaskStatus::Succeeded, None),
        None => (TaskStatus::Partial, None),
    }
}

fn check(description: &str, passed: bool, summary: Option<String>) -> VerificationStep {
    VerificationStep {
        description: description.into(),
        status: if passed {
            VerificationStatus::Passed
        } else {
            VerificationStatus::Failed
        },
        summary,
    }
}

#[cfg(test)]
mod tests;
