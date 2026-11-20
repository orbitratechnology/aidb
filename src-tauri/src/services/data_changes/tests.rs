use std::sync::{Arc, PoisonError};

use super::*;
use crate::adapters::local_store::test_support::temp_store;
use crate::domain::catalog::ObjectName;
use crate::domain::connections::{ConnectionDraft, Environment};
use crate::domain::data_change::ColumnValue;
use crate::domain::policy::risk::{ApprovalLevel, RiskLevel};
use crate::domain::table_data::{Filter, FilterOperator};
use crate::services::fakes::{FakeAdapter, sessions, target};

struct Setup {
    _dir: tempfile::TempDir,
    store: LocalStore,
    adapter: Arc<FakeAdapter>,
    sessions: Sessions<Arc<FakeAdapter>>,
    approvals: Approvals,
    id: ConnectionId,
}

async fn setup(environment: Environment, read_only: bool) -> Setup {
    let (dir, store) = temp_store().await;
    let mut draft = ConnectionDraft::new("Shop", "db", "orders", "writer");
    draft.environment = environment;
    draft.read_only = read_only;
    let profile = store
        .connections()
        .insert(&draft.validated().expect("valid"))
        .await
        .expect("insert");
    let (adapter, sessions) = sessions();
    adapter.catalog.set_fingerprint("v1");
    adapter.catalog.set_matched_rows(2);
    sessions.open(profile.id, target("db")).await.expect("open");
    Setup {
        _dir: dir,
        store,
        adapter,
        sessions,
        approvals: Approvals::default(),
        id: profile.id,
    }
}

fn orders() -> ObjectName {
    ObjectName {
        schema: "public".into(),
        name: "orders".into(),
    }
}

fn ship() -> DataChange {
    DataChange::Update {
        table: orders(),
        set: vec![ColumnValue {
            column: "status".into(),
            value: Some("shipped".into()),
        }],
        filters: vec![Filter {
            column: "customer".into(),
            operator: FilterOperator::Equals,
            value: Some("7".into()),
        }],
    }
}

impl Setup {
    async fn preview(&self, change: DataChange) -> AppResult<DataChangePreview> {
        self.preview_set(vec![change]).await
    }

    async fn preview_set(&self, changes: Vec<DataChange>) -> AppResult<DataChangePreview> {
        preview(
            &self.store,
            &self.sessions,
            &self.approvals,
            self.id,
            changes,
            Actor::User,
        )
        .await
    }

    async fn apply(
        &self,
        id: ApprovalId,
        confirmation: Option<&str>,
    ) -> AppResult<DataChangeResult> {
        apply(
            &self.store,
            &self.sessions,
            &self.approvals,
            self.id,
            id,
            confirmation,
        )
        .await
    }

