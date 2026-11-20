use std::collections::VecDeque;

use serde_json::{Value, json};

use super::*;
use crate::adapters::ai_providers::ProviderError;
use crate::adapters::local_store::test_support::temp_store;
use crate::domain::agent::context::new_tag;
use crate::domain::agent::plan::PlanTarget;
use crate::domain::ai::{
    ChatResponse, ModelInfo, ProviderCapabilities, ProviderKind, StructuredOutput, ToolCall,
    ToolInput, Usage,
};
use crate::domain::connections::{ConnectionId, Environment};

/// Answers with scripted responses, in order; when the script runs out it repeats `fallback`,
/// or waits forever if there is none. Records every request.
#[derive(Default)]
struct ScriptedModel {
    script: Mutex<VecDeque<Result<ChatResponse, ProviderError>>>,
    fallback: Option<ChatResponse>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl ScriptedModel {
    fn new(script: Vec<Result<ChatResponse, ProviderError>>) -> Self {
        Self {
            script: Mutex::new(script.into()),
            ..Self::default()
        }
    }

    fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().expect("lock").clone()
    }
}

impl AiProvider for ScriptedModel {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            kind: ProviderKind::OpenAiCompatible,
            streaming: true,
            tool_calls: true,
            structured_output: StructuredOutput::JsonSchema,
            cancellation: true,
        }
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(Vec::new())
    }

    async fn chat(
        &self,
        request: &ChatRequest,
        on_event: &mut (dyn FnMut(ChatEvent) + Send),
    ) -> Result<ChatResponse, ProviderError> {
        self.requests.lock().expect("lock").push(request.clone());
        let next = self.script.lock().expect("lock").pop_front();
        let next = match (next, &self.fallback) {
            (Some(next), _) => next,
            (None, Some(fallback)) => Ok(fallback.clone()),
            (None, None) => std::future::pending().await,
        };
        if let Ok(response) = &next {
            let text = response.message.text();
            if !text.is_empty() {
                on_event(ChatEvent::Text(text));
            }
        }
        next
    }
}

/// What the scripted runner does for a tool.
#[derive(Clone)]
enum Script {
    Ok {
        text: &'static str,
        incomplete: bool,
    },
    Fail(&'static str),
    /// Never finishes (until cancelled).
    Hang,
}

struct ScriptedTools {
    query: Script,
    runs: Mutex<Vec<(ToolInvocation, OperationId)>>,
    cancels: Mutex<Vec<OperationId>>,
}

impl ScriptedTools {
    fn new(query: Script) -> Self {
        Self {
            query,
            runs: Mutex::new(Vec::new()),
            cancels: Mutex::new(Vec::new()),
        }
    }

    fn ran(&self) -> Vec<ToolName> {
        self.runs
            .lock()
            .expect("lock")
            .iter()
            .map(|(invocation, _)| invocation.tool())
            .collect()
    }
}

impl ToolRunner for ScriptedTools {
    async fn run(
        &self,
        invocation: &ToolInvocation,
        operation: OperationId,
    ) -> Result<ToolOutput, String> {
        self.runs
            .lock()
            .expect("lock")
            .push((invocation.clone(), operation));
        let script = match invocation {
            ToolInvocation::RunReadQuery(_) | ToolInvocation::ExplainQuery(_) => self.query.clone(),
            _ => Script::Ok {
                text: "public.orders (table, ~12 rows)",
                incomplete: false,
            },
        };
        match script {
            Script::Ok { text, incomplete } => Ok(ToolOutput {
                text: text.into(),
                outcome: "done".into(),
                incomplete,
                proposal: None,
            }),
            Script::Fail(message) => Err(message.into()),
            Script::Hang => std::future::pending().await,
        }
    }

