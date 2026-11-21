use std::sync::{Arc, PoisonError};
use std::time::Duration;

use super::*;
use crate::adapters::local_store::test_support::temp_store;
use crate::domain::connections::ConnectionDraft;
use crate::services::fakes::{FakeAdapter, sessions, target};

const UPDATE: &str = "UPDATE orders SET status = $1 WHERE id = $2";

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
    let mut draft = ConnectionDraft::new("Shop", "db", "orders", "reader");
    draft.environment = environment;
    draft.read_only = read_only;
    let profile = store
        .connections()
        .insert(&draft.validated().expect("valid"))
        .await
        .expect("insert");
    let (adapter, sessions) = sessions();
    adapter.catalog.set_fingerprint("v1");
    *adapter
        .catalog
        .write_estimate
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(1);
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

fn params(id: &str) -> Vec<Option<String>> {
    vec![Some("shipped".into()), Some(id.into())]
}

impl Setup {
    async fn request(&self, sql: &str) -> AppResult<ApprovalRequest> {
        request(
            &self.store,
            &self.sessions,
            &self.approvals,
            self.id,
            sql,
            params("7"),
            Actor::Agent,
        )
        .await
    }

    async fn approve(
        &self,
        id: ApprovalId,
        confirmation: Option<&str>,
    ) -> AppResult<ApprovalGrant> {
        approve(
            &self.store,
            &self.sessions,
            &self.approvals,
            id,
            confirmation,
        )
        .await
    }

    async fn consume(
        &self,
        token: &SecretString,
        sql: &str,
        parameters: &[Option<String>],
    ) -> AppResult<ApprovedOperation> {
        consume(
            &self.store,
            &self.sessions,
            &self.approvals,
            self.id,
            token,
            sql,
            parameters,
        )
        .await
    }

    /// An approved UPDATE, ready to use.
    async fn granted(&self) -> (ApprovalId, ApprovalGrant) {
        let request = self.request(UPDATE).await.expect("request");
        let grant = self.approve(request.id, None).await.expect("approve");
        (request.id, grant)
    }

    async fn audit(&self) -> Vec<(String, String, Option<String>, Option<String>)> {
        sqlx::query_as(
            "SELECT operation, status, approval_state, detail FROM operation_audit ORDER BY id",
        )
        .fetch_all(self.store.pool())
        .await
        .expect("audit")
    }
}

fn denied(result: AppResult<impl std::fmt::Debug>, contains: &str) {
    let error = result.expect_err("refused");
    assert_eq!(error.code(), ErrorCode::PermissionDenied, "{error}");
    assert!(error.message().contains(contains), "{error}");
}

#[tokio::test]
async fn a_change_is_requested_approved_used_once_and_finished() {
    let setup = setup(Environment::Development, false).await;
    let request = setup.request(UPDATE).await.expect("request");
    assert_eq!(request.level, ApprovalLevel::Apply);
    assert_eq!(request.state, ApprovalState::AwaitingApproval);
    assert_eq!(request.confirmation, None);
    assert!(!request.expires_at.is_empty());

    let grant = setup.approve(request.id, None).await.expect("approve");
    assert_eq!(grant.token.expose_secret().len(), 64);
    assert_eq!(
        setup.approvals.state(request.id),
        Some(ApprovalState::Approved)
    );

    let operation = setup
        .consume(&grant.token, UPDATE, &params("7"))
        .await
        .expect("use");
    assert_eq!(operation.id, request.id);
    assert_eq!(operation.digest.len(), 64);
    assert_eq!(
        setup.approvals.state(request.id),
        Some(ApprovalState::Executing)
    );

    // Used once: the token is gone.
    denied(
        setup.consume(&grant.token, UPDATE, &params("7")).await,
        "no such approval",
    );
    finish(&setup.approvals, request.id, ApprovalState::Verifying).expect("verifying");
    finish(&setup.approvals, request.id, ApprovalState::Succeeded).expect("succeeded");
    assert_eq!(setup.approvals.state(request.id), None);

    let audit = setup.audit().await;
    assert_eq!(audit[0].0, "approve");
    assert_eq!(audit[0].1, "succeeded");
    assert_eq!(audit[1].1, "rejected");
    assert!(
        audit[1]
            .3
            .as_deref()
            .is_some_and(|d| d.starts_with("unknown_approval"))
    );
    // The audit log never holds the SQL or its values.
    for row in &audit {
        let detail = row.3.as_deref().unwrap_or_default();
        assert!(
            !detail.contains("UPDATE") && !detail.contains("shipped"),
            "{detail}"
        );
    }
}