    fn applied(&self) -> Vec<DataChange> {
        self.adapter
            .catalog
            .applied
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    async fn audit(&self) -> Vec<(String, String, Option<String>, Option<i64>, Option<String>)> {
        sqlx::query_as(
            "SELECT operation, status, approval_state, rows_affected, detail
             FROM operation_audit ORDER BY id",
        )
        .fetch_all(self.store.pool())
        .await
        .expect("audit")
    }
}

#[tokio::test]
async fn a_previewed_change_is_applied_once_and_audited() {
    let setup = setup(Environment::Development, false).await;
    let preview = setup.preview(ship()).await.expect("preview");
    let change = &preview.changes[0];
    assert_eq!(change.kind, ChangeKind::Update);
    assert_eq!(preview.rows, 2);
    assert_eq!(preview.risk, RiskLevel::R2);
    assert_eq!(preview.approval, ApprovalLevel::Apply);
    assert_eq!(preview.confirmation, None);
    assert!(change.before.is_some(), "matching rows are shown");
    let after = change.after.as_ref().expect("values are shown");
    assert_eq!(after.columns, vec!["status"]);
    assert_eq!(after.rows, vec![vec![Some("shipped".to_owned())]]);
    assert_eq!(preview.reasons, vec!["Writes 2 rows (counted).".to_owned()]);
    assert!(setup.applied().is_empty(), "previews write nothing");

    let result = setup
        .apply(preview.approval_id, None)
        .await
        .expect("applies");
    assert_eq!(result.status, ApplyStatus::Applied);
    assert_eq!(result.rows_affected, Some(2));
    assert_eq!(setup.applied(), vec![ship()]);
    assert_eq!(setup.approvals.state(preview.approval_id), None);

    // Used up: a second apply finds nothing, and nothing runs again.
    let again = setup
        .apply(preview.approval_id, None)
        .await
        .expect_err("used");
    assert_eq!(again.code(), ErrorCode::PermissionDenied);
    assert_eq!(setup.applied().len(), 1);

    let audit = setup.audit().await;
    assert_eq!(audit.len(), 2, "{audit:?}");
    assert_eq!(audit[0].0, "approve");
    assert_eq!(
        audit[1],
        (
            "data_change".into(),
            "succeeded".into(),
            Some("succeeded".into()),
            Some(2),
            Some("Update of public.orders: applied.".into())
        )
    );
    // The values never reach the audit log.
    assert!(!format!("{audit:?}").contains("shipped"));
}

#[tokio::test]
async fn rows_that_changed_since_the_preview_invalidate_it() {
    let setup = setup(Environment::Development, false).await;
    let preview = setup.preview(ship()).await.expect("preview");
    setup.adapter.catalog.set_matched_rows(3);
    let error = setup
        .apply(preview.approval_id, None)
        .await
        .expect_err("the impact changed");
    assert_eq!(error.code(), ErrorCode::PermissionDenied);
    assert!(error.message().contains("estimated impact"), "{error}");
    assert!(setup.applied().is_empty());
    assert_eq!(setup.approvals.state(preview.approval_id), None);
}

#[tokio::test]
async fn production_changes_need_the_database_name_typed() {
    let setup = setup(Environment::Production, false).await;
    let preview = setup.preview(ship()).await.expect("preview");
    assert_eq!(preview.risk, RiskLevel::R3);
    assert_eq!(preview.approval, ApprovalLevel::Elevated);
    assert_eq!(preview.confirmation.as_deref(), Some("orders"));

    let missing = setup
        .apply(preview.approval_id, None)
        .await
        .expect_err("needs the name");
    assert_eq!(missing.code(), ErrorCode::InvalidInput);
    let wrong = setup
        .apply(preview.approval_id, Some("order"))
        .await
        .expect_err("wrong name");
    assert_eq!(wrong.code(), ErrorCode::InvalidInput);
    assert!(setup.applied().is_empty());

    let result = setup
        .apply(preview.approval_id, Some("orders"))
        .await
        .expect("applies");
    assert_eq!(result.status, ApplyStatus::Applied);
}

#[tokio::test]
async fn read_only_connections_and_unfiltered_changes_are_refused() {
    let setup = setup(Environment::Development, true).await;
    let error = setup.preview(ship()).await.expect_err("read-only");
    assert_eq!(error.code(), ErrorCode::PermissionDenied);
    assert!(error.message().contains("read-only"), "{error}");
    let audit = setup.audit().await;
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].0, "data_change");
    assert_eq!(audit[0].1, "rejected");

    let unfiltered = DataChange::Delete {
        table: orders(),
        filters: Vec::new(),
    };
    let error = setup.preview(unfiltered).await.expect_err("no filter");
    assert_eq!(error.code(), ErrorCode::InvalidInput);
    assert!(error.message().contains("at least one filter"));
    assert!(setup.approvals.data_changes(ApprovalId::new()).is_none());
}

#[tokio::test]
async fn approvals_belong_to_their_connection_and_kind() {
    let setup = setup(Environment::Development, false).await;
    let preview = setup.preview(ship()).await.expect("preview");
    let other = apply(
        &setup.store,
        &setup.sessions,
        &setup.approvals,
        ConnectionId::new(),
        preview.approval_id,
        None,
    )
    .await
    .expect_err("another connection");
    assert_eq!(other.code(), ErrorCode::PermissionDenied);
    assert!(setup.applied().is_empty());

    // An approval for SQL text cannot be applied as a data change.
    *setup
        .adapter
        .catalog
        .write_estimate
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(1);
    let sql = approvals::request(
        &setup.store,
        &setup.sessions,
        &setup.approvals,
        setup.id,
        "UPDATE orders SET status = 'x' WHERE id = 1",
        Vec::new(),
        Actor::Agent,
    )
    .await
    .expect("sql request");
    let error = setup
        .apply(sql.id, None)
        .await
        .expect_err("not a data change");
    assert_eq!(error.code(), ErrorCode::PermissionDenied);
    let error = cancel(&setup.store, &setup.approvals, sql.id)
        .await
        .expect_err("not a data change");
    assert_eq!(error.code(), ErrorCode::PermissionDenied);
    assert!(setup.approvals.state(sql.id).is_some(), "left alone");
}

#[tokio::test]
async fn failed_and_uncertain_outcomes_are_reported_and_audited() {
    let setup = setup(Environment::Development, false).await;
    let preview = setup.preview(ship()).await.expect("preview");
    *setup
        .adapter
        .catalog
        .apply_failure
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(false);
    let result = setup.apply(preview.approval_id, None).await.expect("ran");
    assert_eq!(result.status, ApplyStatus::RolledBack);
    assert_eq!(result.rows_affected, None);
    assert!(
        result
            .message
            .as_deref()
            .is_some_and(|m| m.contains("unique constraint"))
    );

    let preview = setup.preview(ship()).await.expect("preview");
    *setup
        .adapter
        .catalog
        .apply_failure
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(true);
    let result = setup.apply(preview.approval_id, None).await.expect("ran");
    assert_eq!(result.status, ApplyStatus::Uncertain);
    assert!(
        result
            .message
            .as_deref()
            .is_some_and(|m| m.contains("may or may not"))
    );

    let audit: Vec<_> = setup
        .audit()
        .await
        .into_iter()
        .filter(|event| event.0 == "data_change")
        .collect();
    assert_eq!(audit.len(), 2);
    assert!(audit.iter().all(|event| event.1 == "failed"));
    assert_eq!(
        audit[1].4.as_deref(),
        Some("Update of public.orders: uncertain, the connection was lost while committing.")
    );
}

