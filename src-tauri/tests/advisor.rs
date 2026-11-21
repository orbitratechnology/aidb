//! The query advisor on the PostgreSQL fixture: real plans, real statistics, and suggestions
//! that hold up when acted on.
//!
//! `scripts/postgres-fixture.sh start && source target/postgres-fixture/env`
//! `cargo test -p aidb --test advisor -- --ignored`

mod support;

use aidb_lib::domain::advisor::{ExplainRequest, QueryPlan, SuggestionKind};
use aidb_lib::errors::ErrorCode;
use aidb_lib::services::advisor;
use support::{ScratchDatabase, Workspace};

const SCHEMA: &str = r"
CREATE TABLE orders (
    id int PRIMARY KEY,
    customer_id int NOT NULL,
    total numeric(10, 2) NOT NULL,
    note text NOT NULL
);
INSERT INTO orders
SELECT g, g % 5000, g % 1000, md5(g::text) || md5((g * 7)::text)
FROM generate_series(1, 200000) g;

-- Two columns that always agree: the planner multiplies their selectivities.
CREATE TABLE pairs (a int NOT NULL, b int NOT NULL);
INSERT INTO pairs SELECT g % 100, g % 100 FROM generate_series(1, 100000) g;

ANALYZE;
";

async fn explain(workspace: &Workspace, sql: &str, analyze: bool) -> QueryPlan {
    advisor::explain(
        &workspace.sessions,
        workspace.connection_id,
        &ExplainRequest {
            sql: sql.into(),
            analyze,
            operation_id: None,
        },
    )
    .await
    .expect("explains")
}

fn kinds(plan: &QueryPlan) -> Vec<SuggestionKind> {
    plan.suggestions.iter().map(|s| s.kind).collect()
}

#[tokio::test]
#[ignore = "needs the PostgreSQL fixture"]
async fn a_missing_index_is_suggested_and_the_suggestion_holds_up() {
    let database = ScratchDatabase::create("aidb_advisor", SCHEMA).await;
    let workspace = Workspace::connect(&database.name).await;
    let query = "SELECT id, total FROM orders WHERE customer_id = 42";

    let planned = explain(&workspace, query, false).await;
    assert!(!planned.analyzed);
    assert!(
        planned
            .nodes
            .iter()
            .any(|n| n.node_type.contains("Seq Scan"))
    );
    let suggestion = planned
        .suggestions
        .iter()
        .find(|s| s.kind == SuggestionKind::MissingIndex)
        .unwrap_or_else(|| panic!("{:#?}", planned.suggestions));
    assert_eq!(
        suggestion.sql.as_deref(),
        Some("CREATE INDEX CONCURRENTLY ON public.orders (customer_id);")
    );
    assert!(suggestion.evidence[0].contains("expected to read 200,000 rows"));

    let measured = explain(&workspace, query, true).await;
    assert!(measured.analyzed);
    assert!(measured.execution_ms.is_some());
    let evidence = &measured.suggestions[0].evidence[0];
    assert!(evidence.contains("kept 40"), "{evidence}");
    assert!(measured.nodes[0].actual_ms.is_some());

    // Acting on it: the plan uses the index and the suggestion is gone.
    database
        .execute(suggestion.sql.as_deref().expect("sql"))
        .await;
    database.execute("ANALYZE orders").await;
    let after = explain(&workspace, query, true).await;
    assert!(
        after
            .nodes
            .iter()
            .any(|n| n.node_type.contains("Index") || n.node_type.contains("Bitmap")),
        "{:#?}",
        after.nodes
    );
    assert!(!kinds(&after).contains(&SuggestionKind::MissingIndex));

    workspace.close().await;
    database.drop().await;
}

#[tokio::test]
#[ignore = "needs the PostgreSQL fixture"]
async fn far_off_estimates_and_spilled_sorts_are_found_when_measured() {
    let database = ScratchDatabase::create("aidb_advisor", SCHEMA).await;
    let workspace = Workspace::connect(&database.name).await;

    let correlated = explain(
        &workspace,
        "SELECT * FROM pairs WHERE a = 1 AND b = 1",
        true,
    )
    .await;
    let stale = correlated
        .suggestions
        .iter()
        .find(|s| s.kind == SuggestionKind::StaleStatistics)
        .unwrap_or_else(|| panic!("{:#?}", correlated.suggestions));
    assert_eq!(stale.sql.as_deref(), Some("ANALYZE public.pairs;"));
    assert!(
        stale.evidence[0].contains("got 1,000"),
        "{:?}",
        stale.evidence
    );

    // 200,000 rows of 64-character notes do not fit in the default 4 MB of work_mem.
    let sorted = explain(&workspace, "SELECT note FROM orders ORDER BY note", true).await;
    assert!(
        kinds(&sorted).contains(&SuggestionKind::SortSpill),
        "{:#?}",
        sorted.suggestions
    );
    let sort = sorted
        .nodes
        .iter()
        .find(|n| n.node_type == "Sort")
        .expect("a sort");
    assert!(
        sort.details.iter().any(|d| d.contains("Disk")),
        "{:?}",
        sort.details
    );

    workspace.close().await;
    database.drop().await;
}

#[tokio::test]
#[ignore = "needs the PostgreSQL fixture"]
async fn only_queries_are_explained() {
    let database = ScratchDatabase::create("aidb_advisor", SCHEMA).await;
    let workspace = Workspace::connect(&database.name).await;
    for refused in [
        "DELETE FROM orders",
        "EXPLAIN ANALYZE DELETE FROM orders",
        "SELECT 1; DROP TABLE orders",
    ] {
        let error = advisor::explain(
            &workspace.sessions,
            workspace.connection_id,
            &ExplainRequest {
                sql: refused.into(),
                analyze: true,
                operation_id: None,
            },
        )
        .await
        .expect_err(refused);
        assert_eq!(error.code(), ErrorCode::InvalidInput, "{refused}");
    }
    // Measuring runs the query in a read-only transaction that is rolled back: nothing changes.
    let snapshot = database.snapshot().await;
    let _ = advisor::explain(
        &workspace.sessions,
        workspace.connection_id,
        &ExplainRequest {
            sql: "SELECT count(*) FROM orders".into(),
            analyze: true,
            operation_id: None,
        },
    )
    .await
    .expect("measures");
    assert_eq!(database.snapshot().await, snapshot);
    workspace.close().await;
    database.drop().await;
}