#[tokio::test]
async fn two_uses_at_once_run_one() {
    let setup = setup(Environment::Development, false).await;
    let (_, grant) = setup.granted().await;
    let parameters = params("7");
    let (first, second) = tokio::join!(
        setup.consume(&grant.token, UPDATE, &parameters),
        setup.consume(&grant.token, UPDATE, &parameters),
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
}

#[tokio::test]
async fn any_change_to_what_was_approved_fails_closed() {
    let setup = setup(Environment::Development, false).await;

    let (id, grant) = setup.granted().await;
    denied(
        setup
            .consume(
                &grant.token,
                "UPDATE orders SET status = $1 WHERE id >= $2",
                &params("7"),
            )
            .await,
        "the SQL",
    );
    // Invalidated: the same token no longer works, even with the right SQL.
    assert_eq!(setup.approvals.state(id), None);
    denied(
        setup.consume(&grant.token, UPDATE, &params("7")).await,
        "no such approval",
    );

    let (_, grant) = setup.granted().await;
    denied(
        setup.consume(&grant.token, UPDATE, &params("8")).await,
        "the parameter values",
    );

    let (_, grant) = setup.granted().await;
    setup.adapter.catalog.set_fingerprint("v2");
    denied(
        setup.consume(&grant.token, UPDATE, &params("7")).await,
        "the database's structure",
    );

    let (_, grant) = setup.granted().await;
    *setup
        .adapter
        .catalog
        .write_estimate
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(2);
    denied(
        setup.consume(&grant.token, UPDATE, &params("7")).await,
        "the estimated impact",
    );

    let (_, grant) = setup.granted().await;
    setup
        .sessions
        .open(setup.id, target("db"))
        .await
        .expect("reopen");
    denied(
        setup.consume(&grant.token, UPDATE, &params("7")).await,
        "reopened or switched",
    );

    // Changed between the request and the approval.
    let request = setup.request(UPDATE).await.expect("request");
    setup.adapter.catalog.set_fingerprint("v3");
    denied(
        setup.approve(request.id, None).await,
        "the database's structure",
    );
    assert_eq!(setup.approvals.state(request.id), None);

    let refusals = setup
        .audit()
        .await
        .into_iter()
        .filter(|row| row.1 == "rejected")
        .count();
    assert_eq!(refusals, 7);
}

#[tokio::test]
async fn approvals_belong_to_one_connection() {
    let setup = setup(Environment::Development, false).await;
    let (id, grant) = setup.granted().await;
    let other = consume(
        &setup.store,
        &setup.sessions,
        &setup.approvals,
        ConnectionId::new(),
        &grant.token,
        UPDATE,
        &params("7"),
    )
    .await;
    denied(other, "another connection");
    assert_eq!(setup.approvals.state(id), None);

    // Closing, editing or deleting the connection drops its approvals.
    let (id, _) = setup.granted().await;
    let pending = setup.request(UPDATE).await.expect("request").id;
    assert_eq!(setup.approvals.invalidate_connection(setup.id), 2);
    assert_eq!(setup.approvals.state(id), None);
    assert_eq!(setup.approvals.state(pending), None);

    // Connecting (to this or another connection) drops every open approval.
    let (id, _) = setup.granted().await;
    setup.approvals.invalidate_all();
    assert_eq!(setup.approvals.state(id), None);
}

#[tokio::test]
async fn approvals_expire() {
    let setup = setup(Environment::Development, false).await;
    let request = setup.request(UPDATE).await.expect("request");
    tokio::time::pause();
    tokio::time::advance(PENDING_TTL + Duration::from_secs(1)).await;
    tokio::time::resume();
    denied(setup.approve(request.id, None).await, "expired");
    assert_eq!(setup.approvals.state(request.id), None);

    let (id, grant) = setup.granted().await;
    tokio::time::pause();
    tokio::time::advance(APPROVAL_TTL + Duration::from_secs(1)).await;
    tokio::time::resume();
    denied(
        setup.consume(&grant.token, UPDATE, &params("7")).await,
        "expired",
    );
    assert_eq!(setup.approvals.state(id), None);
}

#[tokio::test]
async fn production_changes_need_the_database_name_typed() {
    let setup = setup(Environment::Production, false).await;
    let request = setup.request(UPDATE).await.expect("request");
    assert_eq!(request.level, ApprovalLevel::Elevated);
    assert_eq!(request.confirmation.as_deref(), Some("orders"));

    for typed in [None, Some("order"), Some("ORDERS")] {
        let error = setup
            .approve(request.id, typed)
            .await
            .expect_err("unconfirmed");
        assert_eq!(error.code(), ErrorCode::InvalidInput);
        // A typo does not end the request.
        assert_eq!(
            setup.approvals.state(request.id),
            Some(ApprovalState::AwaitingApproval)
        );
    }
    setup
        .approve(request.id, Some(" orders "))
        .await
        .expect("confirmed");
}

#[tokio::test]
async fn only_changes_that_may_run_can_be_requested() {
    let setup = setup(Environment::Development, false).await;
    let read = setup
        .request("SELECT * FROM orders")
        .await
        .expect_err("read");
    assert_eq!(read.code(), ErrorCode::InvalidInput);
    denied(setup.request("DROP TABLE orders").await, "cannot run here");
    denied(
        setup.request("UPDATE orders SET status = 'x'").await,
        "cannot run here",
    );

    let read_only = self::setup(Environment::Development, true).await;
    denied(read_only.request(UPDATE).await, "read-only");
}

#[tokio::test]
async fn the_state_machine_guards_every_step() {
    let setup = setup(Environment::Development, false).await;
    let request = setup.request(UPDATE).await.expect("request");
    let grant = setup.approve(request.id, None).await.expect("approve");
    denied(
        setup.approve(request.id, None).await,
        "is approved and cannot be approved",
    );
    denied(
        finish(&setup.approvals, request.id, ApprovalState::Succeeded),
        "cannot be finished",
    );
    cancel(&setup.approvals, request.id).expect("cancel");
    denied(
        setup.consume(&grant.token, UPDATE, &params("7")).await,
        "no such approval",
    );

    let (id, grant) = setup.granted().await;
    setup
        .consume(&grant.token, UPDATE, &params("7"))
        .await
        .expect("use");
    denied(
        cancel(&setup.approvals, id),
        "is executing and cannot be declined",
    );
    // A running change survives invalidation; its outcome is still reported.
    setup.approvals.invalidate_connection(setup.id);
    assert_eq!(setup.approvals.state(id), Some(ApprovalState::Executing));
    denied(
        finish(&setup.approvals, id, ApprovalState::Approved),
        "cannot go from executing to approved",
    );
    finish(&setup.approvals, id, ApprovalState::Failed).expect("failed");
}
