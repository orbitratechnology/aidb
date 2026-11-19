//! Shared by the assistant's integration tests: model endpoints from the environment, scratch
//! databases on the PostgreSQL fixture, and a workspace that runs tasks through the app's own
//! services (read-only session, catalog, query path, local store).

#![allow(
    dead_code,
    reason = "each test file uses part of this module; Rust warns about the rest per file"
)]

use std::time::Duration;

use aidb_lib::adapters::ai_providers::anthropic::{AnthropicProvider, DEFAULT_BASE_URL};
use aidb_lib::adapters::ai_providers::openai_compatible::{
    OpenAiCompatibleOptions, OpenAiCompatibleProvider,
};
use aidb_lib::adapters::ai_providers::{AiProvider, ProviderClient, ProviderSettings};
use aidb_lib::adapters::credentials::Credentials;
use aidb_lib::adapters::database::{ConnectTarget, DatabaseAdapter, DatabaseSession};
use aidb_lib::adapters::local_store::LocalStore;
use aidb_lib::adapters::postgres::{PgSession, PostgresAdapter};
use aidb_lib::domain::agent::context::{self, Attachment, ConnectionContext, ContextRequest};
use aidb_lib::domain::agent::plan::{PlanTarget, TaskId, TaskPlan};
use aidb_lib::domain::agent::task::{TaskEvent, TaskOutcome};
use aidb_lib::domain::connection_uri;
use aidb_lib::domain::connections::{ConnectionDraft, ConnectionId, Environment, SslMode};
use aidb_lib::domain::database::{OperationId, SessionLimits};
use aidb_lib::services::assistant::{self, RunningTasks, SessionTools, TaskInput};
use aidb_lib::services::catalog::{self, CatalogCache};
use aidb_lib::services::sessions::{self, Sessions};
use secrecy::{ExposeSecret, SecretString};
use url::Url;

