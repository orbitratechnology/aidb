//! Running one statement from the SQL editor, after classifying it.
//!
//! Only reads run. The SQL text is never logged: only counts, timings and SQLSTATEs are.

use std::time::Instant;

use super::sessions::{Sessions, not_connected};
use super::table_data::page_limits;
use super::{audit, library, settings};
use crate::adapters::database::{DatabaseAdapter, DatabaseSession};
use crate::adapters::local_store::LocalStore;
use crate::domain::audit::{Actor, AuditEvent, AuditStatus};
use crate::domain::connections::{ConnectionId, Environment};
use crate::domain::policy::approval::sha256_hex;
use crate::domain::policy::classify::{self, PolicyContext};
use crate::domain::policy::facts;
use crate::domain::query::{QueryFailure, QueryRefusal, QueryRequest, QueryResult, QueryRun};
use crate::domain::sql::{self, MAX_SQL_BYTES, SqlAnalysis, StatementKind, TextIndex};
use crate::domain::table_data::CELL_CHAR_LIMIT;
use crate::errors::{AppError, AppResult, redact};

/// Splits and classifies SQL for the editor, without touching a database.
///
/// # Errors
///
/// `invalid_input` for text over [`MAX_SQL_BYTES`].
pub fn analyze(sql: &str) -> AppResult<SqlAnalysis> {
    if sql.len() > MAX_SQL_BYTES {
        return Err(AppError::invalid_input(
            "That SQL is too long to check (1 MB at most).",
        ));
    }
    Ok(sql::analyze(sql))
}

/// Runs the one statement in `request.sql` if it is a read; refuses it otherwise.
///
/// # Errors
///
/// `invalid_input` for a malformed request, `unavailable` when the connection is not open,
/// `storage` if the settings cannot be read. Statement errors are returned as
/// [`QueryRun::Failed`], not as errors.
pub async fn run<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    id: ConnectionId,
    request: QueryRequest,
) -> AppResult<QueryRun> {
    let first_page = request.offset == 0;
    let (run, ran) = execute(store, sessions, id, &request, None).await?;
    // Later pages re-run the same statement; history records it once.
    if first_page && !ran.trim().is_empty() {
        library::record(store, id, &ran, &run).await;
    }
    if let QueryRun::Refused(QueryRefusal {
        kind: Some(kind), ..
    }) = &run
        && !matches!(kind, StatementKind::Read | StatementKind::Unknown)
    {
        audit_refusal(store, id, &ran, *kind).await;
    }
    Ok(run)
}

/// Records a statement the editor refused because it is not a read: an attempt to change
/// something outside the approval flow. The audit log gets its digest and risk, not its text.
async fn audit_refusal(store: &LocalStore, id: ConnectionId, sql: &str, kind: StatementKind) {
    let profile = store.connections().get(id).await.ok().flatten();
    let assessment = classify::assess(
        &facts::inspect(sql),
        &[],
        PolicyContext {
            environment: profile.as_ref().map_or(Environment::default(), |profile| {
                profile.settings.environment
            }),
            writes_allowed: profile
                .as_ref()
                .is_some_and(|profile| !profile.settings.read_only),
        },
    );
    audit::record(
        store,
        &AuditEvent {
            connection_id: Some(id),
            environment: profile.map(|profile| profile.settings.environment),
            actor: Actor::User,
            operation: "sql_refused",
            risk: Some(assessment.risk),
            fingerprint: Some(sha256_hex(sql.trim().as_bytes())),
            approval_state: None,
            status: AuditStatus::Rejected,
            rows_affected: None,
            detail: Some(format!(
                "The SQL editor runs reads only; this was a {} statement.",
                match kind {
                    StatementKind::Write => "data-changing",
                    StatementKind::Schema => "structure-changing",
                    StatementKind::Session => "session or transaction",
                    StatementKind::Privileged => "privileged",
                    StatementKind::Read | StatementKind::Unknown => "other",
                }
            )),
        },
    )
    .await;
}

/// Runs one read statement for the assistant: the same checks and read-only transaction as the
/// editor, at most `max_rows` rows (within the Safety limit), and nothing recorded in the
/// editor's history.
///
/// # Errors
///
/// As [`run`].
pub async fn run_for_assistant<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    id: ConnectionId,
    request: QueryRequest,
    max_rows: u32,
) -> AppResult<QueryRun> {
    Ok(execute(store, sessions, id, &request, Some(max_rows))
        .await?
        .0)
}

