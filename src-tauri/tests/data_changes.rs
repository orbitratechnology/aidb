//! Data changes end to end: previewed, approved and applied through the app's services on a
//! real session on the PostgreSQL fixture, so statements, counts, type conversion, locking and
//! verification all run against the database.
//!
//! `scripts/postgres-fixture.sh start && source target/postgres-fixture/env`
//! `cargo test -p aidb --test data_changes -- --ignored`

mod support;

use aidb_lib::adapters::database::{ApplyError, DatabaseSession, DbErrorKind};
use aidb_lib::domain::audit::{Actor, AuditFilter};
use aidb_lib::domain::catalog::ObjectName;
use aidb_lib::domain::connections::Environment;
use aidb_lib::domain::data_change::{
    ApplyStatus, ColumnValue, DataChange, DataChangePreview, DataChangeResult, PREVIEW_BYTES,
    PREVIEW_ROWS,
};
use aidb_lib::domain::database::OperationId;
use aidb_lib::domain::policy::risk::{ApprovalLevel, RiskLevel};
use aidb_lib::domain::table_data::{Filter, FilterOperator, PageLimits};
use aidb_lib::errors::{AppResult, ErrorCode};
use aidb_lib::services::approvals::Approvals;
use aidb_lib::services::{audit, data_changes};
use support::{ScratchDatabase, Workspace};

const SCHEMA: &str = r"
CREATE TABLE items (
    id int PRIMARY KEY,
    code varchar(10) NOT NULL UNIQUE,
    label char(5),
    price numeric(10, 2),
    flags bit(3),
    tags int[],
    doc jsonb,
    raw json,
    double_price numeric GENERATED ALWAYS AS (price * 2) STORED,
    serial int GENERATED ALWAYS AS IDENTITY,
    status text NOT NULL DEFAULT 'new'
);
INSERT INTO items (id, code, price, status) VALUES
    (1, 'A1', 10, 'new'), (2, 'A2', 20, 'new'), (3, 'B1', 30, 'old');
CREATE VIEW cheap_items AS SELECT id, code, price FROM items WHERE price < 25;

CREATE TABLE notes (id int PRIMARY KEY, body text NOT NULL);
CREATE FUNCTION shout() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN NEW.body := upper(NEW.body); RETURN NEW; END $$;
CREATE TRIGGER shout BEFORE UPDATE ON notes FOR EACH ROW EXECUTE FUNCTION shout();
INSERT INTO notes VALUES (1, 'quiet');
";

fn table(name: &str) -> ObjectName {
    ObjectName {
        schema: "public".into(),
        name: name.into(),
    }
}

fn filter(column: &str, operator: FilterOperator, value: Option<&str>) -> Filter {
    Filter {
        column: column.into(),
        operator,
        value: value.map(Into::into),
    }
}

fn set(column: &str, value: Option<&str>) -> ColumnValue {
    ColumnValue {
        column: column.into(),
        value: value.map(Into::into),
    }
}

fn row(values: &[Option<&str>]) -> Vec<Option<String>> {
    values.iter().map(|v| v.map(Into::into)).collect()
}

struct Changes {
    workspace: Workspace,
    approvals: Approvals,
}

impl Changes {
    async fn open(database: &ScratchDatabase, environment: Environment, writable: bool) -> Self {
        let workspace = Workspace::connect_with(&database.name, |settings| {
            settings.environment = environment;
            settings.read_only = !writable;
        })
        .await;
        Self {
            workspace,
            approvals: Approvals::default(),
        }
    }

    async fn preview(&self, change: DataChange) -> AppResult<DataChangePreview> {
        data_changes::preview(
            &self.workspace.store,
            &self.workspace.sessions,
            &self.approvals,
            self.workspace.connection_id,
            change,
            Actor::User,
        )
        .await
    }

    async fn apply(&self, preview: &DataChangePreview) -> AppResult<DataChangeResult> {
        data_changes::apply(
            &self.workspace.store,
            &self.workspace.sessions,
            &self.approvals,
            self.workspace.connection_id,
            preview.approval_id,
            preview.confirmation.as_deref(),
        )
        .await
    }

