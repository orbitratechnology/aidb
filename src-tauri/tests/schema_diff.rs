//! Schema comparison on the PostgreSQL fixture: schemas read from the catalog, migrations
//! generated for them, and those migrations run, after which comparing again finds nothing to do.
//!
//! `scripts/postgres-fixture.sh start && source target/postgres-fixture/env`
//! `cargo test -p aidb --test schema_diff -- --ignored`

mod support;

use aidb_lib::adapters::database::DatabaseSession;
use aidb_lib::domain::connections::Environment;
use aidb_lib::domain::schema_diff::generate::{Plan, plan};
use aidb_lib::domain::schema_diff::{GenerateOptions, NoteKind, SchemaModel, StepAction, parse};
use support::{ScratchDatabase, Workspace};

/// `public` is the database as it is; `staging` is where it should go.
const SCHEMAS: &str = r"
CREATE TYPE public.mood AS ENUM ('ok', 'sad');
CREATE TABLE public.customers (
    id serial PRIMARY KEY,
    name varchar(40),
    legacy text,
    mood public.mood DEFAULT 'ok'
);
CREATE INDEX customers_legacy_idx ON public.customers (legacy);
CREATE TABLE public.old_log (line text);
CREATE VIEW public.names AS SELECT name FROM public.customers;
CREATE VIEW public.ids AS SELECT id FROM public.customers;
INSERT INTO public.customers (name, legacy) VALUES ('Ada', 'x'), ('Grace', 'y');