pub fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// An OpenAI-compatible endpoint from `AIDB_TEST_OPENAI_*`, and its model.
pub fn openai_compatible() -> (ProviderClient, String) {
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

/// The Anthropic API with `AIDB_TEST_ANTHROPIC_KEY`, and the model to use.
pub fn anthropic() -> (ProviderClient, String) {
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
pub fn fixture() -> (ConnectionDraft, SecretString) {
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

/// The fixture's password, to check that it never reaches a model.
pub fn fixture_password() -> String {
    fixture().1.expose_secret().to_owned()
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

/// Everything in the `public` schema, as text: each relation with its kind and a digest of its
/// rows. Equal snapshots mean nothing was created, dropped or changed.
const SNAPSHOT_SQL: &str = "
SELECT c.relname || ':' || c.relkind::text || ':' || CASE WHEN c.relkind = 'r' THEN
    (xpath('/row/d/text()', query_to_xml(format(
        'SELECT md5(coalesce(string_agg(t::text, %L ORDER BY t::text), %L)) AS d FROM public.%I t',
        ',', '', c.relname), false, true, '')))[1]::text
  ELSE coalesce(md5(pg_get_viewdef(c.oid)), '') END
FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE n.nspname = 'public' AND c.relkind IN ('r', 'v', 'm', 'p', 'f')
ORDER BY c.relname";

/// A database of its own on the fixture, dropped by [`ScratchDatabase::drop`].
pub struct ScratchDatabase {
    pub name: String,
}

impl ScratchDatabase {
    /// Creates `<prefix>_<random>` and runs `schema` in it.
    pub async fn create(prefix: &str, schema: &str) -> Self {
        let (settings, password) = fixture();
        let name = format!(
            "{prefix}_{}",
            OperationId::new().to_string().replace('-', "")
        );
        let admin = admin_session(settings.clone(), password.clone()).await;
        execute(
            &admin,
            &format!("CREATE DATABASE {name} TEMPLATE template0 ENCODING 'UTF8'"),
        )
        .await;
        admin.close().await;
        let session = admin_session(Self::settings(&name), password).await;
        execute(&session, schema).await;
        session.close().await;
        Self { name }
    }

    fn settings(name: &str) -> ConnectionDraft {
        let mut settings = fixture().0;
        name.clone_into(&mut settings.database);
        settings
    }

    /// See [`SNAPSHOT_SQL`].
    pub async fn snapshot(&self) -> Vec<String> {
        let session = admin_session(Self::settings(&self.name), fixture().1).await;
        let rows = {
            let mut connection = session
                .begin_operation(OperationId::new())
                .await
                .expect("operation");
            sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(SNAPSHOT_SQL.to_owned()))
                .fetch_all(&mut *connection)
                .await
                .expect("snapshot")
        };
        session.close().await;
        rows
    }

    pub async fn drop(self) {
        let (settings, password) = fixture();
        let admin = admin_session(settings, password).await;
        execute(&admin, &format!("DROP DATABASE {} WITH (FORCE)", self.name)).await;
        admin.close().await;
    }
}

/// What one task produced.
pub struct Ran {
    pub outcome: TaskOutcome,
    pub events: Vec<TaskEvent>,
    /// The request's tag for untrusted content.
    pub tag: String,
}

/// A saved, read-only connection to a scratch database, open in the app's services.
pub struct Workspace {
    _dir: tempfile::TempDir,
    pub store: LocalStore,
    pub sessions: Sessions<PostgresAdapter>,
    pub cache: CatalogCache,
    pub connection_id: ConnectionId,
    pub database: String,
    server_version: String,
}

impl Workspace {
    pub async fn connect(database: &str) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = LocalStore::open(dir.path()).await.expect("store");
        let credentials = Credentials::session_only("assistant tests");
        let settings = ScratchDatabase::settings(database);
        let profile = store
            .connections()
            .insert(&settings.validated().expect("valid settings"))
            .await
            .expect("profile");
        let sessions = Sessions::new(PostgresAdapter);
        let active = sessions::connect(
            &store,
            &credentials,
            &sessions,
            profile.id,
            Some(fixture().1),
        )
        .await
        .expect("connect");
        assert!(active.server.read_only, "new connections are read-only");
        Self {
            _dir: dir,
            store,
            sessions,
            cache: CatalogCache::default(),
            connection_id: profile.id,
            database: database.to_owned(),
            server_version: active.server.version,
        }
    }

    /// The task's input, with the context built the way the app builds it.
    pub async fn input(
        &self,
        question: &str,
        data_allowed: bool,
        attachments: &[Attachment],
        model: &str,
    ) -> TaskInput {
        let snapshot = catalog::overview(
            &self.store,
            &self.sessions,
            &self.cache,
            self.connection_id,
            false,
        )
        .await
        .expect("catalog");
        let connection = ConnectionContext {
            database: self.database.clone(),
            environment: Environment::Development,
            server_version: self.server_version.clone(),
            read_only: true,
        };
        let built = context::build(&ContextRequest {
            question,
            connection: &connection,
            catalog: Some(&snapshot.overview),
            selected: &[],
            attachments,
            budget_tokens: context::DEFAULT_CONTEXT_TOKENS,
        })
        .expect("context");
        if env("AIDB_TEST_SHOW_CONTEXT").is_some() {
            #[allow(clippy::print_stderr, reason = "asked for with AIDB_TEST_SHOW_CONTEXT")]
            {
                eprintln!("SYSTEM:\n{}\n\nMESSAGE:\n{}", built.system, built.message);
            }
        }
        let plan = TaskPlan::new(
            PlanTarget {
                connection_id: self.connection_id,
                database: self.database.clone(),
                environment: Environment::Development,
                read_only: true,
            },
            question,
        )
        .expect("plan");
        TaskInput {
            context: built,
            plan,
            history: Vec::new(),
            model: model.to_owned(),
            data_allowed,
        }
    }

    /// Runs a task to its end with the real tools.
    pub async fn run<P: AiProvider>(&self, provider: &P, input: TaskInput) -> Ran {
        let tag = input.context.tag.clone();
        let running = RunningTasks::default();
        let cancel = running.start(TaskId::new());
        let tools = SessionTools {
            store: &self.store,
            sessions: &self.sessions,
            cache: &self.cache,
            connection_id: self.connection_id,
        };
        let mut events = Vec::new();
        let outcome = assistant::run(
            &self.store,
            provider,
            &tools,
            input,
            &cancel,
            &mut |event| events.push(event),
        )
        .await;
        Ran {
            outcome,
            events,
            tag,
        }
    }

    pub async fn close(self) {
        self.sessions.close_all().await;
    }
}
