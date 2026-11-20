//! The query advisor: one read statement planned (or run and measured, read-only) on the open
//! connection, with suggestions drawn from the plan and the tables' statistics.

use super::sessions::{Sessions, not_connected};
use crate::adapters::database::{DatabaseAdapter, DatabaseSession};
use crate::domain::advisor::{ExplainRequest, QueryPlan, advise};
use crate::domain::connections::ConnectionId;
use crate::domain::sql::{self, MAX_SQL_BYTES, ReadShape, StatementKind};
use crate::errors::{AppError, AppResult};

/// The one query in `sql`, as it will be explained.
///
/// # Errors
///
/// `invalid_input` for anything but exactly one query (SELECT, VALUES, TABLE or WITH).
pub fn the_query(sql: &str) -> AppResult<String> {
    if sql.len() > MAX_SQL_BYTES {
        return Err(AppError::invalid_input(format!(
            "The SQL is too long ({} MB at most).",
            MAX_SQL_BYTES / (1024 * 1024)
        )));
    }
    let analysis = sql::analyze(sql);
    if let Some(problem) = analysis.problem {
        return Err(AppError::invalid_input(problem.message));
    }
    match analysis.statements.as_slice() {
        [statement]
            if statement.kind == StatementKind::Read
                && statement.shape == Some(ReadShape::Query) =>
        {
            Ok(statement.text.clone())
        }
        [statement] => Err(AppError::invalid_input(
            statement.reason.clone().unwrap_or_else(|| {
                "Explain a query: SELECT, VALUES, TABLE or WITH, without EXPLAIN.".to_owned()
            }),
        )),
        [] => Err(AppError::invalid_input("There is no query to explain.")),
        _ => Err(AppError::invalid_input(
            "Explain one statement at a time: select it, or put the cursor in it.",
        )),
    }
}

/// Plans `request.sql` on the open connection (with `analyze`, also runs it, read-only and
/// rolled back) and advises on the plan.
///
/// # Errors
///
/// `invalid_input` for anything but one query; `unavailable` when not connected; the
/// database's error (a statement timeout, a cancel) otherwise.
pub async fn explain<A: DatabaseAdapter>(
    sessions: &Sessions<A>,
    connection_id: ConnectionId,
    request: &ExplainRequest,
) -> AppResult<QueryPlan> {
    let sql = the_query(&request.sql)?;
    let session = sessions.get(connection_id).ok_or_else(not_connected)?;
    let (output, tables) = session
        .explain_read(
            &sql,
            request.analyze,
            request.operation_id.unwrap_or_default(),
        )
        .await?;
    advise(&output, &tables).map_err(AppError::internal)
}

#[cfg(test)]
mod tests;
