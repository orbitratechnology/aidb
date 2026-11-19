//! Running one read statement from the SQL editor.
//!
//! The statement was classified as a read before it gets here. It is still run defensively: with
//! the extended protocol (the server accepts one command, so nothing can follow a misread
//! statement), in a `READ ONLY` transaction that is always rolled back (which also undoes any
//! session setting it changed), as a cancellable operation, under the session's timeouts.
//!
//! Queries are first prepared as written, which checks syntax and names without running them and
//! reports column types. They then run wrapped:
//!
//! ```sql
//! SELECT <each column as text> FROM (
//! <statement>
//! ) AS aidb_q(c1, c2, ...) LIMIT $1 OFFSET $2
//! ```
//!
//! so every value arrives as text, cut like table pages, and at most one page is fetched.
//! Renaming the columns by position keeps duplicate or unnamed columns apart. Server error
//! positions are shifted back to the statement as written. EXPLAIN and SHOW run as written.

use sqlx::postgres::{PgConnection, PgDatabaseError, PgErrorPosition};
use sqlx::{AssertSqlSafe, Column, Connection, Executor, SqlSafeStr, Statement, TypeInfo};

use super::table_data::{collect, text_expression};
use crate::adapters::database::{DbError, DbErrorKind};
use crate::domain::query::QueryRows;
use crate::domain::sql::ReadShape;
use crate::domain::table_data::{PageColumn, PageLimits, ValueKind};

const ALIAS: &str = "aidb_q";

#[derive(sqlx::FromRow)]
struct TypeRow {
    oid: i64,
    data_type: String,
    category: String,
    type_name: String,
}

/// Runs `sql` (one read statement). Driver errors are turned into [`DbError`]s by `classify`.
pub(super) async fn run_read(
    connection: &mut PgConnection,
    sql: &str,
    shape: ReadShape,
    limits: PageLimits,
    offset: u64,
    classify: impl Fn(sqlx::Error) -> DbError,
) -> Result<QueryRows, DbError> {
    let mut transaction = connection
        .begin_with("BEGIN READ ONLY")
        .await
        .map_err(&classify)?;
    let encoding: String = sqlx::query_scalar("SELECT current_setting('server_encoding')")
        .fetch_one(&mut *transaction)
        .await
        .map_err(&classify)?;
    let raw_bytes = encoding == "SQL_ASCII";

    // Prepared as written: syntax and name errors point into the statement itself.
    let prepared = (&mut *transaction)
        .prepare(AssertSqlSafe(sql.to_owned()).into_sql_str())
        .await
        .map_err(|e| statement_error(e, 0, sql, &classify))?;
    let oids: Vec<i64> = prepared
        .columns()
        .iter()
        .filter_map(|c| c.type_info().oid())
        .map(|oid| i64::from(oid.0))
        .collect();
    let types: Vec<TypeRow> = sqlx::query_as(
        r"SELECT t.oid::int8 AS oid, format_type(t.oid, NULL) AS data_type,
                 bt.typcategory::text AS category, bt.typname::text AS type_name
          FROM pg_type t
          JOIN pg_type bt ON bt.oid = CASE WHEN t.typtype = 'd' THEN t.typbasetype ELSE t.oid END
          WHERE t.oid::int8 = ANY($1)",
    )
    .bind(&oids)
    .fetch_all(&mut *transaction)
    .await
    .map_err(&classify)?;
    let columns: Vec<PageColumn> = prepared
        .columns()
        .iter()
        .map(|column| {
            let info = column.type_info();
            let found = info
                .oid()
                .and_then(|oid| types.iter().find(|t| t.oid == i64::from(oid.0)));
            PageColumn {
                name: column.name().to_owned(),
                data_type: found
                    .map_or_else(|| info.name().to_lowercase(), |t| t.data_type.clone()),
                kind: found.map_or(ValueKind::Other, |t| {
                    ValueKind::classify(&t.category, &t.type_name)
                }),
            }
        })
        .collect();
    drop(prepared);

    let width = columns.len();
    let (statement, prefix_chars) = match shape {
        ReadShape::Query => {
            let select = columns
                .iter()
                .enumerate()
                .map(|(index, column)| {
                    text_expression(
                        &format!("{ALIAS}.c{}", index + 1),
                        column.kind,
                        true,
                        raw_bytes,
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            let names = (1..=width)
                .map(|n| format!("c{n}"))
                .collect::<Vec<_>>()
                .join(", ");
            let aliases = if width == 0 {
                String::new()
            } else {
                format!("({names})")
            };
            let prefix = format!("SELECT {select} FROM (\n");
            let prefix_chars = prefix.chars().count();
            (
                format!("{prefix}{sql}\n) AS {ALIAS}{aliases} LIMIT $1 OFFSET $2"),
                prefix_chars,
            )
        }
        ReadShape::Utility => (sql.to_owned(), 0),
    };

    let mut query = sqlx::query(AssertSqlSafe(statement));
    if shape == ReadShape::Query {
        query = query
            .bind(i64::from(limits.max_rows) + 1)
            .bind(i64::try_from(offset).unwrap_or(i64::MAX));
    }
    let stream = query.fetch(&mut *transaction);
    let collected = collect(stream, width, limits.max_rows, limits.max_bytes)
        .await
        .map_err(|e| statement_error(e, prefix_chars, sql, &classify))?;
    transaction.rollback().await.map_err(&classify)?;

    Ok(QueryRows {
        columns,
        rows: collected.rows,
        truncated_cells: collected.truncated,
        has_more: collected.has_more,
        byte_limit_reached: collected.byte_limit_reached,
    })
}

/// A statement's error with the server's own message and its position within `sql`.
fn statement_error(
    error: sqlx::Error,
    prefix_chars: usize,
    sql: &str,
    classify: &impl Fn(sqlx::Error) -> DbError,
) -> DbError {
    let reported = match &error {
        sqlx::Error::Database(database) => {
            database.try_downcast_ref::<PgDatabaseError>().map(|pg| {
                let mut message = pg.message().trim_end_matches('.').to_owned();
                message.push('.');
                if let Some(hint) = pg.hint() {
                    message.push(' ');
                    message.push_str(hint);
                }
                let position = match pg.position() {
                    Some(PgErrorPosition::Original(at)) => at.checked_sub(prefix_chars),
                    _ => None,
                }
                .filter(|at| (1..=sql.chars().count()).contains(at))
                .and_then(|at| u32::try_from(at).ok());
                (message, position)
            })
        }
        _ => None,
    };
    let classified = classify(error);
    match classified.kind() {
        // aidb's wording is clearer for these than the server's.
        DbErrorKind::Cancelled
        | DbErrorKind::StatementTimeout
        | DbErrorKind::LockTimeout
        | DbErrorKind::ConnectionLost
        | DbErrorKind::ServerUnavailable => classified,
        DbErrorKind::ReadOnly => classified.with_message(
            "The statement tried to change something, which a read-only run does not allow.",
        ),
        _ => match reported {
            Some((message, position)) => classified.with_message(message).with_position(position),
            None => classified,
        },
    }
}