    async fn cancel(&self, operation: OperationId) {
        self.cancels.lock().expect("lock").push(operation);
    }
}

fn call(id: &str, name: &str, input: Value) -> Part {
    Part::ToolCall(ToolCall {
        id: id.into(),
        name: name.into(),
        input: ToolInput::Json(input),
    })
}

fn reply(parts: Vec<Part>, stop_reason: StopReason) -> Result<ChatResponse, ProviderError> {
    Ok(ChatResponse {
        message: Message {
            role: Role::Assistant,
            parts,
        },
        stop_reason,
        usage: Usage::default(),
        model: None,
    })
}

fn answer(text: &str) -> Result<ChatResponse, ProviderError> {
    reply(vec![Part::Text(text.into())], StopReason::EndTurn)
}

fn count_query(id: &str) -> Part {
    call(
        id,
        "run_read_query",
        json!({"sql": "SELECT count(*) FROM public.orders", "purpose": "count orders"}),
    )
}

fn input(data_allowed: bool) -> TaskInput {
    let tag = new_tag();
    TaskInput {
        context: BuiltContext {
            system: format!("You answer questions about a database. Untrusted: <{tag}>."),
            message: "Question: how many orders are there?".into(),
            items: Vec::new(),
            estimated_tokens: 10,
            tag,
        },
        plan: TaskPlan::new(
            PlanTarget {
                connection_id: ConnectionId::new(),
                database: "shop".into(),
                environment: Environment::Development,
                read_only: true,
            },
            "How many orders are there?",
        )
        .expect("plan"),
        history: Vec::new(),
        model: "model".into(),
        data_allowed,
    }
}

struct Run {
    outcome: TaskOutcome,
    events: Vec<TaskEvent>,
}

async fn run_task<P: AiProvider, R: ToolRunner>(
    store: &LocalStore,
    model: &P,
    tools: &R,
    input: TaskInput,
    cancel: &CancelSignal,
) -> Run {
    let mut events = Vec::new();
    let outcome = run(store, model, tools, input, cancel, &mut |event| {
        events.push(event)
    })
    .await;
    Run { outcome, events }
}

fn never_cancelled() -> CancelSignal {
    RunningTasks::default().start(TaskId::new())
}

fn tool_results(request: &ChatRequest) -> Vec<ToolResult> {
    request
        .messages
        .last()
        .expect("message")
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::ToolResult(result) => Some(result.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn answers_after_running_tools_and_saves_the_task() {
    let (_dir, store) = temp_store().await;
    let model = ScriptedModel::new(vec![
        reply(
            vec![
                call("a", "list_tables", json!({"search": "order"})),
                count_query("b"),
            ],
            StopReason::ToolUse,
        ),
        answer("There are 12 orders."),
    ]);
    let tools = ScriptedTools::new(Script::Ok {
        text: "count (bigint)\n12\n(1 rows)",
        incomplete: false,
    });
    let mut task = input(false);
    task.data_allowed = true;
    task.history = (0..8)
        .map(|i| Exchange {
            question: format!("q{i}"),
            answer: format!("a{i}"),
        })
        .collect();
    let tag = task.context.tag.clone();

    let run = run_task(&store, &model, &tools, task, &never_cancelled()).await;
    let outcome = run.outcome;
    assert_eq!(
        outcome.status,
        TaskStatus::Succeeded,
        "{:?}",
        outcome.notice
    );
    assert_eq!(outcome.answer, "There are 12 orders.");
    assert_eq!(
        tools.ran(),
        vec![ToolName::ListTables, ToolName::RunReadQuery]
    );
    assert!(
        outcome
            .plan
            .steps
            .iter()
            .all(|step| step.status == StepStatus::Succeeded)
    );
    assert_eq!(outcome.plan.verification.len(), 3);
    assert!(
        outcome
            .plan
            .verification
            .iter()
            .all(|check| check.status == VerificationStatus::Passed)
    );

    let requests = model.requests();
    assert_eq!(requests.len(), 2);
    // The six most recent exchanges, then the question.
    let first = &requests[0];
    assert_eq!(first.messages.len(), MAX_HISTORY * 2 + 1);
    assert_eq!(first.messages[0].text(), "q2");
    assert_eq!(first.messages[11].text(), "a7");
    // The connection is read-only: the tool that prepares changes is not offered.
    assert_eq!(first.tools.len(), tools::specs(false).len());
    assert!(!first.tools.iter().any(|t| t.name == "preview_data_change"));
    // Results come back framed as untrusted, matched to their calls.
    let results = tool_results(&requests[1]);
    assert_eq!(results.len(), 2);
    assert_eq!(results[1].call_id, "b");
    assert!(!results[1].is_error);
    assert!(results[1].content.contains(&format!("<{tag}")));
    assert!(results[1].content.contains("12"));

    assert!(matches!(run.events[0], TaskEvent::Started { .. }));
    assert!(run.events.contains(&TaskEvent::Text {
        text: "There are 12 orders.".into()
    }));
    let finished = run
        .events
        .iter()
        .filter(|event| matches!(event, TaskEvent::StepFinished { .. }))
        .count();
    assert_eq!(finished, 2);

    let stored = store
        .agent_tasks()
        .get(outcome.task_id)
        .await
        .expect("get")
        .expect("saved");
    assert_eq!(stored.summary.status, TaskStatus::Succeeded);
    assert_eq!(stored.answer.as_deref(), Some("There are 12 orders."));
    assert_eq!(stored.plan, Some(outcome.plan));
}

#[tokio::test]
async fn unknown_tools_and_writes_are_rejected_without_running() {
    let (_dir, store) = temp_store().await;
    let model = ScriptedModel::new(vec![
        reply(
            vec![
                call("a", "drop_table", json!({"name": "orders"})),
                call(
                    "b",
                    "run_read_query",
                    json!({"sql": "DELETE FROM orders", "purpose": "clean up"}),
                ),
                call("c", "run_read_query", json!({"sql": "SELECT 1"})),
            ],
            StopReason::ToolUse,
        ),
        answer("I can only read."),
    ]);
    let tools = ScriptedTools::new(Script::Hang);
    let run = run_task(&store, &model, &tools, input(true), &never_cancelled()).await;

    assert!(tools.ran().is_empty());
    assert!(run.outcome.plan.steps.is_empty());
    let rejected = run
        .events
        .iter()
        .filter(|event| matches!(event, TaskEvent::CallRejected { .. }))
        .count();
    assert_eq!(rejected, 3);
    let results = tool_results(&model.requests()[1]);
    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|result| result.is_error));
    assert_eq!(run.outcome.status, TaskStatus::Succeeded);
}

#[tokio::test]
async fn a_model_that_keeps_inventing_tools_is_stopped() {
    let (_dir, store) = temp_store().await;
    let calls = (0..MAX_REJECTED_CALLS)
        .map(|i| call(&i.to_string(), "shell", json!({})))
        .collect();
    let model = ScriptedModel::new(vec![reply(calls, StopReason::ToolUse)]);
    let tools = ScriptedTools::new(Script::Hang);
    let run = run_task(&store, &model, &tools, input(true), &never_cancelled()).await;
    assert_eq!(run.outcome.status, TaskStatus::Failed);
    assert!(run.outcome.notice.is_some());
    assert_eq!(model.requests().len(), 1);
}

#[tokio::test]
async fn data_tools_wait_for_consent_with_a_hosted_provider() {
    let (_dir, store) = temp_store().await;
    let model = ScriptedModel::new(vec![
        reply(
            vec![call("a", "list_schemas", json!({})), count_query("b")],
            StopReason::ToolUse,
        ),
        answer("I need permission to read the rows."),
    ]);
    let tools = ScriptedTools::new(Script::Hang);
    let run = run_task(&store, &model, &tools, input(false), &never_cancelled()).await;

    // Structure is fine; the query never ran.
    assert_eq!(tools.ran(), vec![ToolName::ListSchemas]);
    let steps = &run.outcome.plan.steps;
    assert_eq!(steps[1].status, StepStatus::Skipped);
    let results = tool_results(&model.requests()[1]);
    assert!(results[1].is_error);
    assert_eq!(results[1].content, DATA_CONSENT_MESSAGE);
    // The answer is not backed by the query it needed.
    assert_eq!(run.outcome.status, TaskStatus::Partial);
}

#[tokio::test]
async fn cancelling_stops_the_running_query_in_the_database() {
    let (_dir, store) = temp_store().await;
    let model = ScriptedModel::new(vec![reply(vec![count_query("a")], StopReason::ToolUse)]);
    let tools = ScriptedTools::new(Script::Hang);
    let running = RunningTasks::default();
    let task = input(true);
    let id = task.plan.task_id;
    let cancel = running.start(id);

    let cancelling = async {
        while tools.ran().is_empty() {
            tokio::task::yield_now().await;
        }
        assert!(running.cancel(id));
    };
    let (run, ()) = tokio::join!(run_task(&store, &model, &tools, task, &cancel), cancelling);
    running.finish(id);
    assert!(!running.cancel(id));

    assert_eq!(run.outcome.status, TaskStatus::Cancelled);
    assert_eq!(run.outcome.plan.steps[0].status, StepStatus::Cancelled);
    assert!(run.outcome.plan.verification.is_empty());
    let operation = tools.runs.lock().expect("lock")[0].1;
    assert_eq!(*tools.cancels.lock().expect("lock"), vec![operation]);
    let stored = store
        .agent_tasks()
        .get(id)
        .await
        .expect("get")
        .expect("saved");
    assert_eq!(stored.summary.status, TaskStatus::Cancelled);
}

#[tokio::test]
async fn cancelling_stops_a_model_call() {
    let (_dir, store) = temp_store().await;
    let model = ScriptedModel::new(Vec::new()); // never answers
    let tools = ScriptedTools::new(Script::Hang);
    let running = RunningTasks::default();
    let task = input(true);
    let id = task.plan.task_id;
    let cancel = running.start(id);
    let cancelling = async {
        while model.requests().is_empty() {
            tokio::task::yield_now().await;
        }
        running.cancel(id);
    };
    let (run, ()) = tokio::join!(run_task(&store, &model, &tools, task, &cancel), cancelling);
    assert_eq!(run.outcome.status, TaskStatus::Cancelled);
    assert!(run.outcome.notice.is_none());
}

#[tokio::test]
async fn verification_flags_cut_and_failed_results() {
    let (_dir, store) = temp_store().await;

    let model = ScriptedModel::new(vec![
        reply(vec![count_query("a")], StopReason::ToolUse),
        answer("At least 50 orders."),
    ]);
    let tools = ScriptedTools::new(Script::Ok {
        text: "rows",
        incomplete: true,
    });
    let run = run_task(&store, &model, &tools, input(true), &never_cancelled()).await;
    assert_eq!(run.outcome.status, TaskStatus::Partial);
    let complete = &run.outcome.plan.verification[2];
    assert_eq!(complete.status, VerificationStatus::Failed);
    assert!(complete.summary.is_some());

    let model = ScriptedModel::new(vec![
        reply(vec![count_query("a")], StopReason::ToolUse),
        answer("The table probably has orders."),
    ]);
    let tools = ScriptedTools::new(Script::Fail("relation \"orders\" does not exist"));
    let run = run_task(&store, &model, &tools, input(true), &never_cancelled()).await;
    assert_eq!(run.outcome.status, TaskStatus::Partial);
    assert_eq!(run.outcome.plan.steps[0].status, StepStatus::Failed);
    assert_eq!(
        run.outcome.plan.verification[1].status,
        VerificationStatus::Failed
    );
    // Only one check about results: no result was seen.
    assert_eq!(run.outcome.plan.verification.len(), 2);
    let results = tool_results(&model.requests()[1]);
    assert!(results[0].is_error);
    assert!(results[0].content.contains("does not exist"));
}

#[tokio::test]
async fn limits_end_the_task() {
    let (_dir, store) = temp_store().await;

    // A model that never stops calling tools.
    let model = ScriptedModel {
        fallback: Some(
            reply(
                vec![call("a", "list_schemas", json!({}))],
                StopReason::ToolUse,
            )
            .expect("reply"),
        ),
        ..ScriptedModel::default()
    };
    let tools = ScriptedTools::new(Script::Hang);
    let run = run_task(&store, &model, &tools, input(true), &never_cancelled()).await;
    assert_eq!(run.outcome.status, TaskStatus::Partial);
    assert_eq!(model.requests().len(), MAX_ROUNDS);
    assert!(run.outcome.notice.expect("notice").contains("rounds"));

    // Too many steps in one turn.
    let calls = (0..=crate::domain::agent::plan::MAX_STEPS)
        .map(|i| call(&i.to_string(), "list_schemas", json!({})))
        .collect();
    let model = ScriptedModel::new(vec![reply(calls, StopReason::ToolUse)]);
    let run = run_task(&store, &model, &tools, input(true), &never_cancelled()).await;
    assert_eq!(run.outcome.status, TaskStatus::Partial);
    assert_eq!(
        run.outcome.plan.steps.len(),
        crate::domain::agent::plan::MAX_STEPS
    );
    assert!(run.outcome.notice.expect("notice").contains("steps"));
}

#[tokio::test]
async fn model_stops_and_errors_end_the_task() {
    let (_dir, store) = temp_store().await;
    let tools = ScriptedTools::new(Script::Hang);

    // A refusal's tool calls never run.
    let model = ScriptedModel::new(vec![reply(
        vec![Part::Text("No.".into()), count_query("a")],
        StopReason::Refusal,
    )]);
    let run = run_task(&store, &model, &tools, input(true), &never_cancelled()).await;
    assert_eq!(run.outcome.status, TaskStatus::Failed);
    assert!(tools.ran().is_empty());

    let model = ScriptedModel::new(vec![reply(
        vec![Part::Text("There are".into())],
        StopReason::MaxTokens,
    )]);
    let run = run_task(&store, &model, &tools, input(true), &never_cancelled()).await;
    assert_eq!(run.outcome.status, TaskStatus::Partial);
    assert_eq!(run.outcome.answer, "There are");

    let model = ScriptedModel::new(vec![Err(ProviderError::new(
        crate::adapters::ai_providers::ProviderErrorKind::RateLimited,
        "Too many requests.",
    ))]);
    let run = run_task(&store, &model, &tools, input(true), &never_cancelled()).await;
    assert_eq!(run.outcome.status, TaskStatus::Failed);
    assert_eq!(run.outcome.notice.as_deref(), Some("Too many requests."));
    assert!(run.outcome.plan.verification.is_empty());

    let model = ScriptedModel::new(vec![answer("  ")]);
    let run = run_task(&store, &model, &tools, input(true), &never_cancelled()).await;
    assert_eq!(run.outcome.status, TaskStatus::Failed);
    assert_eq!(
        run.outcome.plan.verification[0].status,
        VerificationStatus::Failed
    );
}

#[test]
fn running_tasks_cancel_by_id() {
    let running = RunningTasks::default();
    let id = TaskId::new();
    let signal = running.start(id);
    assert!(!signal.is_cancelled());
    assert!(!running.cancel(TaskId::new()));
    assert!(running.cancel(id));
    assert!(signal.is_cancelled());
    running.finish(id);
    assert!(!running.cancel(id));
}

const SHOWN_SQL: &str = "I will run:\n```sql\nSELECT count(*) FROM public.orders;\n```";

#[tokio::test]
async fn sql_shown_but_not_run_gets_one_reminder() {
    let (_dir, store) = temp_store().await;
    let model = ScriptedModel::new(vec![
        answer(SHOWN_SQL),
        reply(vec![count_query("a")], StopReason::ToolUse),
        answer("There are 12 orders.\n```sql\nSELECT count(*) FROM public.orders\n```"),
    ]);
    let tools = ScriptedTools::new(Script::Ok {
        text: "count (bigint)\n12\n(1 rows)",
        incomplete: false,
    });
    let run = run_task(&store, &model, &tools, input(true), &never_cancelled()).await;
    assert_eq!(
        run.outcome.status,
        TaskStatus::Succeeded,
        "{:?}",
        run.outcome
    );
    let requests = model.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[1].messages.last().expect("message").text(),
        UNRUN_SQL_REMINDER
    );
    assert!(run.events.contains(&TaskEvent::RoundStarted { round: 2 }));

