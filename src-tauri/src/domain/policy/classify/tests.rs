use super::*;
use crate::domain::policy::facts::inspect;
use crate::domain::policy::risk::RiskLevel::{R1, R2, R3, R4};

fn development() -> PolicyContext {
    PolicyContext {
        environment: Environment::Development,
        writes_allowed: true,
    }
}

fn rows(rows: u64, source: ImpactSource) -> ImpactFacts {
    ImpactFacts {
        rows: Some(rows),
        source,
        ..ImpactFacts::unknown()
    }
}

fn assess_sql(sql: &str, impacts: &[ImpactFacts], context: PolicyContext) -> RiskAssessment {
    assess(&inspect(sql), impacts, context)
}

/// The risk and the reason codes of a single statement.
fn one(sql: &str, impact: ImpactFacts) -> (RiskLevel, Vec<ReasonCode>, Operation) {
    let assessment = assess_sql(sql, &[impact], development());
    assert_eq!(assessment.statements.len(), 1, "{sql}");
    let statement = &assessment.statements[0];
    (
        statement.risk,
        statement.reasons.iter().map(|reason| reason.code).collect(),
        statement.operation,
    )
}

fn risk(sql: &str) -> RiskLevel {
    one(sql, ImpactFacts::unknown()).0
}

#[test]
fn reads_and_plans_are_bounded_reads() {
    let assessment = assess_sql(
        "SELECT * FROM orders WHERE id = 1",
        &[],
        PolicyContext {
            writes_allowed: false,
            ..development()
        },
    );
    assert_eq!(assessment.risk, R1);
    assert_eq!(assessment.required_approval, ApprovalLevel::None);
    assert!(assessment.blockers.is_empty());
    assert_eq!(assessment.statements[0].impact, None);
    assert_eq!(assessment.policy_version, POLICY_VERSION);

    // EXPLAIN without ANALYZE only plans, even for a write.
    assert_eq!(
        one("EXPLAIN DELETE FROM orders", ImpactFacts::unknown()),
        (R1, vec![ReasonCode::PlanOnly], Operation::Explain)
    );
}

#[test]
fn writes_are_classified_by_their_size() {
    let (level, codes, operation) = one(
        "INSERT INTO orders VALUES (1), (2)",
        rows(2, ImpactSource::Statement),
    );
    assert_eq!((level, operation), (R2, Operation::Insert));
    assert_eq!(codes, vec![ReasonCode::SmallWrite]);
    let small = assess_sql(
        "UPDATE orders SET status = 'shipped' WHERE id = 7",
        &[rows(1, ImpactSource::Planner)],
        development(),
    );
    assert_eq!(small.risk, R2);
    assert_eq!(small.required_approval, ApprovalLevel::Apply);
    assert_eq!(small.statements[0].targets, vec!["orders".to_owned()]);
    assert_eq!(
        small.statements[0].reasons[0].message,
        "Writes about 1 row (estimated)."
    );

    let bulk = one(
        "UPDATE orders SET status = 'x' WHERE placed_on < '2026-01-01'",
        rows(SMALL_WRITE_ROWS + 1, ImpactSource::Planner),
    );
    assert_eq!((bulk.0, bulk.1), (R3, vec![ReasonCode::BulkWrite]));
    let unknown = one(
        "UPDATE orders SET status = 'x' WHERE id = 1",
        ImpactFacts::unknown(),
    );
    assert_eq!(
        (unknown.0, unknown.1),
        (R3, vec![ReasonCode::UnknownImpact])
    );
    assert_eq!(
        risk("MERGE INTO orders o USING src s ON o.id = s.id WHEN MATCHED THEN DELETE"),
        R3
    );
    assert_eq!(risk("COPY orders FROM STDIN"), R3);
    assert_eq!(risk("SELECT * FROM orders FOR UPDATE"), R3);
}

#[test]
fn writes_that_change_every_row_are_blocked() {
    for sql in [
        "UPDATE orders SET status = 'x'",
        "DELETE FROM orders",
        "DELETE FROM orders WHERE 1 = 1",
        "DELETE FROM orders WHERE true",
        "WITH gone AS (DELETE FROM orders RETURNING id) SELECT count(*) FROM gone",
    ] {
        let assessment = assess_sql(sql, &[rows(1, ImpactSource::Planner)], development());
        assert_eq!(assessment.risk, R4, "{sql}");
        assert_eq!(
            assessment.required_approval,
            ApprovalLevel::Blocked,
            "{sql}"
        );
        assert!(
            assessment.statements[0]
                .reasons
                .iter()
                .any(|reason| reason.code == ReasonCode::ChangesEveryRow),
            "{sql}"
        );
    }
    // A filter on a column counts, also through a subquery.
    assert_eq!(
        one(
            "DELETE FROM orders WHERE customer_id IN (SELECT id FROM customers WHERE city = 'Faro')",
            rows(3, ImpactSource::Planner)
        )
        .0,
        R2
    );
}

