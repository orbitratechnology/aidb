use super::*;
use crate::adapters::credentials::Credentials;
use crate::adapters::local_store::test_support::temp_store;
use crate::domain::audit::AuditFilter;
use crate::domain::connections::ConnectionDraft;
use crate::domain::policy::risk::{ApprovalLevel, RiskLevel};
use crate::domain::schema_diff::{CompareRequest, CompareSource, GenerateOptions};
use crate::errors::ErrorCode;
use crate::services::fakes::{FakeAdapter, sessions, target};
use crate::services::schema_diff::{self, SchemaDiffs};

struct Setup {
    _dir: tempfile::TempDir,
    store: LocalStore,
    sessions: Sessions<Arc<FakeAdapter>>,
    approvals: Approvals,
    diffs: SchemaDiffs,
    connection_id: ConnectionId,
}

async fn setup(read_only: bool) -> Setup {
    let (dir, store) = temp_store().await;
    let mut draft = ConnectionDraft::new("Shop", "db", "orders", "owner");
    draft.read_only = read_only;
    let profile = store
        .connections()
        .insert(&draft.validated().expect("valid"))
        .await
        .expect("insert");
    let (_adapter, sessions) = sessions();
    let mut connect = target("db");
    connect.limits.read_only = read_only;
    sessions.open(profile.id, connect).await.expect("open");
    Setup {
        _dir: dir,
        store,
        sessions,
        approvals: Approvals::default(),
        diffs: SchemaDiffs::default(),
        connection_id: profile.id,
    }
}

impl Setup {
    /// Compares the fake's `public` (a `customers` table) with a file.
    async fn compare(&self, file: &str) -> SchemaDiff {
        let opened =
            schema_diff::open_file(&self.diffs, "wanted.sql".into(), file.as_bytes().to_vec())
                .expect("reads");
        schema_diff::compare(
            &self.store,
            &Credentials::session_only("tests"),
            &self.sessions,
            &self.diffs,
            self.connection_id,
            CompareRequest {
                target_schema: "public".into(),
                source: CompareSource::File {
                    file_id: opened.file_id,
                    schema: "public".into(),
                },
                options: GenerateOptions::default(),
            },
            None,
        )
        .await
        .expect("compares")
    }

    async fn prepare(&self, diff: &SchemaDiff) -> AppResult<MigrationPreview> {
        prepare(
            &self.store,
            &self.sessions,
            &self.approvals,
            &self.diffs,
            self.connection_id,
            diff.diff_id,
        )
        .await
    }

    async fn apply(
        &self,
        preview: &MigrationPreview,
        confirmation: &str,
        accept: bool,
    ) -> AppResult<MigrationResult> {
        apply(
            &self.store,
            &self.sessions,
            &self.approvals,
            &self.diffs,
            self.connection_id,
            Decision {
                approval_id: preview.approval_id,
                confirmation: Some(confirmation),
                accept_data_loss: accept,
            },
        )
        .await
    }
}

const ADDITIONS: &str = "CREATE TABLE customers (id integer NOT NULL, email text);
    CREATE TABLE orders (id bigint PRIMARY KEY);";

