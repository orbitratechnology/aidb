//! Risk assessment end to end: the policy service with the app's local store and a real
//! session on the PostgreSQL fixture, so estimates, privileges and the profile's settings all
//! come from where they come from in the app.
//!
//! `scripts/postgres-fixture.sh start && source target/postgres-fixture/env`
//! `cargo test -p aidb --test policy -- --ignored`

mod support;

use aidb_lib::domain::policy::classify::{ImpactSource, ReasonCode};
use aidb_lib::domain::policy::facts::Operation;
use aidb_lib::domain::policy::risk::{ApprovalLevel, RiskLevel};
use aidb_lib::services::policy;
use support::{ScratchDatabase, Workspace};

fn schema() -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/agent/schema.sql"),
    )
    .expect("schema.sql")
}

#[tokio::test]
#[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
async fn assessments_use_the_database_and_the_profile() {
    let database = ScratchDatabase::create("aidb_policy", &schema()).await;
    let before = database.snapshot().await;
    let workspace = Workspace::connect(&database.name).await;
    let assess = |sql: &'static str| {
        let workspace = &workspace;
        async move {
            policy::assess(
                &workspace.store,
                &workspace.sessions,
                workspace.connection_id,
                sql,
            )
            .await
            .expect("assessment")
        }
    };

    let read = assess("SELECT * FROM orders WHERE status = 'shipped'").await;
    let small = assess("UPDATE orders SET status = 'shipped' WHERE id = 3").await;
    let bulk = assess("UPDATE order_items SET quantity = quantity + 1 WHERE quantity > 0").await;
    let insert = assess("INSERT INTO products VALUES (5, 'Cable', 'hardware', 5.00)").await;
    let every = assess("DELETE FROM support_tickets").await;
    let drop = assess("DROP TABLE support_tickets").await;
    let batch = assess(
        "UPDATE orders SET status = 'shipped' WHERE id = 3; \
         UPDATE orders SET status = 'pending' WHERE id = 4",
    )
    .await;

    workspace.close().await;
    let after = database.snapshot().await;
    database.drop().await;

    assert_eq!(read.risk, RiskLevel::R1);
    assert_eq!(read.required_approval, ApprovalLevel::None);

    // The planner estimates one row; the profile is read-only, as new connections are.
    assert_eq!(small.risk, RiskLevel::R2);
    let impact = small.statements[0].impact.expect("impact");
    assert_eq!(
        (impact.rows, impact.source),
        (Some(1), ImpactSource::Planner)
    );
    assert_eq!(small.required_approval, ApprovalLevel::Blocked);
    assert!(
        small.blockers[0].contains("read-only"),
        "{:?}",
        small.blockers
    );

    // 16 order items is small; the threshold is 100.
    assert_eq!(bulk.statements[0].impact.and_then(|i| i.rows), Some(16));
    assert_eq!(bulk.risk, RiskLevel::R2);

    assert_eq!(insert.statements[0].operation, Operation::Insert);
    assert_eq!(
        insert.statements[0].impact.map(|i| i.source),
        Some(ImpactSource::Statement)
    );

    assert_eq!(every.risk, RiskLevel::R4);
    assert!(
        every.statements[0]
            .reasons
            .iter()
            .any(|reason| reason.code == ReasonCode::ChangesEveryRow)
    );
    assert_eq!(drop.risk, RiskLevel::R4);
    assert_eq!(batch.risk, RiskLevel::R3);

    assert_eq!(before, after, "assessing changed the database");
}