/// Runs the request; returns the outcome and the text it is about (the one statement, or the
/// whole submission when it was refused before one could be picked).
async fn execute<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    id: ConnectionId,
    request: &QueryRequest,
    max_rows: Option<u32>,
) -> AppResult<(QueryRun, String)> {
    request.validate()?;
    let session = sessions.get(id).ok_or_else(not_connected)?;
    let analysis = sql::analyze(&request.sql);
    if let Some(problem) = analysis.problem {
        let refusal = QueryRun::Refused(QueryRefusal {
            message: problem.message,
            kind: None,
            position: problem.position,
        });
        return Ok((refusal, request.sql.clone()));
    }
    let statement = match analysis.statements.as_slice() {
        [] => {
            let refusal = QueryRun::Refused(QueryRefusal {
                message: "There is no statement to run.".into(),
                kind: None,
                position: None,
            });
            return Ok((refusal, String::new()));
        }
        [one] => one,
        [_, second, ..] => {
            let refusal = QueryRun::Refused(QueryRefusal {
                message: format!(
                    "This holds {} statements. Run one at a time: select it, or place the cursor \
                     in it.",
                    analysis.statements.len()
                ),
                kind: None,
                position: Some(second.span.start),
            });
            return Ok((refusal, request.sql.clone()));
        }
    };
    let (Some(shape), None) = (statement.shape, &statement.reason) else {
        let refusal = QueryRun::Refused(QueryRefusal {
            message: statement.reason.clone().unwrap_or_default(),
            kind: Some(statement.kind),
            position: statement.problem_at.or(Some(statement.span.start)),
        });
        return Ok((refusal, request.sql.clone()));
    };

    let mut limits = page_limits(&settings::load(store).await?.safety);
    if let Some(max_rows) = max_rows {
        limits.max_rows = limits.max_rows.min(max_rows);
    }
    let started = Instant::now();
    let outcome = session
        .run_read(
            &statement.text,
            shape,
            limits,
            request.offset,
            request.operation_id,
        )
        .await;
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match outcome {
        Ok(rows) => {
            tracing::info!(connection = %id, rows = rows.rows.len(), elapsed_ms, "query ran");
            let run = QueryRun::Completed(QueryResult {
                statement: statement.span,
                columns: rows.columns,
                rows: rows.rows,
                truncated_cells: rows.truncated_cells,
                offset: request.offset,
                has_more: rows.has_more,
                byte_limit_reached: rows.byte_limit_reached,
                cell_char_limit: CELL_CHAR_LIMIT,
                elapsed_ms,
            });
            Ok((run, statement.text.clone()))
        }
        Err(error) => {
            let index = TextIndex::new(&request.sql);
            let position = error.position().map(|at| {
                let at = usize::try_from(at).unwrap_or(usize::MAX).saturating_sub(1);
                index.position_of_char(statement.start_char.saturating_add(at))
            });
            let message = redact(error.message()).into_owned();
            let sqlstate = error.sqlstate().map(str::to_owned);
            tracing::info!(connection = %id, sqlstate = ?sqlstate, elapsed_ms, "query failed");
            let run = QueryRun::Failed(QueryFailure {
                code: AppError::from(error).code(),
                message,
                sqlstate,
                position,
                statement: statement.span,
            });
            Ok((run, statement.text.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::local_store::test_support::temp_store;
    use crate::domain::sql::StatementKind;
    use crate::errors::ErrorCode;
    use crate::services::fakes::{sessions, target};

    fn request(sql: &str) -> QueryRequest {
        QueryRequest {
            sql: sql.into(),
            offset: 0,
            operation_id: None,
        }
    }

    async fn open() -> (
        tempfile::TempDir,
        LocalStore,
        Sessions<std::sync::Arc<crate::services::fakes::FakeAdapter>>,
        ConnectionId,
    ) {
        let (dir, store) = temp_store().await;
        let (_, sessions) = sessions();
        let id = ConnectionId::new();
        sessions.open(id, target("db")).await.expect("open");
        (dir, store, sessions, id)
    }

    #[tokio::test]
    async fn one_read_runs_and_reports_where_it_was() {
        let (_dir, store, sessions, id) = open().await;

        let run = run(&store, &sessions, id, request("\n  SELECT 1;  -- done"))
            .await
            .expect("run");

        let QueryRun::Completed(result) = run else {
            panic!("expected rows: {run:?}");
        };
        assert_eq!(
            result.rows,
            [[Some("8".to_owned())]],
            "only the statement ran"
        );
        assert_eq!(
            (result.statement.start.line, result.statement.start.column),
            (2, 3)
        );
    }

    #[tokio::test]
    async fn writes_and_batches_are_refused_before_reaching_the_database() {
        let (_dir, store, sessions, id) = open().await;

        let write = run(&store, &sessions, id, request("DELETE FROM t"))
            .await
            .expect("run");
        let batch = run(&store, &sessions, id, request("SELECT 1; SELECT 2"))
            .await
            .expect("run");
        let empty = run(&store, &sessions, id, request(" -- nothing"))
            .await
            .expect("run");
        let broken = run(&store, &sessions, id, request("SELECT 'oops"))
            .await
            .expect("run");

        let QueryRun::Refused(write) = write else {
            panic!("{write:?}")
        };
        assert_eq!(write.kind, Some(StatementKind::Write));
        let QueryRun::Refused(batch) = batch else {
            panic!("{batch:?}")
        };
        assert!(batch.message.contains("2 statements"));
        assert_eq!(batch.position.map(|p| p.column), Some(11));
        assert!(matches!(empty, QueryRun::Refused(_)));
        assert!(matches!(broken, QueryRun::Refused(r) if r.position.is_some()));
    }

    #[tokio::test]
    async fn database_errors_point_into_the_submitted_text() {
        let (_dir, store, sessions, id) = open().await;

        let run = run(
            &store,
            &sessions,
            id,
            request("-- first line\nSELECT boom FROM t"),
        )
        .await
        .expect("run");

        let QueryRun::Failed(failure) = run else {
            panic!("{run:?}")
        };
        assert_eq!(failure.code, ErrorCode::InvalidInput);
        assert_eq!(failure.sqlstate.as_deref(), Some("42703"));
        let at = failure.position.expect("position");
        assert_eq!((at.line, at.column), (2, 8), "character 8 of the statement");
    }

    #[tokio::test]
    async fn first_runs_are_recorded_in_history_with_secrets_redacted() {
        let (_dir, store) = temp_store().await;
        let profile = store
            .connections()
            .insert(
                &crate::domain::connections::ConnectionDraft::new("Orders", "db", "orders", "me")
                    .validated()
                    .expect("valid"),
            )
            .await
            .expect("insert");
        let (_, sessions) = sessions();
        sessions.open(profile.id, target("db")).await.expect("open");

        run(&store, &sessions, profile.id, request("SELECT 1"))
            .await
            .expect("read");
        run(
            &store,
            &sessions,
            profile.id,
            request("ALTER ROLE app PASSWORD 'hunter2'"),
        )
        .await
        .expect("refused");
        let mut second_page = request("SELECT 1");
        second_page.offset = 500;
        run(&store, &sessions, profile.id, second_page)
            .await
            .expect("page");

        let history = store
            .query_history()
            .list(profile.id, None, 50)
            .await
            .expect("history");
        let statuses: Vec<_> = history.iter().map(|e| e.status).collect();
        assert_eq!(
            statuses,
            [
                crate::domain::library::HistoryStatus::Refused,
                crate::domain::library::HistoryStatus::Completed
            ],
            "later pages are not recorded again"
        );
        assert_eq!(history[0].sql, "ALTER ROLE app PASSWORD '[redacted]'");
        assert_eq!(history[1].row_count, Some(1));
    }

    #[tokio::test]
    async fn closed_connections_and_huge_text_are_errors() {
        let (_dir, store) = temp_store().await;
        let (_, sessions) = sessions();

        let closed = run(&store, &sessions, ConnectionId::new(), request("SELECT 1"))
            .await
            .expect_err("closed");
        let huge = analyze(&"x".repeat(MAX_SQL_BYTES + 1)).expect_err("huge");

        assert_eq!(closed.code(), ErrorCode::Unavailable);
        assert_eq!(huge.code(), ErrorCode::InvalidInput);
    }

    #[tokio::test]
    async fn refused_changes_are_audited_by_fingerprint() {
        use crate::domain::audit::{AuditFilter, AuditStatus};
        use crate::domain::connections::ConnectionDraft;
        use crate::domain::policy::risk::RiskLevel;

        let (_dir, store, sessions, _) = open().await;
        let profile = store
            .connections()
            .insert(
                &ConnectionDraft::new("Shop", "db", "orders", "reader")
                    .validated()
                    .expect("valid"),
            )
            .await
            .expect("profile");
        sessions.open(profile.id, target("db")).await.expect("open");
        let sql = "DELETE FROM orders WHERE note = 'secret-value'";
        let run = super::run(&store, &sessions, profile.id, request(sql))
            .await
            .expect("refusal");
        assert!(matches!(run, QueryRun::Refused(_)));
        // Reads and statements aidb cannot read are not audited.
        super::run(&store, &sessions, profile.id, request("SELECT 1"))
            .await
            .expect("read");
        super::run(&store, &sessions, profile.id, request("FROBNICATE"))
            .await
            .expect("unknown");

        let events = store
            .audit()
            .list(&AuditFilter {
                connection_id: Some(profile.id),
                before_id: None,
                limit: 10,
            })
            .await
            .expect("audit");
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.operation, "sql_refused");
        assert_eq!(event.status, AuditStatus::Rejected);
        assert_eq!(event.risk, Some(RiskLevel::R3));
        assert_eq!(
            event.fingerprint.as_deref(),
            Some(sha256_hex(sql.as_bytes()).as_str())
        );
        let detail = event.detail.as_deref().unwrap_or_default();
        assert!(
            !detail.contains("secret-value") && !detail.contains("DELETE"),
            "{detail}"
        );
    }
}
