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

/// Creates a UTF8 database named `<prefix>_<random>`, runs `schema` in it and returns a writable
/// session on it. Tests that read whole-database state get a database of their own.
async fn scratch_database(prefix: &str, schema: &str) -> (String, PgSession) {
    scratch_database_with(prefix, "ENCODING 'UTF8'", schema).await
}

/// [`scratch_database`] with other `CREATE DATABASE` options, such as an encoding.
async fn scratch_database_with(prefix: &str, options: &str, schema: &str) -> (String, PgSession) {
    let fixture = fixture();
    let admin = open(target(&fixture, false)).await;
    let name = format!(
        "{prefix}_{}",
        OperationId::new().to_string().replace('-', "")
    );
    run(
        &admin,
        &format!("CREATE DATABASE {name} TEMPLATE template0 {options}"),
    )
    .await
    .expect("create database");
    admin.close().await;
    let mut scratch = target(&fixture, false);
    scratch.settings.database = name.clone();
    let session = open(scratch).await;
    run(&session, schema).await.expect("schema");
    (name, session)
}

async fn drop_scratch(name: &str, session: PgSession) {
    session.close().await;
    let admin = open(target(&fixture(), false)).await;
    run(&admin, &format!("DROP DATABASE {name} WITH (FORCE)"))
        .await
        .expect("drop database");
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

/// Catalog introspection, each test in its own scratch database so the whole-database
/// fingerprint is not disturbed by other tests.
mod catalog {
    use aidb_lib::domain::catalog::{Identity, ObjectKind, ObjectName, ReferentialAction};

    use super::*;

    const SCHEMA_SQL: &str = r"
        CREATE SCHEMA shop;
        COMMENT ON SCHEMA shop IS 'Storefront';
        CREATE TABLE shop.customers (
            id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
            email text NOT NULL UNIQUE,
            name varchar(80) DEFAULT 'anonymous'
        );
        COMMENT ON TABLE shop.customers IS 'People who order';
        COMMENT ON COLUMN shop.customers.email IS 'Login address';
        CREATE TABLE shop.orders (
            id bigserial PRIMARY KEY,
            customer_id bigint NOT NULL REFERENCES shop.customers (id) ON DELETE CASCADE,
            qty integer NOT NULL,
            price numeric(10, 2) NOT NULL,
            subtotal numeric GENERATED ALWAYS AS (qty * price) STORED,
            created_at timestamptz NOT NULL DEFAULT now()
        );
        CREATE INDEX orders_created_at_idx ON shop.orders (created_at DESC);
        CREATE VIEW shop.recent_orders AS
            SELECT id, customer_id FROM shop.orders WHERE created_at > now() - interval '1 day';
        CREATE MATERIALIZED VIEW shop.order_totals AS
            SELECT customer_id, sum(subtotal) AS total FROM shop.orders GROUP BY customer_id;
        CREATE TABLE shop.events (at date NOT NULL, kind text) PARTITION BY RANGE (at);
        CREATE TABLE shop.events_2026 PARTITION OF shop.events
            FOR VALUES FROM ('2026-01-01') TO ('2027-01-01');
        INSERT INTO shop.customers (email) VALUES ('a@example.com'), ('b@example.com');
        ANALYZE shop.customers;
    ";

    /// Creates a database with the shop schema; returns a writable session on it.
    async fn scratch() -> (String, PgSession) {
        scratch_database("aidb_catalog", SCHEMA_SQL).await
    }

    fn object(name: &str) -> ObjectName {
        ObjectName {
            schema: "shop".into(),
            name: name.into(),
        }
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn overview_lists_schemas_tables_and_views() {
        let (name, session) = scratch().await;

        let overview = session.catalog_overview().await.expect("overview");

        assert_eq!(overview.database, name);
        let shop = overview
            .schemas
            .iter()
            .find(|s| s.name == "shop")
            .expect("schema");
        assert_eq!(shop.comment.as_deref(), Some("Storefront"));
        let kinds: Vec<(&str, ObjectKind)> = overview
            .objects
            .iter()
            .filter(|o| o.schema == "shop")
            .map(|o| (o.name.as_str(), o.kind))
            .collect();
        assert_eq!(
            kinds,
            [
                ("customers", ObjectKind::Table),
                ("events", ObjectKind::PartitionedTable),
                ("order_totals", ObjectKind::MaterializedView),
                ("orders", ObjectKind::Table),
                ("recent_orders", ObjectKind::View),
            ],
            "partitions are listed under their parent only"
        );
        let customers = overview
            .objects
            .iter()
            .find(|o| o.name == "customers")
            .expect("listed");
        assert_eq!(customers.row_estimate, Some(2));
        assert_eq!(customers.comment.as_deref(), Some("People who order"));
        assert!(customers.readable);
        assert!(!overview.truncated);
        drop_scratch(&name, session).await;
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn tables_describe_columns_keys_references_and_indexes() {
        let (name, session) = scratch().await;

        let customers = session
            .describe_object(&object("customers"))
            .await
            .expect("customers");
        let orders = session
            .describe_object(&object("orders"))
            .await
            .expect("orders");

        let id = &customers.columns[0];
        assert_eq!((id.name.as_str(), id.data_type.as_str()), ("id", "bigint"));
        assert_eq!(id.identity, Some(Identity::Always));
        assert!(!id.nullable);
        let customer_name = &customers.columns[2];
        assert_eq!(customer_name.data_type, "character varying(80)");
        assert_eq!(
            customer_name.default.as_deref(),
            Some("'anonymous'::character varying")
        );
        assert_eq!(
            customers.columns[1].comment.as_deref(),
            Some("Login address")
        );
        assert_eq!(
            customers.primary_key.as_ref().map(|k| k.columns.clone()),
            Some(vec!["id".to_owned()])
        );
        assert_eq!(customers.unique_constraints[0].columns, ["email"]);
        let incoming = &customers.referenced_by[0];
        assert_eq!(
            (incoming.from.name.as_str(), incoming.columns.as_slice()),
            ("orders", &["customer_id".to_owned()][..])
        );

        let fk = &orders.foreign_keys[0];
        assert_eq!(fk.to, object("customers"));
        assert_eq!(fk.referenced_columns, ["id"]);
        assert_eq!(fk.on_delete, ReferentialAction::Cascade);
        assert_eq!(fk.on_update, ReferentialAction::NoAction);
        let subtotal = orders
            .columns
            .iter()
            .find(|c| c.name == "subtotal")
            .expect("column");
        assert_eq!(
            subtotal.generated.as_deref(),
            Some("((qty)::numeric * price)")
        );
        assert_eq!(subtotal.default, None);
        let index = orders
            .indexes
            .iter()
            .find(|i| i.name == "orders_created_at_idx")
            .expect("index");
        assert!(index.definition.contains("(created_at DESC)"));
        assert_eq!(index.method, "btree");
        assert!(orders.indexes.iter().any(|i| i.primary));
        assert_eq!(orders.definition, None);
        drop_scratch(&name, session).await;
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn views_include_their_query_and_missing_objects_are_reported() {
        let (name, session) = scratch().await;

        let view = session
            .describe_object(&object("recent_orders"))
            .await
            .expect("view");
        let missing = session
            .describe_object(&object("ghost"))
            .await
            .expect_err("missing");

        assert_eq!(view.summary.kind, ObjectKind::View);
        assert!(
            view.definition
                .as_deref()
                .is_some_and(|d| d.contains("FROM shop.orders"))
        );
        assert_eq!(view.columns.len(), 2);
        assert_eq!(missing.kind(), DbErrorKind::ObjectNotFound);
        drop_scratch(&name, session).await;
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn the_fingerprint_follows_structure_not_statistics() {
        let (name, session) = scratch().await;
        let before = session.catalog_fingerprint().await.expect("fingerprint");

        run(
            &session,
            "INSERT INTO shop.customers (email) VALUES ('c@example.com'); ANALYZE",
        )
        .await
        .expect("data and statistics");
        // VACUUM cannot share a batch: a multi-statement batch is one implicit transaction.
        run(&session, "VACUUM shop.customers")
            .await
            .expect("vacuum");
        let after_stats = session.catalog_fingerprint().await.expect("fingerprint");
        run(&session, "ALTER TABLE shop.customers ADD COLUMN phone text")
            .await
            .expect("ddl");
        let after_column = session.catalog_fingerprint().await.expect("fingerprint");
        run(
            &session,
            "COMMENT ON COLUMN shop.customers.phone IS 'Mobile'",
        )
        .await
        .expect("comment");
        let after_comment = session.catalog_fingerprint().await.expect("fingerprint");
        run(
            &session,
            "CREATE OR REPLACE VIEW shop.recent_orders AS SELECT id, customer_id FROM shop.orders",
        )
        .await
        .expect("view");
        let after_view = session.catalog_fingerprint().await.expect("fingerprint");

        assert_eq!(
            before, after_stats,
            "statistics and data do not change the structure"
        );
        assert_ne!(after_stats, after_column);
        assert_ne!(after_column, after_comment);
        assert_ne!(after_comment, after_view);
        let overview = session.catalog_overview().await.expect("overview");
        assert_eq!(overview.fingerprint, after_view);
        drop_scratch(&name, session).await;
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn read_only_sessions_can_read_the_catalog() {
        let (name, writable) = scratch().await;
        let mut read_only = target(&fixture(), true);
        read_only.settings.database = name.clone();
        let session = open(read_only).await;

        let overview = session.catalog_overview().await.expect("overview");
        let details = session
            .describe_object(&object("orders"))
            .await
            .expect("details");

        assert!(overview.objects.iter().any(|o| o.name == "orders"));
        assert!(!details.columns.is_empty());
        session.close().await;
        drop_scratch(&name, writable).await;
    }
}

/// Pages of rows: type mapping, bounds, filters, sorting, privileges and cancellation.
mod table_data {
    use std::sync::Arc;

    use aidb_lib::domain::catalog::ObjectName;
    use aidb_lib::domain::table_data::{
        CELL_CHAR_LIMIT, Filter, FilterOperator, PageLimits, PageRequest, SortDirection, SortKey,
        TablePage, ValueKind,
    };

    use super::*;

    const SCHEMA_SQL: &str = r#"
        CREATE TYPE mood AS ENUM ('ok', 'great');
        CREATE DOMAIN positive AS integer CHECK (VALUE > 0);
        CREATE TABLE samples (
            id integer PRIMARY KEY,
            flag boolean,
            amount numeric,
            doc jsonb,
            raw json,
            tags text[],
            grid integer[][],
            ref uuid,
            at timestamptz,
            day date,
            body text,
            bytes bytea,
            feeling mood,
            qty positive,
            span interval,
            "Weird ""Name""" text
        );
        INSERT INTO samples VALUES (
            1, true, 12345678901234567890.123456789, '{"a": [1, 2]}', '{"b":1}',
            ARRAY['x', 'y,z'], ARRAY[[1, 2], [3, 4]], 'a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11',
            '2026-01-02 03:04:05+00', '2026-01-02', repeat('é', 5000), '\x00ff', 'great', 5,
            '1 day 2 hours', 'odd'
        );
        INSERT INTO samples (id) VALUES (2);
        CREATE TABLE items (id integer PRIMARY KEY, label text NOT NULL);
        INSERT INTO items SELECT g, 'item ' || lpad(g::text, 2, '0') FROM generate_series(1, 25) g;
        INSERT INTO items VALUES (26, '50%_off'), (27, '50x_off');
        CREATE TABLE numbers AS SELECT g AS n FROM generate_series(1, 5) g;
        CREATE TABLE staff (id integer PRIMARY KEY, name text, salary numeric);
        INSERT INTO staff VALUES (1, 'Ada', 100);
        CREATE TABLE counter (n integer);
        CREATE FUNCTION bump() RETURNS integer LANGUAGE sql VOLATILE
            AS 'INSERT INTO counter VALUES (1) RETURNING 1';
        CREATE VIEW sneaky AS SELECT bump() AS b;
        CREATE VIEW slow AS SELECT 1 AS x FROM pg_sleep(20);
    "#;

    const LIMITS: PageLimits = PageLimits {
        max_rows: 500,
        max_bytes: 16 * 1024 * 1024,
    };

    fn request(table: &str) -> PageRequest {
        PageRequest {
            object: ObjectName {
                schema: "public".into(),
                name: table.into(),
            },
            filters: Vec::new(),
            sort: Vec::new(),
            offset: 0,
            limit: None,
            operation_id: None,
        }
    }

    fn filter(column: &str, operator: FilterOperator, value: Option<&str>) -> Filter {
        Filter {
            column: column.into(),
            operator,
            value: value.map(Into::into),
        }
    }

    fn cell<'a>(page: &'a TablePage, row: usize, column: &str) -> Option<&'a str> {
        let index = page
            .columns
            .iter()
            .position(|c| c.name == column)
            .unwrap_or_else(|| panic!("no column {column}"));
        page.rows[row][index].as_deref()
    }

    fn kind(page: &TablePage, column: &str) -> ValueKind {
        page.columns
            .iter()
            .find(|c| c.name == column)
            .map(|c| c.kind)
            .unwrap_or_else(|| panic!("no column {column}"))
    }

    fn ids(page: &TablePage) -> Vec<&str> {
        page.rows
            .iter()
            .map(|row| row[0].as_deref().unwrap_or("NULL"))
            .collect()
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn values_arrive_as_exact_text_with_their_kind() {
        let (name, session) = scratch_database("aidb_pages", SCHEMA_SQL).await;
        let mut read = request("samples");
        read.sort = vec![SortKey {
            column: "id".into(),
            direction: SortDirection::Asc,
        }];

        let page = session.read_page(&read, LIMITS).await.expect("page");

        let expected = [
            ("flag", "true", ValueKind::Boolean),
            (
                "amount",
                "12345678901234567890.123456789",
                ValueKind::Number,
            ),
            ("doc", r#"{"a": [1, 2]}"#, ValueKind::Json),
            ("raw", r#"{"b":1}"#, ValueKind::Json),
            ("tags", r#"["x","y,z"]"#, ValueKind::Array),
            ("grid", "[[1,2],[3,4]]", ValueKind::Array),
            (
                "ref",
                "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11",
                ValueKind::Uuid,
            ),
            ("day", "2026-01-02", ValueKind::DateTime),
            ("bytes", r"\x00ff", ValueKind::Binary),
            ("feeling", "great", ValueKind::Text),
            ("qty", "5", ValueKind::Number),
            ("span", "1 day 02:00:00", ValueKind::Other),
            ("Weird \"Name\"", "odd", ValueKind::Text),
        ];
        for (column, value, value_kind) in expected {
            assert_eq!(cell(&page, 0, column), Some(value), "{column}");
            assert_eq!(kind(&page, column), value_kind, "{column}");
        }
        let at = cell(&page, 0, "at").expect("timestamp");
        assert_eq!(kind(&page, "at"), ValueKind::DateTime);
        assert!(
            at.starts_with("2026-01-0") && at.contains(':'),
            "ISO text: {at}"
        );
        assert!(
            at.rfind(['+', '-']).is_some_and(|i| i > 10),
            "with an offset: {at}"
        );
        assert!(
            page.rows[1][1..].iter().all(Option::is_none),
            "NULLs stay null"
        );
        assert_eq!(
            page.columns
                .iter()
                .find(|c| c.name == "amount")
                .map(|c| c.data_type.as_str()),
            Some("numeric")
        );
        drop_scratch(&name, session).await;
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn long_values_are_cut_and_flagged() {
        let (name, session) = scratch_database("aidb_pages", SCHEMA_SQL).await;
        let mut read = request("samples");
        read.filters = vec![filter("id", FilterOperator::Equals, Some("1"))];

        let page = session.read_page(&read, LIMITS).await.expect("page");

        let body_index = page
            .columns
            .iter()
            .position(|c| c.name == "body")
            .expect("body");
        let body = cell(&page, 0, "body").expect("body");
        assert_eq!(body.chars().count(), CELL_CHAR_LIMIT);
        assert_eq!(page.truncated_cells.len(), 1);
        assert_eq!(
            (
                page.truncated_cells[0].row,
                page.truncated_cells[0].column as usize
            ),
            (0, body_index)
        );
        drop_scratch(&name, session).await;
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn pages_are_bounded_and_ordered_by_the_primary_key() {
        let (name, session) = scratch_database("aidb_pages", SCHEMA_SQL).await;
        let mut read = request("items");
        read.limit = Some(10);

        let first = session.read_page(&read, LIMITS).await.expect("first");
        read.offset = 20;
        let last = session.read_page(&read, LIMITS).await.expect("last");
        read.offset = 0;
        read.limit = Some(1_000);
        let capped = session
            .read_page(
                &read,
                PageLimits {
                    max_rows: 3,
                    ..LIMITS
                },
            )
            .await
            .expect("capped");
        let heap = session
            .read_page(&request("numbers"), LIMITS)
            .await
            .expect("no key");

        assert_eq!(
            ids(&first),
            ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10"]
        );
        assert!(first.has_more && first.stable_order);
        assert_eq!(first.order.len(), 1, "ordered by the primary key");
        assert_eq!(ids(&last), ["21", "22", "23", "24", "25", "26", "27"]);
        assert!(!last.has_more);
        assert_eq!(capped.rows.len(), 3, "the setting caps the request");
        assert!(capped.has_more);
        assert!(!heap.stable_order && heap.order.is_empty());
        assert_eq!(heap.rows.len(), 5);
        drop_scratch(&name, session).await;
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn filters_and_sorting_bind_values_and_match_types() {
        let (name, session) = scratch_database("aidb_pages", SCHEMA_SQL).await;
        let read = |filters: Vec<Filter>, sort: Vec<SortKey>| {
            let mut read = request("items");
            read.filters = filters;
            read.sort = sort;
            read
        };

        let numeric = session
            .read_page(
                &read(
                    vec![filter("id", FilterOperator::GreaterOrEqual, Some("20"))],
                    vec![],
                ),
                LIMITS,
            )
            .await
            .expect("numeric");
        let literal = session
            .read_page(
                &read(
                    vec![filter("label", FilterOperator::Contains, Some("%_"))],
                    vec![],
                ),
                LIMITS,
            )
            .await
            .expect("literal wildcards");
        let prefix = session
            .read_page(
                &read(
                    vec![filter("label", FilterOperator::StartsWith, Some("ITEM 2"))],
                    vec![],
                ),
                LIMITS,
            )
            .await
            .expect("prefix");
        let injection = session
            .read_page(
                &read(
                    vec![filter(
                        "label",
                        FilterOperator::Equals,
                        Some("x' OR '1'='1"),
                    )],
                    vec![],
                ),
                LIMITS,
            )
            .await
            .expect("bound value");
        let sorted = session
            .read_page(
                &read(
                    vec![filter("id", FilterOperator::Less, Some("4"))],
                    vec![SortKey {
                        column: "label".into(),
                        direction: SortDirection::Desc,
                    }],
                ),
                LIMITS,
            )
            .await
            .expect("sorted");
        let nulls = session
            .read_page(
                &{
                    let mut read = request("samples");
                    read.filters = vec![filter("doc", FilterOperator::IsNull, None)];
                    read
                },
                LIMITS,
            )
            .await
            .expect("null check");

        assert_eq!(
            ids(&numeric),
            ["20", "21", "22", "23", "24", "25", "26", "27"]
        );
        assert_eq!(ids(&literal), ["26"]);
        assert_eq!(ids(&prefix), ["20", "21", "22", "23", "24", "25"]);
        assert!(injection.rows.is_empty());
        assert_eq!(ids(&sorted), ["3", "2", "1"]);
        assert_eq!(sorted.order.len(), 2, "the primary key breaks ties");
        assert_eq!(ids(&nulls), ["2"]);
        drop_scratch(&name, session).await;
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn requests_that_do_not_fit_the_table_are_refused() {
        let (name, session) = scratch_database("aidb_pages", SCHEMA_SQL).await;
        let mut bad_value = request("items");
        bad_value.filters = vec![filter("id", FilterOperator::Equals, Some("abc"))];
        let mut no_operator = request("samples");
        no_operator.filters = vec![filter("raw", FilterOperator::Equals, Some("{}"))];
        let mut unknown = request("items");
        unknown.sort = vec![SortKey {
            column: "label; DROP TABLE items".into(),
            direction: SortDirection::Asc,
        }];

        let errors = [
            session
                .read_page(&bad_value, LIMITS)
                .await
                .expect_err("bad value"),
            session
                .read_page(&no_operator, LIMITS)
                .await
                .expect_err("json ="),
            session
                .read_page(&unknown, LIMITS)
                .await
                .expect_err("unknown"),
            session
                .read_page(&request("missing"), LIMITS)
                .await
                .expect_err("missing"),
        ];

        let kinds: Vec<DbErrorKind> = errors.iter().map(DbError::kind).collect();
        assert_eq!(
            kinds,
            [
                DbErrorKind::InvalidRequest,
                DbErrorKind::InvalidRequest,
                DbErrorKind::InvalidRequest,
                DbErrorKind::ObjectNotFound,
            ]
        );
        assert!(errors[0].message().contains("abc"), "{}", errors[0]);
        session.ping().await.expect("the session still works");
        drop_scratch(&name, session).await;
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn the_byte_limit_stops_a_page_early() {
        let (name, session) = scratch_database("aidb_pages", SCHEMA_SQL).await;

        let page = session
            .read_page(
                &request("items"),
                PageLimits {
                    max_rows: 500,
                    max_bytes: 100,
                },
            )
            .await
            .expect("page");

        assert!(page.byte_limit_reached && page.has_more);
        assert!((1..10).contains(&page.rows.len()), "{}", page.rows.len());
        drop_scratch(&name, session).await;
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn pages_are_read_only_even_on_writable_sessions() {
        let (name, session) = scratch_database("aidb_pages", SCHEMA_SQL).await;

        let error = session
            .read_page(&request("sneaky"), LIMITS)
            .await
            .expect_err("the view writes");

        assert_eq!(error.kind(), DbErrorKind::ReadOnly);
        drop_scratch(&name, session).await;
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn page_reads_can_be_cancelled() {
        let (name, session) = scratch_database("aidb_pages", SCHEMA_SQL).await;
        let session = Arc::new(session);
        let operation = OperationId::new();
        let mut read = request("slow");
        read.operation_id = Some(operation);
        let started = Instant::now();
        let reader = Arc::clone(&session);
        let running = tokio::spawn(async move { reader.read_page(&read, LIMITS).await });

        tokio::time::sleep(Duration::from_millis(500)).await;
        let signalled = session.cancel(operation).await.expect("cancel");
        let error = running.await.expect("task").expect_err("cancelled");

        assert!(signalled);
        assert_eq!(error.kind(), DbErrorKind::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(5));
        let session = Arc::into_inner(session).expect("only owner");
        drop_scratch(&name, session).await;
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn columns_the_role_cannot_read_are_left_out() {
        let (name, session) = scratch_database("aidb_pages", SCHEMA_SQL).await;
        let role = format!(
            "aidb_reader_{}",
            OperationId::new().to_string().replace('-', "")
        );
        run(
            &session,
            &format!(
                "CREATE ROLE {role} LOGIN PASSWORD 'reader-only'; \
                 GRANT SELECT (id, name) ON staff TO {role};"
            ),
        )
        .await
        .expect("role");
        let mut reader_target = target(&fixture(), true);
        reader_target.settings.database = name.clone();
        reader_target.settings.username = role.clone();
        reader_target.password = Some(SecretString::from("reader-only"));
        let reader = open(reader_target).await;

        let page = reader
            .read_page(&request("staff"), LIMITS)
            .await
            .expect("page");
        let mut by_salary = request("staff");
        by_salary.sort = vec![SortKey {
            column: "salary".into(),
            direction: SortDirection::Asc,
        }];
        let refused = reader
            .read_page(&by_salary, LIMITS)
            .await
            .expect_err("salary");
        let none = reader
            .read_page(&request("items"), LIMITS)
            .await
            .expect_err("no grant");

        assert_eq!(
            page.columns
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["id", "name"]
        );
        assert_eq!(page.omitted_columns, ["salary"]);
        assert_eq!(page.rows, [[Some("1".to_owned()), Some("Ada".to_owned())]]);
        assert_eq!(refused.kind(), DbErrorKind::PermissionDenied);
        assert_eq!(none.kind(), DbErrorKind::PermissionDenied);
        reader.close().await;
        drop_scratch(&name, session).await;
        let admin = open(target(&fixture(), false)).await;
        run(&admin, &format!("DROP ROLE {role}"))
            .await
            .expect("drop role");
        admin.close().await;
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn sql_ascii_databases_are_readable_whatever_their_bytes() {
        let (name, session) = scratch_database_with(
            "aidb_pages",
            "ENCODING 'SQL_ASCII' LC_COLLATE 'C' LC_CTYPE 'C'",
            r"
                CREATE TABLE legacy (id integer PRIMARY KEY, body text);
                INSERT INTO legacy VALUES
                    (1, convert_from('\xff41'::bytea, 'SQL_ASCII')),
                    (2, repeat('é', 3000)),
                    (3, 'plain');
            ",
        )
        .await;

        let page = session
            .read_page(&request("legacy"), LIMITS)
            .await
            .expect("page");

        assert_eq!(
            cell(&page, 0, "body"),
            Some("\u{fffd}A"),
            "invalid bytes replaced"
        );
        let long = cell(&page, 1, "body").expect("long");
        assert_eq!(long.chars().count(), CELL_CHAR_LIMIT);
        assert!(long.chars().all(|c| c == 'é'), "cut between characters");
        assert_eq!(page.truncated_cells.len(), 1);
        assert_eq!(cell(&page, 2, "body"), Some("plain"));
        drop_scratch(&name, session).await;
    }
}
