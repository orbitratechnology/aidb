//! Performance on the PostgreSQL fixture: a large catalog and a large table, through the same
//! services the interface waits on. Each step has a budget well above what it takes on a CI
//! machine, so a failure means a regression of an order of magnitude, not noise; the timings
//! are printed (run with `--nocapture`) and recorded in `docs/performance.md`.
//!
//! `scripts/postgres-fixture.sh start && source target/postgres-fixture/env`
//! `cargo test -p aidb --test performance -- --ignored --nocapture`

#![allow(
    clippy::print_stderr,
    reason = "the measurements are the report; they are printed for the CI log"
)]

mod support;

use std::time::{Duration, Instant};

use aidb_lib::domain::catalog::ObjectName;
use aidb_lib::domain::export::{ExportFormat, ExportRequest};
use aidb_lib::domain::query::{QueryRequest, QueryRun};
use aidb_lib::domain::table_data::{PageRequest, SortDirection, SortKey};
use aidb_lib::services::{catalog, query, table_data};
use support::{ScratchDatabase, Workspace};

const TABLES: usize = 2_000;
const VIEWS: usize = 200;
const ROWS: u64 = 300_000;

const SCHEMAS: &str = "ARRAY['public', 'sales', 'billing', 'ops', 'archive']";

/// Four schemas and one large table.
fn schema() -> String {
    format!(
        r"
CREATE SCHEMA sales; CREATE SCHEMA billing; CREATE SCHEMA ops; CREATE SCHEMA archive;
CREATE TABLE public.events (
  id bigint PRIMARY KEY,
  kind text NOT NULL,
  payload text NOT NULL,
  at timestamptz NOT NULL
);
INSERT INTO public.events
SELECT g, 'kind_' || (g % 37), md5(g::text) || md5((g * 7)::text), now() - g * interval '1 second'
FROM generate_series(1, {ROWS}) g;
"
    )
}

/// Tables `from..=to` (a primary key, a few columns and an index each), spread over the
/// schemas. Created in batches: one transaction would need a lock for every table.
fn tables(from: usize, to: usize) -> String {
    format!(
        r"
DO $$
DECLARE s text[] := {SCHEMAS};
BEGIN
  FOR i IN {from}..{to} LOOP
    EXECUTE format(
      'CREATE TABLE %I.t%s (id bigint PRIMARY KEY, name text NOT NULL, amount numeric(12, 2),
         created timestamptz NOT NULL DEFAULT now(), note text)',
      s[1 + i % 5], i);
    EXECUTE format('CREATE INDEX ON %I.t%s (created)', s[1 + i % 5], i);
  END LOOP;
END $$;
"
    )
}

/// A view over each of the first tables.
fn views() -> String {
    format!(
        r"
DO $$
DECLARE s text[] := {SCHEMAS};
BEGIN
  FOR i IN 1..{VIEWS} LOOP
    EXECUTE format('CREATE VIEW %I.v%s AS SELECT id, name FROM %I.t%s',
      s[1 + i % 5], i, s[1 + i % 5], i);
  END LOOP;
END $$;
ANALYZE;
"
    )
}

