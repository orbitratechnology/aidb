//! Applying a schema migration: one transaction for the statements that can run in one, after
//! checking the catalog is still as approved; then the others, one by one.

use sqlx::{AssertSqlSafe, Connection, PgConnection};

use super::catalog::FINGERPRINT;
use crate::adapters::database::{DbError, DbErrorKind, MigrationRun, MigrationStatement};
use crate::domain::schema_diff::normalize::quote_ident;

/// Whether an error leaves a commit's outcome unknown: no answer from the server.
fn lost(error: &DbError) -> bool {
    error.sqlstate().is_none()
        || matches!(
            error.kind(),
            DbErrorKind::ConnectionLost | DbErrorKind::ServerUnavailable
        )
}

pub(super) async fn apply(
    connection: &mut PgConnection,
    schema: &str,
    statements: &[MigrationStatement],
    fingerprint: &str,
    classify: impl Fn(sqlx::Error) -> DbError,
) -> MigrationRun {
    let rolled_back = |step, error| MigrationRun::RolledBack { step, error };
    // A plain BEGIN: a read-only session keeps refusing writes.
    let mut transaction = match connection.begin().await {
        Ok(transaction) => transaction,
        Err(error) => return rolled_back(None, classify(error)),
    };
    // Before the search path changes: the fingerprint prints names as the approval saw them.
    match sqlx::query_scalar::<_, String>(FINGERPRINT)
        .fetch_one(&mut *transaction)
        .await
    {
        Ok(current) if current == fingerprint => {}
        Ok(_) => {
            return rolled_back(
                None,
                DbError::new(
                    DbErrorKind::InvalidRequest,
                    "The database's structure changed after the migration was approved. Compare \
                     again.",
                ),
            );
        }
        Err(error) => return rolled_back(None, classify(error)),
    }
    let search_path = format!("SET LOCAL search_path TO {}", quote_ident(schema));
    if let Err(error) = sqlx::query(AssertSqlSafe(search_path))
        .execute(&mut *transaction)
        .await
    {
        return rolled_back(None, classify(error));
    }
    let mut applied = 0;
    for (index, statement) in statements.iter().enumerate() {
        if !statement.transactional {
            continue;
        }
        // The simple protocol, one statement at a time: DDL takes no parameters.
        if let Err(error) = sqlx::raw_sql(AssertSqlSafe(statement.sql.clone()))
            .execute(&mut *transaction)
            .await
        {
            return rolled_back(Some(index), classify(error));
        }
        applied += 1;
    }
    if let Err(error) = transaction.commit().await {
        let error = classify(error);
        return if lost(&error) {
            MigrationRun::Uncertain(error)
        } else {
            rolled_back(None, error)
        };
    }

    let outside: Vec<(usize, &MigrationStatement)> = statements
        .iter()
        .enumerate()
        .filter(|(_, statement)| !statement.transactional)
        .collect();
    if outside.is_empty() {
        return MigrationRun::Applied;
    }
    let search_path = format!("SET search_path TO {}", quote_ident(schema));
    if let Err(error) = sqlx::raw_sql(AssertSqlSafe(search_path))
        .execute(&mut *connection)
        .await
    {
        return MigrationRun::Partial {
            applied,
            step: outside[0].0,
            error: classify(error),
        };
    }
    let mut result = MigrationRun::Applied;
    for (index, statement) in outside {
        if let Err(error) = sqlx::raw_sql(AssertSqlSafe(statement.sql.clone()))
            .execute(&mut *connection)
            .await
        {
            result = MigrationRun::Partial {
                applied,
                step: index,
                error: classify(error),
            };
            break;
        }
        applied += 1;
    }
    // The connection goes back to the pool: give it back its usual search path.
    let _ = sqlx::raw_sql("RESET search_path")
        .execute(&mut *connection)
        .await;
    result
}
