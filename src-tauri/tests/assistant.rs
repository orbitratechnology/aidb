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
//! the model is sent.
//!
//! `cargo test -p aidb --test assistant -- --ignored`

#![allow(
    clippy::print_stderr,
    reason = "the outcome is printed so a failing model run can be read"
)]

use std::time::Duration;

use aidb_lib::adapters::ai_providers::anthropic::{AnthropicProvider, DEFAULT_BASE_URL};
use aidb_lib::adapters::ai_providers::openai_compatible::{
    OpenAiCompatibleOptions, OpenAiCompatibleProvider,
};
use aidb_lib::adapters::ai_providers::{ProviderClient, ProviderSettings};
use aidb_lib::adapters::credentials::Credentials;
use aidb_lib::adapters::database::{ConnectTarget, DatabaseAdapter, DatabaseSession};
use aidb_lib::adapters::local_store::LocalStore;
use aidb_lib::adapters::postgres::{PgSession, PostgresAdapter};
use aidb_lib::domain::agent::context::{self, ConnectionContext, ContextRequest};
use aidb_lib::domain::agent::plan::{PlanTarget, StepStatus, TaskId, TaskPlan};
use aidb_lib::domain::agent::task::{TaskEvent, TaskOutcome, TaskStatus};
use aidb_lib::domain::agent::tools::ToolName;
use aidb_lib::domain::connection_uri;
use aidb_lib::domain::connections::{ConnectionDraft, Environment, SslMode};
use aidb_lib::domain::database::{OperationId, SessionLimits};
use aidb_lib::services::assistant::{self, RunningTasks, SessionTools, TaskInput};
use aidb_lib::services::catalog::{self, CatalogCache};
use aidb_lib::services::sessions::{self, Sessions};
use secrecy::SecretString;
use url::Url;

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

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn openai_compatible() -> (ProviderClient, String) {
    let url = env("AIDB_TEST_OPENAI_URL").expect("set AIDB_TEST_OPENAI_URL");
    let model = env("AIDB_TEST_OPENAI_MODEL").expect("set AIDB_TEST_OPENAI_MODEL");
    let provider = OpenAiCompatibleProvider::new(
        ProviderSettings {
            base_url: Url::parse(&url).expect("AIDB_TEST_OPENAI_URL is a URL"),
            api_key: env("AIDB_TEST_OPENAI_KEY").map(SecretString::from),
            idle_timeout: Duration::from_secs(120),
        },
        OpenAiCompatibleOptions::default(),
    )
    .expect("provider");
    (ProviderClient::OpenAiCompatible(provider), model)
}

fn anthropic() -> (ProviderClient, String) {
    let key = env("AIDB_TEST_ANTHROPIC_KEY").expect("set AIDB_TEST_ANTHROPIC_KEY");
    let url = env("AIDB_TEST_ANTHROPIC_URL").unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());
    let provider = AnthropicProvider::new(ProviderSettings {
        base_url: Url::parse(&url).expect("AIDB_TEST_ANTHROPIC_URL is a URL"),
        api_key: Some(SecretString::from(key)),
        idle_timeout: Duration::from_secs(120),
    })
    .expect("provider");
    let model = env("AIDB_TEST_ANTHROPIC_MODEL").unwrap_or_else(|| "claude-opus-5-5".to_owned());
    (ProviderClient::Anthropic(provider), model)
}

/// The fixture's settings and password.
fn fixture() -> (ConnectionDraft, SecretString) {
    let url = env("AIDB_TEST_POSTGRES_URL")
        .expect("AIDB_TEST_POSTGRES_URL is set by target/postgres-fixture/env");
    let parsed = connection_uri::parse(&url).expect("the fixture URL parses");
    let mut settings = ConnectionDraft::new(
        "fixture",
        parsed.host,
        parsed.database.expect("the fixture URL names a database"),
        parsed.username.expect("the fixture URL names a user"),
    );
    settings.port = parsed.port;
    settings.ssl_mode = SslMode::Disable;
    (settings, parsed.password.expect("a password").0)
}

async fn admin_session(settings: ConnectionDraft, password: SecretString) -> PgSession {
    PostgresAdapter
        .connect(ConnectTarget {
            settings,
            password: Some(password),
            limits: SessionLimits {
                connect_timeout: Duration::from_secs(5),
                statement_timeout: Duration::from_secs(30),
                lock_timeout: Duration::from_secs(5),
                read_only: false,
            },
        })
        .await
        .expect("connect")
}

/// Fixed test statements and generated names only.
async fn execute(session: &PgSession, sql: &str) {
    let mut connection = session
        .begin_operation(OperationId::new())
        .await
        .expect("operation");
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_owned()))
        .execute(&mut *connection)
        .await
        .expect("statement");
}

