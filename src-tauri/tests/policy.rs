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

mod approvals {
    use aidb_lib::domain::audit::Actor;
    use aidb_lib::domain::connections::Environment;
    use aidb_lib::domain::policy::approval::ApprovalState;
    use aidb_lib::domain::policy::risk::ApprovalLevel;
    use aidb_lib::errors::ErrorCode;
    use aidb_lib::services::approvals::{self, Approvals};

    use super::schema;
    use super::support::{ScratchDatabase, Workspace};

    // Literal: statements with $n placeholders cannot be planned without their values yet, so
    // their impact counts as unknown (R3).
    const UPDATE: &str = "UPDATE orders SET status = 'shipped' WHERE id = 3";

    fn params() -> Vec<Option<String>> {
        Vec::new()
    }

    /// Requests and approves the UPDATE on a writable profile.
    async fn approved(workspace: &Workspace, approvals: &Approvals) -> approvals::ApprovalGrant {
        let request = approvals::request(
            &workspace.store,
            &workspace.sessions,
            approvals,
            workspace.connection_id,
            UPDATE,
            params(),
            Actor::User,
        )
        .await
        .expect("request");
        assert_eq!(request.level, ApprovalLevel::Apply);
        approvals::approve(
            &workspace.store,
            &workspace.sessions,
            approvals,
            request.id,
            None,
        )
        .await
        .expect("approve")
    }

    #[tokio::test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    async fn approvals_bind_to_the_live_database() {
        let database = ScratchDatabase::create("aidb_approval", &schema()).await;
        let workspace =
            Workspace::connect_with(&database.name, |draft| draft.read_only = false).await;
        let approvals = Approvals::default();

        // Used as approved: the execution layer gets the go-ahead (nothing runs here).
        let grant = approved(&workspace, &approvals).await;
        let operation = approvals::consume(
            &workspace.store,
            &workspace.sessions,
            &approvals,
            workspace.connection_id,
            &grant.token,
            UPDATE,
            &params(),
        )
        .await
        .expect("use");
        assert_eq!(
            approvals.state(operation.id),
            Some(ApprovalState::Executing)
        );
        approvals::finish(&approvals, operation.id, ApprovalState::Cancelled).expect("finish");

        // The schema changes after approval: the live fingerprint no longer matches.
        let grant = approved(&workspace, &approvals).await;
        database
            .execute("ALTER TABLE orders ADD COLUMN note text")
            .await;
        let error = approvals::consume(
            &workspace.store,
            &workspace.sessions,
            &approvals,
            workspace.connection_id,
            &grant.token,
            UPDATE,
            &params(),
        )
        .await
        .expect_err("schema changed");
        assert_eq!(error.code(), ErrorCode::PermissionDenied);
        assert!(error.message().contains("structure"), "{error}");

        // The profile's environment changes after approval.
        let grant = approved(&workspace, &approvals).await;
        let mut profile = workspace
            .store
            .connections()
            .get(workspace.connection_id)
            .await
            .expect("read")
            .expect("profile");
        profile.settings.environment = Environment::Staging;
        workspace
            .store
            .connections()
            .update(
                workspace.connection_id,
                &profile.settings.clone().validated().expect("valid"),
            )
            .await
            .expect("update");
        let error = approvals::consume(
            &workspace.store,
            &workspace.sessions,
            &approvals,
            workspace.connection_id,
            &grant.token,
            UPDATE,
            &params(),
        )
        .await
        .expect_err("environment changed");
        assert!(error.message().contains("environment"), "{error}");

        workspace.close().await;
        database.drop().await;
    }
}