    /// One text value from fixed test SQL, read on the workspace's own session.
    async fn read(&self, sql: &str) -> String {
        let session = self
            .workspace
            .sessions
            .get(self.workspace.connection_id)
            .expect("open");
        let mut connection = session
            .begin_operation(OperationId::new())
            .await
            .expect("operation");
        sqlx::query_scalar::<_, String>(sqlx::AssertSqlSafe(sql.to_owned()))
            .fetch_one(&mut *connection)
            .await
            .expect("read")
    }

    /// Audit events, oldest first: operation, status, rows written.
    async fn audit(&self) -> Vec<(String, &'static str, Option<u64>)> {
        let filter = AuditFilter {
            connection_id: Some(self.workspace.connection_id),
            before_id: None,
            limit: 100,
        };
        let mut events: Vec<_> = audit::list(&self.workspace.store, filter)
            .await
            .expect("audit")
            .into_iter()
            .map(|event| (event.operation, event.status.as_str(), event.rows_affected))
            .collect();
        events.reverse();
        events
    }
}

#[tokio::test]
#[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
async fn changes_are_previewed_applied_and_verified() {
    let database = ScratchDatabase::create("aidb_changes", SCHEMA).await;
    let changes = Changes::open(&database, Environment::Development, true).await;

    // UPDATE: counted, shown before and after, and nothing written by the preview.
    let before = database.snapshot().await;
    let update = changes
        .preview(DataChange::Update {
            table: table("items"),
            set: vec![set("status", Some("sold")), set("price", Some("12.345"))],
            filters: vec![filter("status", FilterOperator::Equals, Some("new"))],
        })
        .await
        .expect("preview");
    assert_eq!(database.snapshot().await, before, "previews write nothing");
    assert_eq!(update.rows, 2);
    assert_eq!(update.risk, RiskLevel::R2);
    assert_eq!(update.approval, ApprovalLevel::Apply);
    assert_eq!(update.reasons, vec!["Writes 2 rows (counted).".to_owned()]);
    assert!(
        update.statement.starts_with(
            "UPDATE public.items SET status = CAST($1 AS text), price = CAST($2 AS numeric) \
             WHERE status = CAST($3 AS pg_catalog.text) RETURNING"
        ),
        "{}",
        update.statement
    );
    let page = update.before.as_ref().expect("matching rows");
    assert_eq!(page.rows.len(), 2);
    // The stored value, as the column's scale rounds it.
    let after = update.after.as_ref().expect("values");
    assert_eq!(after.rows, vec![row(&[Some("sold"), Some("12.35")])]);

    let result = changes.apply(&update).await.expect("applies");
    assert_eq!(result.status, ApplyStatus::Applied);
    assert_eq!(result.rows_affected, Some(2));
    assert_eq!(
        changes
            .read(
                "SELECT string_agg(id || ':' || status || ':' || price, ',' ORDER BY id) FROM items"
            )
            .await,
        "1:sold:12.35,2:sold:12.35,3:old:30.00"
    );

    // INSERT: every type converted by the database, stored values checked.
    let insert = changes
        .preview(DataChange::Insert {
            table: table("items"),
            columns: ["id", "code", "label", "flags", "tags", "doc", "raw"]
                .map(String::from)
                .to_vec(),
            rows: vec![
                row(&[
                    Some("4"),
                    Some("C1"),
                    Some("ab"),
                    Some("101"),
                    Some("{3,1}"),
                    Some(r#"{"b": 1,   "a": [true]}"#),
                    Some(r#"{"b": 1,   "a": 2}"#),
                ]),
                row(&[Some("5"), Some("C2"), None, None, None, None, None]),
            ],
        })
        .await
        .expect("preview");
    assert_eq!(insert.rows, 2);
    assert!(insert.before.is_none());
    let after = insert.after.as_ref().expect("values");
    assert_eq!(
        after.rows[0],
        row(&[
            Some("4"),
            Some("C1"),
            // As the grid shows char(n): text drops the padding.
            Some("ab"),
            Some("101"),
            Some("[3,1]"),
            Some(r#"{"a": [true], "b": 1}"#),
            Some(r#"{"b": 1,   "a": 2}"#),
        ])
    );
    let result = changes.apply(&insert).await.expect("applies");
    assert_eq!(result.rows_affected, Some(2));
    assert_eq!(
        changes
            .read("SELECT string_agg(id || ':' || status || ':' || serial, ',' ORDER BY id) FROM items WHERE id > 3")
            .await,
        "4:new:4,5:new:5",
        "left-out columns get their defaults"
    );

    // DELETE.
    let delete = changes
        .preview(DataChange::Delete {
            table: table("items"),
            filters: vec![filter("id", FilterOperator::GreaterOrEqual, Some("4"))],
        })
        .await
        .expect("preview");
    assert_eq!(delete.rows, 2);
    assert!(delete.after.is_none());
    let result = changes.apply(&delete).await.expect("applies");
    assert_eq!(result.rows_affected, Some(2));
    assert_eq!(changes.read("SELECT count(*)::text FROM items").await, "3");

    let audit = changes.audit().await;
    let applied: Vec<_> = audit
        .iter()
        .filter(|event| event.0 == "data_change")
        .map(|event| (event.1, event.2))
        .collect();
    assert_eq!(
        applied,
        vec![
            ("succeeded", Some(2)),
            ("succeeded", Some(2)),
            ("succeeded", Some(2))
        ]
    );

    changes.workspace.close().await;
    database.drop().await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
async fn values_and_columns_are_checked_before_approval() {
    let database = ScratchDatabase::create("aidb_changes", SCHEMA).await;
    let changes = Changes::open(&database, Environment::Development, true).await;
    let before = database.snapshot().await;
    let update = |column: &str, value: &str| DataChange::Update {
        table: table("items"),
        set: vec![set(column, Some(value))],
        filters: vec![filter("id", FilterOperator::Equals, Some("1"))],
    };

    let cases = [
        // An explicit cast would cut these silently; the column refuses them.
        (update("code", "ABCDEFGHIJK"), "too long"),
        (update("flags", "1010"), "too long"),
        (update("price", "lots"), "invalid input syntax"),
        (update("tags", "{1,x}"), "invalid input syntax"),
        (update("double_price", "1"), "generated column"),
        (update("serial", "1"), "identity column"),
        (update("nope", "1"), "has no column"),
        (
            DataChange::Delete {
                table: table("cheap_items"),
                filters: vec![filter("id", FilterOperator::Equals, Some("1"))],
            },
            "not a table",
        ),
        (
            DataChange::Delete {
                table: table("items"),
                filters: vec![filter("id", FilterOperator::Equals, Some("one"))],
            },
            "invalid input syntax",
        ),
    ];
    for (change, text) in cases {
        let error = changes.preview(change).await.expect_err(text);
        assert_eq!(error.code(), ErrorCode::InvalidInput, "{text}: {error}");
        assert!(error.message().contains(text), "{text}: {error}");
    }
    // Trailing spaces are dropped by assignment, as PostgreSQL does, so they fit.
    let padded = changes
        .preview(update("code", "ABCDEFGHIJ   "))
        .await
        .expect("fits");
    assert_eq!(
        padded.after.as_ref().map(|a| a.rows.clone()),
        Some(vec![row(&[Some("ABCDEFGHIJ")])])
    );
    let missing = changes
        .preview(DataChange::Delete {
            table: table("ghost"),
            filters: vec![filter("id", FilterOperator::Equals, Some("1"))],
        })
        .await
        .expect_err("no such table");
    assert_eq!(missing.code(), ErrorCode::NotFound);

    // A constraint the database checks when writing: rolled back, nothing written.
    let duplicate = changes
        .preview(update("code", "A2"))
        .await
        .expect("previews");
    let result = changes.apply(&duplicate).await.expect("ran");
    assert_eq!(result.status, ApplyStatus::RolledBack);
    assert!(
        result
            .message
            .as_deref()
            .is_some_and(|m| m.contains("unique constraint")),
        "{result:?}"
    );

    assert_eq!(database.snapshot().await, before);
    changes.workspace.close().await;
    database.drop().await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
async fn changes_since_the_preview_are_never_applied() {
    let database = ScratchDatabase::create("aidb_changes", SCHEMA).await;
    let changes = Changes::open(&database, Environment::Development, true).await;
    let retire = DataChange::Update {
        table: table("items"),
        set: vec![set("status", Some("retired"))],
        filters: vec![filter("status", FilterOperator::Equals, Some("new"))],
    };

    // Another writer adds a matching row after the preview: the approval no longer holds.
    let preview = changes.preview(retire.clone()).await.expect("preview");
    database
        .execute("INSERT INTO items (id, code, status) VALUES (9, 'Z9', 'new')")
        .await;
    let error = changes.apply(&preview).await.expect_err("stale");
    assert_eq!(error.code(), ErrorCode::PermissionDenied);
    assert!(error.message().contains("estimated impact"), "{error}");
    assert_eq!(
        changes
            .read("SELECT count(*)::text FROM items WHERE status = 'retired'")
            .await,
        "0"
    );

    // A structure change after the preview is caught too.
    let preview = changes.preview(retire.clone()).await.expect("preview");
    database
        .execute("ALTER TABLE items ADD COLUMN note text")
        .await;
    let error = changes.apply(&preview).await.expect_err("stale");
    assert_eq!(error.code(), ErrorCode::PermissionDenied);
    assert!(error.message().contains("structure"), "{error}");

    // Between the last check and the write, the engine itself compares with the approved plan.
    let session = changes
        .workspace
        .sessions
        .get(changes.workspace.connection_id)
        .expect("open");
    let plan = session.plan_data_change(&retire, None).await.expect("plan");
    database
        .execute("UPDATE items SET status = 'new' WHERE id = 3")
        .await;
    let before = database.snapshot().await;
    let error = session
        .apply_data_change(&retire, &plan)
        .await
        .expect_err("stale plan");
    match error {
        ApplyError::RolledBack(error) => {
            assert!(error.message().contains("now match"), "{error}");
        }
        ApplyError::Uncertain(error) => panic!("not uncertain: {error}"),
    }
    assert_eq!(database.snapshot().await, before);

    // A trigger that rewrites the values: verification rolls the write back.
    let shout = DataChange::Update {
        table: table("notes"),
        set: vec![set("body", Some("hello"))],
        filters: vec![filter("id", FilterOperator::Equals, Some("1"))],
    };
    let preview = changes.preview(shout).await.expect("preview");
    let result = changes.apply(&preview).await.expect("ran");
    assert_eq!(result.status, ApplyStatus::RolledBack);
    assert!(
        result
            .message
            .as_deref()
            .is_some_and(|m| m.contains("trigger")),
        "{result:?}"
    );
    assert_eq!(changes.read("SELECT body FROM notes").await, "quiet");

    changes.workspace.close().await;
    database.drop().await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
async fn read_only_and_production_connections_are_guarded() {
    let database = ScratchDatabase::create("aidb_changes", SCHEMA).await;
    let before = database.snapshot().await;
    let change = DataChange::Update {
        table: table("items"),
        set: vec![set("status", Some("sold"))],
        filters: vec![filter("id", FilterOperator::Equals, Some("1"))],
    };

    // New connections are read-only: the policy blocks the change, and the session itself
    // refuses it even when asked directly.
    let reader = Changes::open(&database, Environment::Development, false).await;
    let error = reader.preview(change.clone()).await.expect_err("read-only");
    assert_eq!(error.code(), ErrorCode::PermissionDenied);
    assert!(error.message().contains("read-only"), "{error}");
    let session = reader
        .workspace
        .sessions
        .get(reader.workspace.connection_id)
        .expect("open");
    let plan = session
        .plan_data_change(
            &change,
            Some(PageLimits {
                max_rows: PREVIEW_ROWS,
                max_bytes: PREVIEW_BYTES,
            }),
        )
        .await
        .expect("planning only reads");
    match session.apply_data_change(&change, &plan).await {
        Err(ApplyError::RolledBack(error)) => assert_eq!(error.kind(), DbErrorKind::ReadOnly),
        other => panic!("the session must refuse: {other:?}"),
    }
    reader.workspace.close().await;

    // Production: elevated, and the database name must be typed.
    let production = Changes::open(&database, Environment::Production, true).await;
    let preview = production.preview(change).await.expect("preview");
    assert_eq!(preview.risk, RiskLevel::R3);
    assert_eq!(preview.approval, ApprovalLevel::Elevated);
    assert_eq!(
        preview.confirmation.as_deref(),
        Some(database.name.as_str())
    );
    let unconfirmed = data_changes::apply(
        &production.workspace.store,
        &production.workspace.sessions,
        &production.approvals,
        production.workspace.connection_id,
        preview.approval_id,
        None,
    )
    .await
    .expect_err("needs the name");
    assert_eq!(unconfirmed.code(), ErrorCode::InvalidInput);
    assert_eq!(database.snapshot().await, before);
    let result = production.apply(&preview).await.expect("applies");
    assert_eq!(result.status, ApplyStatus::Applied);
    production.workspace.close().await;

    database.drop().await;
}