/// A database of its own with [`SCHEMA`]; dropped by [`drop_database`].
async fn scratch_database() -> String {
    let (settings, password) = fixture();
    let name = format!(
        "assistant_{}",
        OperationId::new().to_string().replace('-', "")
    );
    let admin = admin_session(settings.clone(), password.clone()).await;
    execute(
        &admin,
        &format!("CREATE DATABASE {name} TEMPLATE template0 ENCODING 'UTF8'"),
    )
    .await;
    admin.close().await;
    let mut scratch = settings;
    scratch.database = name.clone();
    let session = admin_session(scratch, password).await;
    execute(&session, SCHEMA).await;
    session.close().await;
    name
}

async fn drop_database(name: &str) {
    let (settings, password) = fixture();
    let admin = admin_session(settings, password).await;
    execute(&admin, &format!("DROP DATABASE {name} WITH (FORCE)")).await;
    admin.close().await;
}

/// Saves a read-only profile for `database`, connects, builds the context the way the app
/// does, and runs the task.
async fn ask(
    (provider, model): (ProviderClient, String),
    database: &str,
    question: &str,
    data_allowed: bool,
) -> (TaskOutcome, Vec<TaskEvent>, LocalStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = LocalStore::open(dir.path()).await.expect("store");
    let credentials = Credentials::session_only("assistant tests");
    let (mut settings, password) = fixture();
    settings.database = database.to_owned();
    let profile = store
        .connections()
        .insert(&settings.validated().expect("valid settings"))
        .await
        .expect("profile");
    let sessions = Sessions::new(PostgresAdapter);
    let cache = CatalogCache::default();
    let active = sessions::connect(&store, &credentials, &sessions, profile.id, Some(password))
        .await
        .expect("connect");
    assert!(active.server.read_only, "new connections are read-only");

    let snapshot = catalog::overview(&store, &sessions, &cache, profile.id, false)
        .await
        .expect("catalog");
    let connection = ConnectionContext {
        database: database.to_owned(),
        environment: Environment::Development,
        server_version: active.server.version.clone(),
        read_only: true,
    };
    let built = context::build(&ContextRequest {
        question,
        connection: &connection,
        catalog: Some(&snapshot.overview),
        selected: &[],
        attachments: &[],
        budget_tokens: context::DEFAULT_CONTEXT_TOKENS,
    })
    .expect("context");
    if env("AIDB_TEST_SHOW_CONTEXT").is_some() {
        eprintln!("SYSTEM:\n{}\n\nMESSAGE:\n{}", built.system, built.message);
    }
    let plan = TaskPlan::new(
        PlanTarget {
            connection_id: profile.id,
            database: database.to_owned(),
            environment: Environment::Development,
            read_only: true,
        },
        question,
    )
    .expect("plan");
    let running = RunningTasks::default();
    let cancel = running.start(TaskId::new());
    let tools = SessionTools {
        store: &store,
        sessions: &sessions,
        cache: &cache,
        connection_id: profile.id,
    };
    let mut events = Vec::new();
    let outcome = assistant::run(
        &store,
        &provider,
        &tools,
        TaskInput {
            context: built,
            plan,
            history: Vec::new(),
            model,
            data_allowed,
        },
        &cancel,
        &mut |event| events.push(event),
    )
    .await;
    sessions.close_all().await;
    (outcome, events, store, dir)
}

fn ran(outcome: &TaskOutcome, tool: ToolName) -> bool {
    outcome
        .plan
        .steps
        .iter()
        .any(|step| step.tool == tool && step.status == StepStatus::Succeeded)
}

async fn answers_from_the_database(endpoint: (ProviderClient, String)) {
    let database = scratch_database().await;
    let (outcome, events, store, _dir) = ask(
        endpoint,
        &database,
        "How many orders have the status 'shipped'?",
        true,
    )
    .await;
    drop_database(&database).await;

    eprintln!("{outcome:#?}");
    assert!(
        matches!(outcome.status, TaskStatus::Succeeded | TaskStatus::Partial),
        "{outcome:?}"
    );
    assert!(ran(&outcome, ToolName::RunReadQuery), "{outcome:?}");
    assert!(outcome.answer.contains('7'), "{}", outcome.answer);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, TaskEvent::StepFinished { .. }))
    );
    let stored = store
        .agent_tasks()
        .get(outcome.task_id)
        .await
        .expect("read")
        .expect("saved");
    assert_eq!(stored.summary.status, outcome.status);
}

async fn holds_back_rows_without_consent(endpoint: (ProviderClient, String)) {
    let database = scratch_database().await;
    let (outcome, _, _, _dir) = ask(
        endpoint,
        &database,
        "List the names of all customers.",
        false,
    )
    .await;
    drop_database(&database).await;

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
