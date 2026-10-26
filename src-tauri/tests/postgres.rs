//! Integration tests against a real PostgreSQL server.
//!
//! ```text
//! scripts/postgres-fixture.sh start 17
//! source target/postgres-fixture/env
//! cargo test -p aidb --test postgres -- --ignored
//! ```
//!
//! Every test is ignored by default and panics if the fixture's variables are missing, so CI
//! cannot pass them by accident. The `contract` module holds the checks any future database
//! adapter must also pass; the rest covers PostgreSQL behavior.

use std::time::{Duration, Instant};

use aidb_lib::adapters::database::{
    ConnectTarget, DatabaseAdapter, DatabaseSession, DbError, DbErrorKind,
};
use aidb_lib::adapters::postgres::{PgSession, PostgresAdapter};
use aidb_lib::domain::connection_uri;
use aidb_lib::domain::connections::{ConnectionDraft, SslMode};
use aidb_lib::domain::database::{OperationId, SessionLimits};
use secrecy::{ExposeSecret, SecretString};

const NEEDS_FIXTURE: &str = "needs PostgreSQL: run scripts/postgres-fixture.sh start";

struct Fixture {
    settings: ConnectionDraft,
    password: SecretString,
    ca_path: String,
}

fn fixture() -> Fixture {
    let url = std::env::var("AIDB_TEST_POSTGRES_URL")
        .expect("AIDB_TEST_POSTGRES_URL is set by target/postgres-fixture/env");
    let ca_path = std::env::var("AIDB_TEST_POSTGRES_CA")
        .expect("AIDB_TEST_POSTGRES_CA is set by target/postgres-fixture/env");
    let parsed = connection_uri::parse(&url).expect("the fixture URL parses");
    let mut settings = ConnectionDraft::new(
        "fixture",
        parsed.host,
        parsed.database.expect("the fixture URL names a database"),
        parsed.username.expect("the fixture URL names a user"),
    );
    settings.port = parsed.port;
    settings.ssl_mode = SslMode::Disable;
    Fixture {
        settings,
        password: parsed.password.expect("the fixture URL has a password").0,
        ca_path,
    }
}

fn limits(read_only: bool) -> SessionLimits {
    SessionLimits {
        connect_timeout: Duration::from_secs(5),
        statement_timeout: Duration::from_secs(30),
        lock_timeout: Duration::from_secs(5),
        read_only,
    }
}

fn target(fixture: &Fixture, read_only: bool) -> ConnectTarget {
    ConnectTarget {
        settings: fixture.settings.clone(),
        password: Some(fixture.password.clone()),
        limits: limits(read_only),
    }
}

async fn open(target: ConnectTarget) -> PgSession {
    PostgresAdapter
        .connect(target)
        .await
        .unwrap_or_else(|error| panic!("connect failed: {error}"))
}

async fn connect_error(target: ConnectTarget) -> DbError {
    match PostgresAdapter.connect(target).await {
        Ok(_) => panic!("the connection should have failed"),
        Err(error) => error,
    }
}

/// Test statements are fixed text or use generated names, never outside input.
fn statement(sql: impl Into<String>) -> sqlx::AssertSqlSafe<String> {
    sqlx::AssertSqlSafe(sql.into())
}

/// Runs one statement as a cancellable operation and classifies its error.
async fn run(session: &PgSession, sql: &str) -> Result<(), DbError> {
    let mut connection = session.begin_operation(OperationId::new()).await?;
    sqlx::raw_sql(statement(sql))
        .execute(&mut *connection)
        .await
        .map(drop)
        .map_err(|error| session.session_error(error))
}

/// Checks every adapter must pass, whatever its engine.
mod contract {
    use super::*;

    pub async fn connects_and_reports_the_server<A: DatabaseAdapter>(
        adapter: &A,
        target: ConnectTarget,
    ) {
        let expected_user = target.settings.username.clone();
        let expected_database = target.settings.database.clone();
        let password = target.password.clone();

        let session = adapter.connect(target).await.expect("connects");
        let info = session.server_info();

        assert_eq!(info.engine, adapter.capabilities().engine);
        assert_eq!(info.user, expected_user);
        assert_eq!(info.database, expected_database);
        assert!(info.read_only, "the session is read-only by default");
        assert!(
            info.supported_version,
            "version {} is outside the tested range",
            info.version
        );
        session.ping().await.expect("ping");
        if let Some(password) = password {
            assert!(!format!("{session:?}").contains(password.expose_secret()));
        }
        session.close().await;
    }