#[test]
fn cascading_deletes_are_high_impact() {
    let cascades = |rows: Option<u64>| ImpactFacts {
        rows,
        source: ImpactSource::Planner,
        cascades_to: vec!["public.order_items".into()],
        missing_privileges: Vec::new(),
    };
    let (level, codes, _) = one("DELETE FROM orders WHERE id = 1", cascades(Some(1)));
    assert_eq!(level, R3);
    assert!(codes.contains(&ReasonCode::CascadingDelete));
    assert_eq!(
        one("DELETE FROM orders WHERE id > 1", cascades(Some(500))).0,
        R4
    );
    assert_eq!(one("DELETE FROM orders WHERE id > 1", cascades(None)).0, R4);
}

#[test]
fn structure_changes_follow_what_they_do() {
    assert_eq!(risk("CREATE TABLE notes (id int PRIMARY KEY)"), R3);
    assert_eq!(risk("ALTER TABLE orders ADD COLUMN note text"), R3);
    assert_eq!(
        risk("ALTER TABLE orders ALTER COLUMN note TYPE varchar(20)"),
        R3
    );
    assert_eq!(risk("COMMENT ON TABLE orders IS 'Orders'"), R2);
    assert_eq!(risk("VACUUM orders"), R3);
    assert_eq!(risk("GRANT SELECT ON orders TO reporting"), R3);

    let (level, codes, _) = one(
        "CREATE INDEX CONCURRENTLY orders_status ON orders (status)",
        ImpactFacts::unknown(),
    );
    assert_eq!(level, R3);
    assert!(codes.contains(&ReasonCode::NonTransactional));

    for sql in [
        "DROP TABLE orders",
        "DROP INDEX orders_status",
        "TRUNCATE orders",
        "ALTER TABLE orders DROP COLUMN note",
        "ALTER TABLE orders DROP CONSTRAINT orders_pkey",
        "ALTER TABLE orders DISABLE TRIGGER ALL",
        "ALTER TABLE orders DISABLE ROW LEVEL SECURITY",
    ] {
        assert_eq!(risk(sql), R4, "{sql}");
    }
}

#[test]
fn server_session_and_unknown_statements_are_blocked() {
    for sql in [
        "CREATE ROLE admin SUPERUSER",
        "ALTER SYSTEM SET work_mem = '1GB'",
        "CREATE EXTENSION dblink",
        "DO $$ BEGIN PERFORM 1; END $$",
        "CALL cleanup()",
        "SET search_path = evil",
        "SET session_replication_role = replica",
        "BEGIN",
        "COMMIT",
        "SELECT pg_terminate_backend(42)",
        "COPY orders FROM '/etc/passwd'",
        "COPY orders TO PROGRAM 'curl example.invalid'",
        "FROBNICATE orders",
    ] {
        let assessment = assess_sql(sql, &[], development());
        assert_eq!(assessment.risk, R4, "{sql}");
        assert_eq!(
            assessment.required_approval,
            ApprovalLevel::Blocked,
            "{sql}"
        );
    }
    let (_, codes, operation) = one("SET search_path = evil", ImpactFacts::unknown());
    assert_eq!(operation, Operation::SessionControl);
    assert!(codes.contains(&ReasonCode::ManagedByAidb));
    let (_, codes, _) = one(
        "SET session_replication_role = replica",
        ImpactFacts::unknown(),
    );
    assert!(codes.contains(&ReasonCode::DisablesSafeguards));
}

#[test]
fn production_raises_every_change() {
    let production = PolicyContext {
        environment: Environment::Production,
        writes_allowed: true,
    };
    let assessment = assess_sql(
        "UPDATE orders SET status = 'x' WHERE id = 1",
        &[rows(1, ImpactSource::Planner)],
        production,
    );
    assert_eq!(assessment.risk, R3);
    assert_eq!(assessment.required_approval, ApprovalLevel::Elevated);
    assert!(
        assessment.statements[0]
            .reasons
            .iter()
            .any(|reason| reason.code == ReasonCode::Production)
    );
    assert_eq!(
        assess_sql("COMMENT ON TABLE orders IS 'x'", &[], production).risk,
        R3
    );
    // Reads are unaffected.
    assert_eq!(assess_sql("SELECT 1", &[], production).risk, R1);
}

#[test]
fn batches_of_changes_are_high_impact() {
    let small = || rows(1, ImpactSource::Planner);
    let assessment = assess_sql(
        "UPDATE a SET x = 1 WHERE id = 1; UPDATE b SET y = 2 WHERE id = 2",
        &[small(), small()],
        development(),
    );
    assert_eq!(
        assessment
            .statements
            .iter()
            .map(|s| s.risk)
            .collect::<Vec<_>>(),
        [R2, R2]
    );
    assert_eq!(assessment.risk, R3);
    assert_eq!(assessment.reasons[0].code, ReasonCode::Batch);
    // Several reads are still reads.
    assert_eq!(
        assess_sql("SELECT 1; SELECT 2", &[], development()).risk,
        R1
    );
}