#[tokio::test]
async fn an_approved_migration_runs_is_checked_and_recorded() {
    let setup = setup(false).await;
    let diff = setup.compare(ADDITIONS).await;
    assert_eq!(diff.steps.len(), 2);
    let preview = setup.prepare(&diff).await.expect("prepares");
    assert_eq!(preview.steps, 2);
    assert!(preview.data_loss.is_empty());
    assert_eq!(preview.risk, RiskLevel::R3);
    assert_eq!(preview.approval, ApprovalLevel::Elevated);
    assert_eq!(preview.confirmation.as_deref(), Some("orders"));
    assert_eq!(preview.reasons, ["Changes the database's structure."]);

    // The wrong name leaves it waiting; the right one runs it.
    let wrong = setup
        .apply(&preview, "shop", false)
        .await
        .expect_err("wrong name");
    assert_eq!(wrong.code(), ErrorCode::InvalidInput);
    let result = setup
        .apply(&preview, "orders", false)
        .await
        .expect("applies");
    assert_eq!(result.status, MigrationStatus::Succeeded);
    assert_eq!(result.applied_steps, 2);
    // The fake's schema does not change, so comparing again still finds both differences.
    assert_eq!(result.verification, Verification::Failed);
    assert_eq!(result.remaining, 2);
    assert!(
        result.message.contains("2 things still differ"),
        "{}",
        result.message
    );

    let records = list(&setup.store, Some(setup.connection_id))
        .await
        .expect("lists");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].status, MigrationStatus::Succeeded);
    assert_eq!(records[0].applied_steps, 2);
    assert_eq!(records[0].source, "wanted.sql (public)");
    assert_eq!(records[0].script_sha256, sha256_hex(diff.script.as_bytes()));
    let events = setup
        .store
        .audit()
        .list(&AuditFilter {
            connection_id: Some(setup.connection_id),
            before_id: None,
            limit: 10,
        })
        .await
        .expect("audit");
    let applied = events
        .iter()
        .find(|event| event.operation == "migration")
        .expect("audited");
    assert!(
        applied
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("succeeded (2 of 2 steps)")),
        "{applied:?}"
    );
    // Applied comparisons are gone: the schema they describe changed.
    assert_eq!(
        setup.prepare(&diff).await.expect_err("gone").code(),
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn steps_that_delete_data_must_be_acknowledged() {
    let setup = setup(false).await;
    let diff = setup.compare("CREATE TABLE other (a int);").await;
    let preview = setup.prepare(&diff).await.expect("prepares");
    assert_eq!(preview.data_loss, ["Drop table customers"]);
    assert_eq!(preview.risk, RiskLevel::R4);
    assert_eq!(preview.approval, ApprovalLevel::Elevated, "not refused");
    let refused = setup
        .apply(&preview, "orders", false)
        .await
        .expect_err("not accepted");
    assert_eq!(refused.code(), ErrorCode::InvalidInput);
    let result = setup
        .apply(&preview, "orders", true)
        .await
        .expect("applies");
    assert_eq!(result.status, MigrationStatus::Succeeded);
}

#[tokio::test]
async fn a_failed_step_rolls_back_and_the_comparison_stays() {
    let setup = setup(false).await;
    let diff = setup
        .compare("CREATE TABLE customers (id integer NOT NULL); CREATE TABLE fail (a int);")
        .await;
    let preview = setup.prepare(&diff).await.expect("prepares");
    let result = setup.apply(&preview, "orders", false).await.expect("ends");
    assert_eq!(result.status, MigrationStatus::Failed);
    assert_eq!(result.applied_steps, 0);
    assert_eq!(result.verification, Verification::NotRun);
    let failed = result.failed_step.expect("names the step");
    assert_eq!(failed.number, 1);
    assert_eq!(failed.summary, "Create table fail");
    assert!(
        result.message.contains("nothing changed"),
        "{}",
        result.message
    );
    let records = list(&setup.store, None).await.expect("lists");
    assert_eq!(records[0].status, MigrationStatus::Failed);
    // Nothing changed, so the comparison can be applied again (with a new approval).
    let again = setup.prepare(&diff).await.expect("prepares again");
    cancel(&setup.approvals, again.approval_id).expect("declines");
    assert!(cancel(&setup.approvals, again.approval_id).is_err());
}

#[tokio::test]
async fn read_only_connections_refuse_migrations() {
    let setup = setup(true).await;
    let diff = setup.compare(ADDITIONS).await;
    let refused = setup.prepare(&diff).await.expect_err("read-only");
    assert_eq!(refused.code(), ErrorCode::PermissionDenied);
    assert!(
        refused.message().contains("read-only"),
        "{}",
        refused.message()
    );
}