CREATE SCHEMA staging;
CREATE TYPE staging.mood AS ENUM ('ok', 'sad');
CREATE TABLE staging.customers (
    id serial PRIMARY KEY,
    name text NOT NULL CHECK (length(name) > 0),
    mood staging.mood DEFAULT 'ok',
    email text,
    created timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX customers_email_lower ON staging.customers (lower(email))
    WHERE email IS NOT NULL;
CREATE TABLE staging.orders (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    customer_id int NOT NULL REFERENCES staging.customers (id) ON DELETE CASCADE,
    total numeric(10, 2) CHECK (total >= 0),
    doubled numeric GENERATED ALWAYS AS (total * 2) STORED
);
CREATE INDEX orders_customer_idx ON staging.orders (customer_id);
CREATE VIEW staging.names AS SELECT name FROM staging.customers;
CREATE VIEW staging.ids AS SELECT id FROM staging.customers;
CREATE VIEW staging.big_orders AS SELECT id, total FROM staging.orders WHERE total > 100;
";

async fn read(workspace: &Workspace, schema: &str) -> SchemaModel {
    workspace
        .sessions
        .get(workspace.connection_id)
        .expect("open")
        .read_schema(schema)
        .await
        .expect("reads")
}

fn sqls(plan: &Plan) -> Vec<&str> {
    plan.steps.iter().map(|step| step.sql.as_str()).collect()
}

#[tokio::test]
#[ignore = "needs the PostgreSQL fixture"]
async fn a_schema_migrated_to_match_another_compares_equal() {
    let database = ScratchDatabase::create("aidb_diff", SCHEMAS).await;
    let workspace = Workspace::connect(&database.name).await;

    let source = read(&workspace, "staging").await;
    let target = read(&workspace, "public").await;
    assert_eq!(target.tables[0].name, "customers");
    assert!(target.tables[0].columns[0].serial);
    assert!(target.tables[0].rows.unwrap_or(0) <= 2);
    assert_eq!(target.views[0].reads, ["customers.name"]);
    assert!(
        target.not_compared.iter().any(|n| n == "1 type"),
        "{:?}",
        target.not_compared
    );

    let migration = plan(
        &source,
        &target,
        GenerateOptions::default(),
        Environment::Staging,
    );
    let steps = sqls(&migration);
    assert!(steps.contains(&"DROP TABLE public.old_log"), "{steps:#?}");
    assert!(
        steps
            .contains(&"ALTER TABLE public.customers ALTER COLUMN name TYPE text USING name::text"),
        "{steps:#?}"
    );
    // Only the view reading the retyped column is rebuilt.
    assert!(steps.contains(&"DROP VIEW public.names"), "{steps:#?}");
    assert!(!steps.contains(&"DROP VIEW public.ids"), "{steps:#?}");
    let drop_legacy = migration
        .steps
        .iter()
        .find(|step| step.sql.ends_with("DROP COLUMN legacy"))
        .expect("drops legacy");
    assert_eq!(drop_legacy.action, StepAction::Remove);
    assert_eq!(drop_legacy.notes[0].kind, NoteKind::DataLoss);

    database.execute(&migration.script).await;

    let after = read(&workspace, "public").await;
    let again = plan(
        &read(&workspace, "staging").await,
        &after,
        GenerateOptions::default(),
        Environment::Staging,
    );
    assert!(again.steps.is_empty(), "left over: {:#?}", sqls(&again));
    let snapshot = database.snapshot().await;
    assert!(
        snapshot.iter().any(|row| row.starts_with("customers:r:")),
        "{snapshot:?}"
    );

    workspace.close().await;
    database.drop().await;
}

/// A hand-written schema with the forms `pg_dump` uses mixed in.
const FILE: &str = r"
CREATE TYPE mood AS ENUM ('ok', 'sad');
CREATE TABLE customers (
    id serial PRIMARY KEY,
    email varchar(120) NOT NULL UNIQUE,
    mood mood DEFAULT 'ok',
    score numeric(5) DEFAULT 0 CHECK (score BETWEEN 0 AND 100),
    tags text[] DEFAULT '{}',
    seen timestamp(3) without time zone
);
CREATE TABLE public.orders (
    id bigint NOT NULL,
    customer_id integer NOT NULL,
    placed date DEFAULT CURRENT_DATE NOT NULL,
    note text
);
ALTER TABLE public.orders ALTER COLUMN id ADD GENERATED BY DEFAULT AS IDENTITY (
    SEQUENCE NAME public.orders_id_seq START WITH 1 INCREMENT BY 1 NO MINVALUE NO MAXVALUE CACHE 1
);
ALTER TABLE ONLY public.orders ADD CONSTRAINT orders_pkey PRIMARY KEY (id);
ALTER TABLE ONLY public.orders
    ADD CONSTRAINT orders_customer_id_fkey FOREIGN KEY (customer_id) REFERENCES public.customers(id);
CREATE INDEX orders_recent ON public.orders USING btree (placed DESC) WHERE (note IS NOT NULL);
CREATE VIEW recent_orders AS SELECT id, placed FROM orders WHERE placed > CURRENT_DATE - 7;
";

#[tokio::test]
#[ignore = "needs the PostgreSQL fixture"]
async fn a_schema_built_from_a_file_compares_equal_to_the_file() {
    let database = ScratchDatabase::create(
        "aidb_diff",
        "CREATE SCHEMA app; CREATE TYPE app.mood AS ENUM ('ok', 'sad');",
    )
    .await;
    let workspace = Workspace::connect(&database.name).await;
    let file = parse::read(FILE, "public").expect("reads");
    assert!(file.not_compared.iter().any(|n| n == "1 type"));

    let migration = plan(
        &file,
        &read(&workspace, "app").await,
        GenerateOptions::default(),
        Environment::Development,
    );
    assert_eq!(migration.summary.tables_added, 2);
    database.execute(&migration.script).await;

    let again = plan(
        &file,
        &read(&workspace, "app").await,
        GenerateOptions::default(),
        Environment::Development,
    );
    assert!(again.steps.is_empty(), "left over: {:#?}", sqls(&again));

    workspace.close().await;
    database.drop().await;
}

#[tokio::test]
#[ignore = "needs the PostgreSQL fixture"]
async fn concurrent_indexes_run_after_the_transaction() {
    let database = ScratchDatabase::create(
        "aidb_diff",
        r"
        CREATE TABLE public.t (a int, b int);
        CREATE INDEX t_a ON public.t (a);
        CREATE SCHEMA wanted;
        CREATE TABLE wanted.t (a int, b int);
        CREATE INDEX t_b ON wanted.t (b);
        ",
    )
    .await;
    let workspace = Workspace::connect(&database.name).await;
    let options = GenerateOptions {
        concurrent_indexes: true,
    };
    let migration = plan(
        &read(&workspace, "wanted").await,
        &read(&workspace, "public").await,
        options,
        Environment::Development,
    );
    assert_eq!(
        sqls(&migration),
        [
            "DROP INDEX CONCURRENTLY public.t_a",
            "CREATE INDEX CONCURRENTLY t_b ON public.t USING btree (b)",
        ]
    );
    // Each runs on its own, as psql runs a script; none can run in a transaction block.
    for step in &migration.steps {
        assert!(!step.transactional);
        database.execute(&step.sql).await;
    }
    let again = plan(
        &read(&workspace, "wanted").await,
        &read(&workspace, "public").await,
        options,
        Environment::Development,
    );
    assert!(again.steps.is_empty(), "left over: {:#?}", sqls(&again));

    workspace.close().await;
    database.drop().await;
}

#[tokio::test]
#[ignore = "needs the PostgreSQL fixture"]
async fn another_connection_is_read_once_without_joining_the_open_sessions() {
    use aidb_lib::adapters::credentials::Credentials;
    use aidb_lib::services::sessions::read_schema_elsewhere;

    let database = ScratchDatabase::create("aidb_diff", SCHEMAS).await;
    let workspace = Workspace::connect(&database.name).await;
    let (mut settings, password) = support::fixture();
    settings.name = "Staging copy".into();
    database.name.clone_into(&mut settings.database);
    settings.read_only = false;
    let other = workspace
        .store
        .connections()
        .insert(&settings.validated().expect("valid"))
        .await
        .expect("profile");

    let credentials = Credentials::session_only("tests");
    let (name, model) = read_schema_elsewhere(
        &workspace.store,
        &credentials,
        &workspace.sessions,
        other.id,
        Some(password),
        "staging",
    )
    .await
    .expect("reads");
    assert_eq!(name, "Staging copy");
    assert_eq!(model.schema, "staging");
    assert_eq!(model.tables.len(), 2);
    // The workspace still has its one session; the other was closed after reading.
    let open = workspace.sessions.list();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].connection_id, workspace.connection_id);

    let missing = read_schema_elsewhere(
        &workspace.store,
        &credentials,
        &workspace.sessions,
        other.id,
        Some(support::fixture().1),
        "nowhere",
    )
    .await
    .expect_err("no such schema");
    assert_eq!(missing.code(), aidb_lib::errors::ErrorCode::NotFound);

    workspace.close().await;
    database.drop().await;
}