#[test]
fn permissions_block_without_lowering_risk() {
    let read_only = assess_sql(
        "UPDATE orders SET status = 'x' WHERE id = 1",
        &[rows(1, ImpactSource::Planner)],
        PolicyContext {
            writes_allowed: false,
            ..development()
        },
    );
    assert_eq!(read_only.risk, R2);
    assert_eq!(read_only.required_approval, ApprovalLevel::Blocked);
    assert!(read_only.blockers[0].contains("read-only"));

    let missing = assess_sql(
        "UPDATE orders SET status = 'x' WHERE id = 1",
        &[ImpactFacts {
            missing_privileges: vec!["UPDATE on public.orders".into()],
            ..rows(1, ImpactSource::Planner)
        }],
        development(),
    );
    assert_eq!(missing.required_approval, ApprovalLevel::Blocked);
    assert_eq!(
        missing.blockers,
        vec!["The database role lacks UPDATE on public.orders.".to_owned()]
    );
}

#[test]
fn unreadable_text_is_blocked() {
    assert_eq!(risk("INSERT INTO orders VALUES ("), R4);
    let unterminated = assess_sql("SELECT 'open", &[], development());
    assert_eq!(unterminated.risk, R4);
    assert_eq!(unterminated.reasons[0].code, ReasonCode::Unreadable);
    let empty = assess_sql("  -- nothing\n", &[], development());
    assert_eq!(empty.required_approval, ApprovalLevel::Blocked);
    assert_eq!(
        empty.blockers,
        vec!["There is no statement to run.".to_owned()]
    );
}

#[test]
fn change_sets_are_judged_by_their_rows_in_all() {
    let sql =
        inspect("UPDATE orders SET status = 'x' WHERE id = 1; DELETE FROM orders WHERE id = 2");
    let counted = |a, b| {
        [
            rows(a, ImpactSource::Counted),
            rows(b, ImpactSource::Counted),
        ]
    };

    // Two small edits: as small as one, where SQL text would be a batch (R3).
    let small = assess_change_set(&sql, &counted(1, 1), development());
    assert_eq!(small.risk, R2);
    assert_eq!(small.required_approval, ApprovalLevel::Apply);
    assert!(small.reasons.is_empty());
    assert_eq!(assess(&sql, &counted(1, 1), development()).risk, R3);

    // Small one by one, bulk together.
    let bulk = assess_change_set(&sql, &counted(60, 60), development());
    assert_eq!(bulk.risk, R3);
    assert_eq!(bulk.reasons[0].code, ReasonCode::BulkWrite);
    assert_eq!(
        bulk.reasons[0].message,
        "Writes 120 rows in all, more than 100."
    );

    let unknown = assess_change_set(&sql, &[rows(1, ImpactSource::Counted)], development());
    assert_eq!(unknown.risk, R3);

    // Every other rule still applies to each change.
    let production = assess_change_set(
        &sql,
        &counted(1, 1),
        PolicyContext {
            environment: Environment::Production,
            writes_allowed: true,
        },
    );
    assert_eq!(production.risk, R3);
    let every = assess_change_set(
        &inspect("UPDATE orders SET status = 'x' WHERE id = 1; DELETE FROM orders"),
        &counted(1, 5),
        development(),
    );
    assert_eq!(every.risk, R4);
}

#[test]
fn migrations_may_remove_schema_objects_with_the_elevated_approval() {
    let migration = "ALTER TABLE public.items ADD COLUMN note text;
        ALTER TABLE public.items DROP COLUMN legacy;
        DROP INDEX public.items_legacy_idx;
        DROP TABLE public.old_log";
    let facts = inspect(migration);
    let assessment = assess_migration(&facts, &[], development());
    assert_eq!(assessment.risk, R4);
    assert_eq!(assessment.required_approval, ApprovalLevel::Elevated);
    assert!(assessment.blockers.is_empty(), "{:?}", assessment.blockers);
    assert!(
        !assessment
            .reasons
            .iter()
            .any(|r| r.code == ReasonCode::Batch),
        "no batch rule: each step is generated and shown"
    );
    // The same SQL from the editor or the assistant stays refused.
    assert_eq!(
        assess(&facts, &[], development()).required_approval,
        ApprovalLevel::Blocked
    );
    // Other reasons for R4 are still refused in a migration, and so are read-only connections.
    for refused in [
        "DROP TABLE t; TRUNCATE u",
        "ALTER TABLE t DISABLE TRIGGER ALL",
        "SELECT pg_terminate_backend(1)",
        "DROP TABLE t; this is not sql",
    ] {
        let assessment = assess_migration(&inspect(refused), &[], development());
        assert_eq!(
            assessment.required_approval,
            ApprovalLevel::Blocked,
            "{refused}"
        );
    }
    let read_only = PolicyContext {
        writes_allowed: false,
        ..development()
    };
    assert_eq!(
        assess_migration(&facts, &[], read_only).required_approval,
        ApprovalLevel::Blocked
    );
    // Without removals a migration is an ordinary elevated schema change.
    let additive = assess_migration(
        &inspect("CREATE TABLE t (a int); CREATE INDEX ON t (a)"),
        &[],
        development(),
    );
    assert_eq!(additive.risk, R3);
    assert_eq!(additive.required_approval, ApprovalLevel::Elevated);
}