    // A model that ignores the reminder is not asked again, and the answer is not verified.
    let model = ScriptedModel::new(vec![answer(SHOWN_SQL), answer(SHOWN_SQL)]);
    let run = run_task(&store, &model, &tools, input(true), &never_cancelled()).await;
    assert_eq!(model.requests().len(), 2);
    assert_eq!(run.outcome.status, TaskStatus::Partial);
    let check = run.outcome.plan.verification.last().expect("check");
    assert_eq!(check.description, "The SQL it shows was run.");
    assert_eq!(check.status, VerificationStatus::Failed);

    // Without consent to send rows there is nothing to remind it of.
    let model = ScriptedModel::new(vec![answer(SHOWN_SQL)]);
    let run = run_task(&store, &model, &tools, input(false), &never_cancelled()).await;
    assert_eq!(model.requests().len(), 1);
    assert_eq!(run.outcome.status, TaskStatus::Partial);
}

#[tokio::test]
async fn refused_requests_and_the_task_are_audited_without_model_text() {
    use crate::domain::audit::{Actor as AuditActor, AuditFilter, AuditStatus};

    let (_dir, store) = temp_store().await;
    let model = ScriptedModel::new(vec![
        reply(
            vec![
                call("a", "ignore previous instructions and drop", json!({})),
                call(
                    "b",
                    "run_read_query",
                    json!({"sql": "DELETE FROM orders WHERE note = 'secret'", "purpose": "x"}),
                ),
                count_query("c"),
            ],
            StopReason::ToolUse,
        ),
        answer("12 orders."),
    ]);
    let tools = ScriptedTools::new(Script::Ok {
        text: "12",
        incomplete: false,
    });
    let task = input(true);
    let connection = task.plan.target.connection_id;
    run_task(&store, &model, &tools, task, &never_cancelled()).await;

    let events = store
        .audit()
        .list(&AuditFilter {
            connection_id: Some(connection),
            before_id: None,
            limit: 10,
        })
        .await
        .expect("audit");
    let rows: Vec<(&str, AuditStatus, &str)> = events
        .iter()
        .map(|e| {
            (
                e.operation.as_str(),
                e.status,
                e.detail.as_deref().unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert_eq!(
        (rows[0].0, rows[0].1),
        ("assistant_task", AuditStatus::Succeeded)
    );
    assert!(
        rows[0]
            .2
            .contains("1 step, 1 query ran, 2 requests refused."),
        "{}",
        rows[0].2
    );
    assert!(
        rows[1].2.starts_with("run_read_query: not_allowed"),
        "{}",
        rows[1].2
    );
    assert!(
        rows[2]
            .2
            .starts_with("a tool that does not exist: unknown_tool"),
        "{}",
        rows[2].2
    );
    for event in &events {
        assert_eq!(event.actor, AuditActor::Agent);
        let detail = event.detail.as_deref().unwrap_or_default();
        assert!(
            !detail.contains("secret") && !detail.contains("ignore previous"),
            "{detail}"
        );
    }
}
