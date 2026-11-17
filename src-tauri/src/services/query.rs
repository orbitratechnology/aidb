//! Running one statement from the SQL editor, after classifying it.
//!
//! Only reads run. The SQL text is never logged: only counts, timings and SQLSTATEs are.

use std::time::Instant;

use super::sessions::{Sessions, not_connected};
use super::settings;
use super::table_data::page_limits;
use crate::adapters::database::{DatabaseAdapter, DatabaseSession};
use crate::adapters::local_store::LocalStore;
use crate::domain::connections::ConnectionId;
use crate::domain::query::{QueryFailure, QueryRefusal, QueryRequest, QueryResult, QueryRun};
use crate::domain::sql::{self, MAX_SQL_BYTES, SqlAnalysis, TextIndex};
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
    request.validate()?;
    let session = sessions.get(id).ok_or_else(not_connected)?;
    let analysis = sql::analyze(&request.sql);
    if let Some(problem) = analysis.problem {
        return Ok(QueryRun::Refused(QueryRefusal {
            message: problem.message,
            kind: None,
            position: problem.position,
        }));
    }
    let statement = match analysis.statements.as_slice() {
        [] => {
            return Ok(QueryRun::Refused(QueryRefusal {
                message: "There is no statement to run.".into(),
                kind: None,
                position: None,
            }));
        }
        [one] => one,
        [_, second, ..] => {
            return Ok(QueryRun::Refused(QueryRefusal {
                message: format!(
                    "This holds {} statements. Run one at a time: select it, or place the cursor \
                     in it.",
                    analysis.statements.len()
                ),
                kind: None,
                position: Some(second.span.start),
            }));
        }
    };
    let (Some(shape), None) = (statement.shape, &statement.reason) else {
        return Ok(QueryRun::Refused(QueryRefusal {
            message: statement.reason.clone().unwrap_or_default(),
            kind: Some(statement.kind),
            position: statement.problem_at.or(Some(statement.span.start)),
        }));
    };

    let limits = page_limits(&settings::load(store).await?.safety);
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
            Ok(QueryRun::Completed(QueryResult {
                statement: statement.span,
                columns: rows.columns,
                rows: rows.rows,
                truncated_cells: rows.truncated_cells,
                offset: request.offset,
                has_more: rows.has_more,
                byte_limit_reached: rows.byte_limit_reached,
                cell_char_limit: CELL_CHAR_LIMIT,
                elapsed_ms,
            }))
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
            Ok(QueryRun::Failed(QueryFailure {
                code: AppError::from(error).code(),
                message,
                sqlstate,
                position,
                statement: statement.span,
            }))
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
}