#[tokio::test]
async fn declined_changes_are_dropped_and_recorded() {
    let setup = setup(Environment::Development, false).await;
    let preview = setup.preview(ship()).await.expect("preview");
    cancel(&setup.store, &setup.approvals, preview.approval_id)
        .await
        .expect("cancel");
    assert_eq!(setup.approvals.state(preview.approval_id), None);
    let error = setup
        .apply(preview.approval_id, None)
        .await
        .expect_err("declined");
    assert_eq!(error.code(), ErrorCode::PermissionDenied);
    let audit = setup.audit().await;
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].1, "cancelled");
    assert_eq!(
        audit[0].4.as_deref(),
        Some("Update of public.orders: declined.")
    );
}

#[tokio::test]
async fn inserts_count_their_rows_and_show_their_values() {
    let setup = setup(Environment::Development, false).await;
    let rows: Vec<Vec<Option<String>>> =
        (0..150).map(|i| vec![Some(i.to_string()), None]).collect();
    let insert = DataChange::Insert {
        table: orders(),
        columns: vec!["id".into(), "note".into()],
        rows,
    };
    let preview = setup.preview(insert).await.expect("preview");
    assert_eq!(preview.rows, 150);
    assert_eq!(preview.risk, RiskLevel::R3, "more than 100 rows is bulk");
    let change = &preview.changes[0];
    assert!(change.before.is_none());
    let after = change.after.as_ref().expect("values");
    assert_eq!(after.total_rows, 150);
    assert_eq!(after.rows.len(), 20);
}

#[tokio::test]
async fn sets_of_changes_are_approved_and_applied_together() {
    let setup = setup(Environment::Development, false).await;
    setup.adapter.catalog.set_matched_rows(1);
    let delete = DataChange::Delete {
        table: orders(),
        filters: vec![Filter {
            column: "id".into(),
            operator: FilterOperator::Equals,
            value: Some("9".into()),
        }],
    };
    let preview = setup
        .preview_set(vec![ship(), delete.clone()])
        .await
        .expect("preview");
    assert_eq!(preview.changes.len(), 2);
    assert_eq!(preview.rows, 2);
    // Two one-row edits: a small write, not a batch of SQL statements.
    assert_eq!(preview.risk, RiskLevel::R2);
    assert_eq!(preview.approval, ApprovalLevel::Apply);
    assert_eq!(preview.reasons, vec!["Writes 1 row (counted).".to_owned()]);
    assert_eq!(preview.changes[1].kind, ChangeKind::Delete);
    assert!(preview.changes[1].after.is_none());

    let result = setup
        .apply(preview.approval_id, None)
        .await
        .expect("applies");
    assert_eq!(result.rows_affected, Some(2));
    assert_eq!(setup.applied(), vec![ship(), delete.clone()]);
    let audit = setup.audit().await;
    assert_eq!(
        audit.last().and_then(|event| event.4.clone()),
        Some("2 changes to public.orders: 1 update, 1 delete: applied.".to_owned())
    );

    // The set is checked as a whole: one bad change names itself and nothing is held.
    let error = setup
        .preview_set(vec![
            ship(),
            DataChange::Delete {
                table: orders(),
                filters: Vec::new(),
            },
        ])
        .await
        .expect_err("no filter");
    assert_eq!(error.code(), ErrorCode::InvalidInput);
    assert!(error.message().starts_with("Change 2 of 2: "), "{error}");
    let empty = setup.preview_set(Vec::new()).await.expect_err("empty");
    assert_eq!(empty.code(), ErrorCode::InvalidInput);
}

#[tokio::test]
async fn approving_for_another_connection_invalidates_the_approval() {
    let setup = setup(Environment::Development, false).await;
    let preview = setup.preview(ship()).await.expect("preview");
    let error = apply(
        &setup.store,
        &setup.sessions,
        &setup.approvals,
        ConnectionId::new(),
        preview.approval_id,
        None,
    )
    .await
    .expect_err("another connection");
    assert_eq!(error.code(), ErrorCode::PermissionDenied);
    assert_eq!(setup.approvals.state(preview.approval_id), None);
    assert!(setup.applied().is_empty());
}
