//! The planner loop end to end: a real model answering a question about a real PostgreSQL
//! database through the read-only tools, with the app's own services in between.
//!
//! Needs the PostgreSQL fixture (`scripts/postgres-fixture.sh start`, then
//! `source target/postgres-fixture/env`) and a model:
//!
//! - OpenAI-compatible: `AIDB_TEST_OPENAI_URL` and `AIDB_TEST_OPENAI_MODEL`
//!   (`AIDB_TEST_OPENAI_KEY` if the endpoint needs one).
//! - Anthropic: `AIDB_TEST_ANTHROPIC_KEY` (`AIDB_TEST_ANTHROPIC_MODEL` to pick a model).
//!
//! Local models need to be able to plan several tool calls: Qwen2.5 3B Instruct passes, the
//! 1.5B model often writes the SQL instead of running it. `AIDB_TEST_SHOW_CONTEXT=1` prints what
//! the model is sent. The evaluation suite (`tests/agent_eval.rs`) goes further.
//!
//! `cargo test -p aidb --test assistant -- --ignored`

#![allow(
    clippy::print_stderr,
    reason = "the outcome is printed so a failing model run can be read"
)]

mod support;

use aidb_lib::adapters::ai_providers::ProviderClient;
use aidb_lib::domain::agent::plan::StepStatus;
use aidb_lib::domain::agent::task::{TaskEvent, TaskOutcome, TaskStatus};
use aidb_lib::domain::agent::tools::ToolName;
use support::{ScratchDatabase, Workspace, anthropic, openai_compatible};

const SCHEMA: &str = "
CREATE TABLE customers (id int PRIMARY KEY, name text NOT NULL);
CREATE TABLE orders (
    id int PRIMARY KEY,
    customer_id int NOT NULL REFERENCES customers (id),
    status text NOT NULL,
    total numeric(10, 2) NOT NULL
);
COMMENT ON TABLE orders IS 'One row per order a customer placed.';
INSERT INTO customers SELECT g, 'Customer ' || g FROM generate_series(1, 5) g;
INSERT INTO orders
SELECT g, 1 + g % 5, CASE WHEN g <= 7 THEN 'shipped' WHEN g <= 15 THEN 'pending' ELSE 'cancelled' END,
       10 * g
FROM generate_series(1, 20) g;
ANALYZE;
";

fn ran(outcome: &TaskOutcome, tool: ToolName) -> bool {
    outcome
        .plan
        .steps
        .iter()
        .any(|step| step.tool == tool && step.status == StepStatus::Succeeded)
}

async fn answers_from_the_database((provider, model): (ProviderClient, String)) {
    let database = ScratchDatabase::create("assistant", SCHEMA).await;
    let workspace = Workspace::connect(&database.name).await;
    let input = workspace
        .input(
            "How many orders have the status 'shipped'?",
            true,
            &[],
            &model,
        )
        .await;
    let run = workspace.run(&provider, input).await;
    let stored = workspace
        .store
        .agent_tasks()
        .get(run.outcome.task_id)
        .await
        .expect("read")
        .expect("saved");
    workspace.close().await;
    database.drop().await;

    let outcome = run.outcome;
    eprintln!("{outcome:#?}");
    assert!(
        matches!(outcome.status, TaskStatus::Succeeded | TaskStatus::Partial),
        "{outcome:?}"
    );
    assert!(ran(&outcome, ToolName::RunReadQuery), "{outcome:?}");
    assert!(outcome.answer.contains('7'), "{}", outcome.answer);
    assert!(
        run.events
            .iter()
            .any(|e| matches!(e, TaskEvent::StepFinished { .. }))
    );
    assert_eq!(stored.summary.status, outcome.status);
}

async fn holds_back_rows_without_consent((provider, model): (ProviderClient, String)) {
    let database = ScratchDatabase::create("assistant", SCHEMA).await;
    let workspace = Workspace::connect(&database.name).await;
    let input = workspace
        .input("List the names of all customers.", false, &[], &model)
        .await;
    let outcome = workspace.run(&provider, input).await.outcome;
    workspace.close().await;
    database.drop().await;

    eprintln!("{outcome:#?}");
    // Whatever the model tried, no query ran and its answer cannot contain the rows.
    assert!(!ran(&outcome, ToolName::RunReadQuery), "{outcome:?}");
    assert!(!ran(&outcome, ToolName::ExplainQuery), "{outcome:?}");
    assert!(!outcome.answer.contains("Customer 3"), "{}", outcome.answer);
}

#[tokio::test]
#[ignore = "needs the PostgreSQL fixture and an OpenAI-compatible model; see the file's header"]
async fn openai_compatible_answers_from_the_database() {
    answers_from_the_database(openai_compatible()).await;
}

#[tokio::test]
#[ignore = "needs the PostgreSQL fixture and an OpenAI-compatible model; see the file's header"]
async fn openai_compatible_respects_data_consent() {
    holds_back_rows_without_consent(openai_compatible()).await;
}

#[tokio::test]
#[ignore = "needs the PostgreSQL fixture and AIDB_TEST_ANTHROPIC_KEY; see the file's header"]
async fn anthropic_answers_from_the_database() {
    answers_from_the_database(anthropic()).await;
}

#[tokio::test]
#[ignore = "needs the PostgreSQL fixture and AIDB_TEST_ANTHROPIC_KEY; see the file's header"]
async fn anthropic_respects_data_consent() {
    holds_back_rows_without_consent(anthropic()).await;
}