    pub async fn rejects_a_wrong_password<A: DatabaseAdapter>(
        adapter: &A,
        mut target: ConnectTarget,
    ) {
        target.password = Some(SecretString::from("not-the-password".to_owned()));

        let error = adapter.connect(target).await.expect_err("rejected");

        assert_eq!(error.kind(), DbErrorKind::Authentication);
        assert!(!error.message().contains("not-the-password"));
    }

    pub async fn reports_a_missing_database<A: DatabaseAdapter>(
        adapter: &A,
        mut target: ConnectTarget,
    ) {
        target.settings.database = "aidb_no_such_database".into();

        let error = adapter.connect(target).await.expect_err("rejected");

        assert_eq!(error.kind(), DbErrorKind::DatabaseNotFound);
        assert!(error.message().contains("aidb_no_such_database"));
    }

    pub async fn cancelling_an_unknown_operation_is_a_no_op<A: DatabaseAdapter>(
        adapter: &A,
        target: ConnectTarget,
    ) {
        let session = adapter.connect(target).await.expect("connects");

        assert!(!session.cancel(OperationId::new()).await.expect("cancel"));
        session.close().await;
    }
}

#[tokio::test]
#[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
async fn contract_connects_and_reports_the_server() {
    contract::connects_and_reports_the_server(&PostgresAdapter, target(&fixture(), true)).await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
async fn contract_rejects_a_wrong_password() {
    contract::rejects_a_wrong_password(&PostgresAdapter, target(&fixture(), true)).await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
async fn contract_reports_a_missing_database() {
    contract::reports_a_missing_database(&PostgresAdapter, target(&fixture(), true)).await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
async fn contract_cancelling_an_unknown_operation_is_a_no_op() {
    contract::cancelling_an_unknown_operation_is_a_no_op(
        &PostgresAdapter,
        target(&fixture(), true),
    )
    .await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
async fn profiles_without_a_password_never_use_pgpassword() {
    // The fixture exports the right password as PGPASSWORD; the driver must not pick it up.
    assert!(std::env::var("PGPASSWORD").is_ok(), "{NEEDS_FIXTURE}");
    let mut target = target(&fixture(), true);
    target.password = None;

    let error = connect_error(target).await;

    assert_eq!(error.kind(), DbErrorKind::Authentication);
}

#[tokio::test]
#[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
async fn read_only_sessions_reject_writes_even_with_pgoptions() {
    // The fixture exports PGOPTIONS turning read-only off; aidb's setting must win.
    assert!(std::env::var("PGOPTIONS").is_ok(), "{NEEDS_FIXTURE}");
    let fixture = fixture();
    let read_only = open(target(&fixture, true)).await;
    let writable = open(target(&fixture, false)).await;

    let refused = run(&read_only, "CREATE TEMP TABLE aidb_probe (x int)")
        .await
        .expect_err("read-only");
    let allowed = run(
        &writable,
        "CREATE TEMP TABLE aidb_probe (x int); INSERT INTO aidb_probe VALUES (1)",
    )
    .await;

    assert_eq!(refused.kind(), DbErrorKind::ReadOnly);
    assert_eq!(refused.sqlstate(), Some("25006"));
    assert!(allowed.is_ok(), "{allowed:?}");
    assert!(!writable.server_info().read_only);
}

#[tokio::test]
#[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
async fn closed_ports_are_unreachable() {
    let mut target = target(&fixture(), true);
    target.settings.port = 1;
    let started = Instant::now();

    let error = connect_error(target).await;

    assert_eq!(error.kind(), DbErrorKind::Unreachable);
    assert!(error.message().contains(":1"));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
#[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
async fn silent_servers_hit_the_connect_timeout() {
    // Accepts the TCP connection but never answers, like a server behind a dropping firewall.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let mut target = target(&fixture(), true);
    target.settings.host = "127.0.0.1".into();
    target.settings.port = listener.local_addr().expect("address").port();
    target.limits.connect_timeout = Duration::from_secs(1);
    let started = Instant::now();

    let error = connect_error(target).await;

    assert_eq!(error.kind(), DbErrorKind::ConnectTimeout);
    assert!(started.elapsed() < Duration::from_secs(3));
    drop(listener);
}

#[tokio::test]
#[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
async fn statement_timeouts_stop_long_statements() {
    let mut target = target(&fixture(), true);
    target.limits.statement_timeout = Duration::from_secs(1);
    let session = open(target).await;
    let started = Instant::now();

    let error = run(&session, "SELECT pg_sleep(10)")
        .await
        .expect_err("timed out");

    assert_eq!(error.kind(), DbErrorKind::StatementTimeout);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
#[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
async fn lock_timeouts_stop_waiting_statements() {
    let fixture = fixture();
    let table = format!(
        "aidb_lock_probe_{}",
        OperationId::new().to_string().replace('-', "")
    );
    let holder = open(target(&fixture, false)).await;
    run(&holder, &format!("CREATE TABLE {table} (x int)"))
        .await
        .expect("create table");

    let mut lock = holder
        .begin_operation(OperationId::new())
        .await
        .expect("acquire");
    sqlx::raw_sql(statement(format!(
        "BEGIN; LOCK TABLE {table} IN ACCESS EXCLUSIVE MODE"
    )))
    .execute(&mut *lock)
    .await
    .expect("lock");

    let mut waiting_target = target(&fixture, true);
    waiting_target.limits.lock_timeout = Duration::from_secs(1);
    let waiter = open(waiting_target).await;
    let error = run(&waiter, &format!("SELECT count(*) FROM {table}"))
        .await
        .expect_err("lock timeout");

    sqlx::raw_sql("ROLLBACK")
        .execute(&mut *lock)
        .await
        .expect("rollback");
    drop(lock);
    run(&holder, &format!("DROP TABLE {table}"))
        .await
        .expect("drop table");
    assert_eq!(error.kind(), DbErrorKind::LockTimeout);
}

#[tokio::test]
#[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
async fn running_statements_can_be_cancelled() {
    let session = std::sync::Arc::new(open(target(&fixture(), true)).await);
    let operation = OperationId::new();
    let mut connection = session.begin_operation(operation).await.expect("acquire");
    let started = Instant::now();
    let running = tokio::spawn(async move {
        let result = sqlx::query("SELECT pg_sleep(20)")
            .execute(&mut *connection)
            .await;
        drop(connection);
        result
    });

    tokio::time::sleep(Duration::from_millis(300)).await;
    let signalled = session.cancel(operation).await.expect("cancel");
    let error = running
        .await
        .expect("task")
        .map(drop)
        .map_err(|error| session.session_error(error))
        .expect_err("cancelled");

    assert!(signalled);
    assert_eq!(error.kind(), DbErrorKind::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(
        !session.cancel(operation).await.expect("cancel again"),
        "already finished"
    );
    session.ping().await.expect("the session still works");
}

mod tls {
    use super::*;

    fn tls_target(mode: SslMode, host: &str, with_ca: bool) -> ConnectTarget {
        let fixture = fixture();
        let mut target = target(&fixture, true);
        target.settings.ssl_mode = mode;
        target.settings.host = host.into();
        target.settings.ca_cert_path = with_ca.then(|| fixture.ca_path.clone());
        target
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn verify_full_with_the_ca_connects_encrypted() {
        let session = open(tls_target(SslMode::VerifyFull, "localhost", true)).await;

        assert!(session.server_info().encrypted);
        assert!(session.server_info().tls_version.is_some());
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn verify_full_rejects_an_untrusted_certificate() {
        let error = connect_error(tls_target(SslMode::VerifyFull, "localhost", false)).await;

        assert_eq!(error.kind(), DbErrorKind::Tls);
        assert!(error.message().contains("trusted authority"), "{error}");
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn verify_full_checks_the_host_name() {
        // The certificate names only `localhost`.
        let error = connect_error(tls_target(SslMode::VerifyFull, "127.0.0.1", true)).await;

        assert_eq!(error.kind(), DbErrorKind::Tls);
        assert!(
            error.message().contains("not valid for 127.0.0.1"),
            "{error}"
        );
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn verify_ca_trusts_the_ca_without_checking_the_name() {
        let session = open(tls_target(SslMode::VerifyCa, "127.0.0.1", true)).await;

        assert!(session.server_info().encrypted);
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn require_encrypts_without_verifying() {
        let session = open(tls_target(SslMode::Require, "127.0.0.1", false)).await;

        assert!(session.server_info().encrypted);
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn disable_connects_in_plaintext() {
        let session = open(tls_target(SslMode::Disable, "localhost", false)).await;

        assert!(!session.server_info().encrypted);
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn a_missing_ca_file_is_reported() {
        let mut target = tls_target(SslMode::VerifyFull, "localhost", false);
        target.settings.ca_cert_path = Some("/nonexistent/aidb-ca.pem".into());

        let error = connect_error(target).await;

        assert_eq!(error.kind(), DbErrorKind::Tls);
    }
}