struct Timings(Vec<(&'static str, Duration, Duration)>);

impl Timings {
    /// Times `step`, records it with its budget, and fails the test if it ran over.
    async fn measure<T>(
        &mut self,
        name: &'static str,
        budget: Duration,
        step: impl std::future::Future<Output = T>,
    ) -> T {
        let started = Instant::now();
        let value = step.await;
        let took = started.elapsed();
        self.0.push((name, took, budget));
        value
    }

    fn report_and_check(&self) {
        eprintln!("\n{:<48} {:>10} {:>10}", "step", "took", "budget");
        for (name, took, budget) in &self.0 {
            eprintln!(
                "{name:<48} {:>8}ms {:>8}ms",
                took.as_millis(),
                budget.as_millis()
            );
        }
        let over: Vec<_> = self
            .0
            .iter()
            .filter(|(_, took, budget)| took > budget)
            .map(|(name, took, budget)| format!("{name}: {took:?} > {budget:?}"))
            .collect();
        assert!(over.is_empty(), "over budget:\n{}", over.join("\n"));
    }
}

fn object(schema: &str, name: &str) -> ObjectName {
    ObjectName {
        schema: schema.into(),
        name: name.into(),
    }
}

fn page(name: &str, sort: Vec<SortKey>, offset: u64) -> PageRequest {
    PageRequest {
        object: object("public", name),
        filters: Vec::new(),
        sort,
        offset,
        limit: None,
        operation_id: None,
    }
}

#[tokio::test]
#[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
async fn large_catalogs_and_tables_stay_within_budget() {
    let setup = Instant::now();
    let db = ScratchDatabase::create("aidb_perf", &schema()).await;
    for from in (1..=TABLES).step_by(200) {
        db.execute(&tables(from, (from + 199).min(TABLES))).await;
    }
    db.execute(&views()).await;
    eprintln!(
        "setup: {TABLES} tables, {VIEWS} views, {ROWS} rows in {:?}",
        setup.elapsed()
    );
    let w = Workspace::connect(&db.name).await;
    let mut t = Timings(Vec::new());
    let id = w.connection_id;

    // The schema browser: the first overview reads the catalog; later ones reuse it.
    let snapshot = t
        .measure(
            "catalog overview, cold",
            Duration::from_secs(5),
            catalog::overview(&w.store, &w.sessions, &w.cache, id, true),
        )
        .await
        .expect("overview");
    assert!(
        snapshot.overview.objects.len() > TABLES + VIEWS,
        "every object is listed"
    );
    t.measure(
        "catalog overview, cached",
        Duration::from_millis(500),
        catalog::overview(&w.store, &w.sessions, &w.cache, id, false),
    )
    .await
    .expect("cached overview");
    t.measure(
        "describe a table",
        Duration::from_secs(2),
        catalog::describe(
            &w.store,
            &w.sessions,
            &w.cache,
            id,
            object("public", "events"),
        ),
    )
    .await
    .expect("describe");

    // The data grid: pages are bounded, whatever the table's size.
    let first = t
        .measure(
            "first page, primary key order",
            Duration::from_secs(1),
            table_data::read_page(&w.store, &w.sessions, id, page("events", Vec::new(), 0)),
        )
        .await
        .expect("first page");
    assert!(first.rows.len() <= 500, "a page is bounded");
    t.measure(
        "first page, sorted by an unindexed column",
        Duration::from_secs(5),
        table_data::read_page(
            &w.store,
            &w.sessions,
            id,
            page(
                "events",
                vec![SortKey {
                    column: "kind".into(),
                    direction: SortDirection::Desc,
                }],
                0,
            ),
        ),
    )
    .await
    .expect("sorted page");
    t.measure(
        "a page 250,000 rows in",
        Duration::from_secs(5),
        table_data::read_page(
            &w.store,
            &w.sessions,
            id,
            page("events", Vec::new(), 250_000),
        ),
    )
    .await
    .expect("deep page");

    // The SQL editor: a query over the whole table returns its first page, not every row.
    let run = t
        .measure(
            "query: first page of SELECT * (300,000 rows)",
            Duration::from_secs(2),
            query::run(
                &w.store,
                &w.sessions,
                id,
                QueryRequest {
                    sql: "SELECT * FROM events".into(),
                    offset: 0,
                    operation_id: None,
                },
            ),
        )
        .await
        .expect("query");
    match run {
        QueryRun::Completed(result) => {
            assert!(result.has_more, "the rest is left for later pages");
            assert!(result.rows.len() <= 500);
        }
        other => panic!("query did not complete: {other:?}"),
    }

    // Export streams every row to a file.
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("events.csv");
    let summary = t
        .measure(
            "export 300,000 rows to CSV",
            Duration::from_secs(30),
            table_data::export(
                &w.sessions,
                id,
                &ExportRequest {
                    object: object("public", "events"),
                    filters: Vec::new(),
                    sort: Vec::new(),
                    format: ExportFormat::Csv,
                    operation_id: None,
                },
                &path,
                1_000_000,
            ),
        )
        .await
        .expect("export");
    assert_eq!(summary.rows, ROWS);
    let bytes = std::fs::metadata(&path).expect("file").len();
    eprintln!("export: {} MB", bytes / 1_000_000);

    w.close().await;
    db.drop().await;
    t.report_and_check();
}
