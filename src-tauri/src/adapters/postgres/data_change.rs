//! INSERT, UPDATE and DELETE statements written from a [`DataChange`], planned without writing
//! and applied in one checked transaction.
//!
//! Statements are built like table pages (see `table_data`): from fixed SQL fragments and the
//! names and types as the server prints them for the live table. Names in the change are only
//! compared with that list, and every value is a bound parameter cast to its column's type.
//!
//! Values are cast to the column's type without its modifier, so storing them applies the
//! assignment rules: a string longer than `varchar(n)` is refused rather than cut, as an
//! explicit cast would do. To show and check the stored value beforehand, each value is also
//! converted with the modifier, and a string or bit value that the modifier would shorten is
//! refused at planning time.
//!
//! Applying re-plans the change inside the write transaction, after locking the table against
//! structure changes and the matching rows against other writers. Anything that differs from
//! the approved plan (statement, parameters, matching rows, stored values), a different number
//! of rows written, or stored values other than the planned ones (a trigger changed them) rolls
//! the transaction back.

use std::collections::HashMap;

use sqlx::postgres::{PgArguments, PgConnection};
use sqlx::query::Query;
use sqlx::{AssertSqlSafe, Connection, Postgres};

use super::table_data::{self, ColumnRow, Target, conditions, decode_cell, text_expression};
use crate::adapters::database::{ApplyError, DbError, DbErrorKind};
use crate::domain::catalog::ObjectName;
use crate::domain::data_change::{ChangeKind, DataChange, DataChangePlan, MAX_CHANGED_ROWS};

/// The statements for one change, written from the live table.
#[derive(Debug, PartialEq, Eq)]
struct Built {
    /// The change itself, as approved and run.
    statement: String,
    parameters: Vec<Option<String>>,
    /// Counts the rows an UPDATE or DELETE matches, with its parameters. `FOR UPDATE` is
    /// appended inside the subquery to lock them.
    count: Option<(String, Vec<String>)>,
    /// Converts the values to their columns' types (INSERT, UPDATE).
    normalize: Option<Normalize>,
    value_columns: Vec<String>,
    /// Rows an INSERT adds.
    insert_rows: u64,
}

/// A SELECT that returns each value as stored, as text, then one check per column whose type
/// modifier could shorten a string or bit value.
#[derive(Debug, PartialEq, Eq)]
struct Normalize {
    sql: String,
    parameters: Vec<Option<String>>,
    width: usize,
    /// Indexes into the value columns, in the order of the checks after them.
    checked: Vec<usize>,
}

fn invalid(message: String) -> DbError {
    DbError::new(DbErrorKind::InvalidRequest, message)
}

/// The column `name`, if this change may write it.
fn writable_column<'c>(
    target: &'c Target,
    object: &ObjectName,
    name: &str,
    kind: ChangeKind,
) -> Result<&'c ColumnRow, DbError> {
    let column = target
        .columns
        .iter()
        .find(|c| c.name == name)
        .ok_or_else(|| {
            invalid(format!(
                "{}.{} has no column {name}.",
                object.schema, object.name
            ))
        })?;
    if column.generated {
        return Err(invalid(format!(
            "{name} is a generated column; the database computes its value."
        )));
    }
    if column.always_identity {
        return Err(invalid(format!(
            "{name} is an identity column that the database always fills."
        )));
    }
    let allowed = match kind {
        ChangeKind::Insert => column.can_insert,
        _ => column.can_update,
    };
    if !allowed {
        let action = if kind == ChangeKind::Insert {
            "insert into"
        } else {
            "update"
        };
        return Err(DbError::new(
            DbErrorKind::PermissionDenied,
            format!("Your database role cannot {action} the column {name}."),
        ));
    }
    if !column.readable {
        return Err(DbError::new(
            DbErrorKind::PermissionDenied,
            format!("Your database role cannot read the column {name}, so aidb cannot check it."),
        ));
    }
    Ok(column)
}

/// Text of a column's value as stored, the same way for planned values and written ones.
fn stored_text(expression: &str, column: &ColumnRow, raw_bytes: bool) -> String {
    text_expression(expression, column.kind(), false, raw_bytes)
}

/// Whether a value converted without and with the column's modifier must match: strings
/// (ignoring trailing spaces, which assignment drops silently) and bit strings.
fn modifier_check(column: &ColumnRow, value: &str) -> Option<String> {
    if !column.has_modifier {
        return None;
    }
    let plain = format!("CAST({value} AS {})::text", column.plain_type);
    let full = format!("CAST({value} AS {})::text", column.data_type);
    match column.category.as_str() {
        "S" => Some(format!(
            "({value} IS NULL OR rtrim({plain}) = rtrim({full}))::text"
        )),
        "V" => Some(format!("({value} IS NULL OR {plain} = {full})::text")),
        _ => None,
    }
}

fn normalize(columns: &[&ColumnRow], rows: &[Vec<Option<String>>], raw_bytes: bool) -> Normalize {
    let width = columns.len();
    let mut parameters = Vec::with_capacity(rows.len() * width);
    let mut tuples = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let mut tuple = vec![(index + 1).to_string()];
        for value in row {
            parameters.push(value.clone());
            tuple.push(format!("${}::text", parameters.len()));
        }
        tuples.push(format!("({})", tuple.join(", ")));
    }
    let names: Vec<String> = (0..width).map(|index| format!("c{index}")).collect();
    let mut select: Vec<String> = columns
        .iter()
        .zip(&names)
        .map(|(column, name)| {
            stored_text(
                &format!("CAST(v.{name} AS {})", column.data_type),
                column,
                raw_bytes,
            )
        })
        .collect();
    let mut checked = Vec::new();
    for (index, (column, name)) in columns.iter().zip(&names).enumerate() {
        if let Some(check) = modifier_check(column, &format!("v.{name}")) {
            select.push(check);
            checked.push(index);
        }
    }
    let sql = format!(
        "SELECT {} FROM (VALUES {}) AS v(n, {}) ORDER BY v.n",
        select.join(", "),
        tuples.join(", "),
        names.join(", ")
    );
    Normalize {
        sql,
        parameters,
        width,
        checked,
    }
}

fn build(change: &DataChange, target: &Target) -> Result<Built, DbError> {
    let object = change.table();
    let relation = &target.relation;
    if !relation.table {
        return Err(invalid(format!(
            "{}.{} is not a table. aidb changes rows of tables only.",
            object.schema, object.name
        )));
    }
    let raw_bytes = relation.raw_bytes();
    let kind = change.kind();
    let returning = |columns: &[&ColumnRow]| {
        columns
            .iter()
            .map(|column| stored_text(&column.quoted, column, raw_bytes))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let count = |filters| -> Result<(String, Vec<String>), DbError> {
        let mut values = Vec::new();
        let conditions = conditions(filters, &target.columns, object, &mut values, 0)?;
        Ok((
            format!(
                "SELECT count(*) FROM (SELECT 1 FROM {} WHERE {} LIMIT {}",
                relation.quoted,
                conditions.join(" AND "),
                MAX_CHANGED_ROWS + 1
            ),
            values,
        ))
    };

    match change {
        DataChange::Insert { columns, rows, .. } => {
            let columns = columns
                .iter()
                .map(|name| writable_column(target, object, name, kind))
                .collect::<Result<Vec<_>, _>>()?;
            let width = columns.len();
            let tuples: Vec<String> = (0..rows.len())
                .map(|row| {
                    let casts: Vec<String> = columns
                        .iter()
                        .enumerate()
                        .map(|(index, column)| {
                            format!(
                                "CAST(${} AS {})",
                                row * width + index + 1,
                                column.plain_type
                            )
                        })
                        .collect();
                    format!("({})", casts.join(", "))
                })
                .collect();
            let statement = format!(
                "INSERT INTO {} ({}) VALUES {} RETURNING {}",
                relation.quoted,
                columns
                    .iter()
                    .map(|c| c.quoted.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                tuples.join(", "),
                returning(&columns)
            );
            Ok(Built {
                statement,
                parameters: rows.iter().flatten().cloned().collect(),
                count: None,
                normalize: Some(normalize(&columns, rows, raw_bytes)),
                value_columns: columns.iter().map(|c| c.name.clone()).collect(),
                insert_rows: rows.len() as u64,
            })
        }
        DataChange::Update { set, filters, .. } => {
            let columns = set
                .iter()
                .map(|value| writable_column(target, object, &value.column, kind))
                .collect::<Result<Vec<_>, _>>()?;
            let assignments: Vec<String> = columns
                .iter()
                .enumerate()
                .map(|(index, column)| {
                    format!(
                        "{} = CAST(${} AS {})",
                        column.quoted,
                        index + 1,
                        column.plain_type
                    )
                })
                .collect();
            let mut filter_values = Vec::new();
            let predicate = conditions(
                filters,
                &target.columns,
                object,
                &mut filter_values,
                set.len(),
            )?;
            let statement = format!(
                "UPDATE {} SET {} WHERE {} RETURNING {}",
                relation.quoted,
                assignments.join(", "),
                predicate.join(" AND "),
                returning(&columns)
            );
            let values: Vec<Option<String>> = set.iter().map(|v| v.value.clone()).collect();
            let mut parameters = values.clone();
            parameters.extend(filter_values.into_iter().map(Some));
            Ok(Built {
                statement,
                parameters,
                count: Some(count(filters)?),
                normalize: Some(normalize(&columns, &[values], raw_bytes)),
                value_columns: columns.iter().map(|c| c.name.clone()).collect(),
                insert_rows: 0,
            })
        }
        DataChange::Delete { filters, .. } => {
            let mut values = Vec::new();
            let predicate = conditions(filters, &target.columns, object, &mut values, 0)?;
            Ok(Built {
                statement: format!(
                    "DELETE FROM {} WHERE {}",
                    relation.quoted,
                    predicate.join(" AND ")
                ),
                parameters: values.into_iter().map(Some).collect(),
                count: Some(count(filters)?),
                normalize: None,
                value_columns: Vec::new(),
                insert_rows: 0,
            })
        }
    }
}

fn bind_all<'q>(
    mut query: Query<'q, Postgres, PgArguments>,
    values: &'q [Option<String>],
) -> Query<'q, Postgres, PgArguments> {
    for value in values {
        query = query.bind(value.as_deref());
    }
    query
}

/// Counts the matching rows, up to one more than [`MAX_CHANGED_ROWS`], locking them when
/// `lock`.
async fn count_rows(
    connection: &mut PgConnection,
    (sql, values): &(String, Vec<String>),
    lock: bool,
) -> Result<u64, sqlx::Error> {
    let sql = format!("{sql}{}) AS matched", if lock { " FOR UPDATE" } else { "" });
    let mut query = sqlx::query_scalar::<_, i64>(AssertSqlSafe(sql));
    for value in values {
        query = query.bind(value);
    }
    let rows = query.fetch_one(&mut *connection).await?;
    Ok(u64::try_from(rows).unwrap_or_default())
}

/// The values as they will be stored, or why one does not fit its column.
async fn stored_values(
    connection: &mut PgConnection,
    built: &Built,
) -> Result<Result<Vec<Vec<Option<String>>>, DbError>, sqlx::Error> {
    let Some(normalize) = &built.normalize else {
        return Ok(Ok(Vec::new()));
    };
    let query = bind_all(
        sqlx::query(AssertSqlSafe(normalize.sql.clone())),
        &normalize.parameters,
    );
    let rows = query.fetch_all(&mut *connection).await?;
    let mut values = Vec::with_capacity(rows.len());
    for (number, row) in rows.iter().enumerate() {
        let mut cells = Vec::with_capacity(normalize.width);
        for index in 0..normalize.width {
            cells.push(decode_cell(row, index)?);
        }
        for (offset, column) in normalize.checked.iter().enumerate() {
            if decode_cell(row, normalize.width + offset)?.as_deref() != Some("true") {
                let name = &built.value_columns[*column];
                let row_text = if built.insert_rows > 1 {
                    format!(" in row {}", number + 1)
                } else {
                    String::new()
                };
                return Ok(Err(invalid(format!(
                    "The value for {name}{row_text} is too long for the column."
                ))));
            }
        }
        values.push(cells);
    }
    Ok(Ok(values))
}

fn too_many_rows() -> DbError {
    invalid(format!(
        "The filters match more than {MAX_CHANGED_ROWS} rows. Narrow them to change fewer rows \
         at once."
    ))
}

/// Names the change an error comes from, when there are several.
fn label(error: DbError, index: usize, total: usize) -> DbError {
    if total == 1 {
        return error;
    }
    let message = format!("Change {} of {total}: {}", index + 1, error.message());
    error.with_message(message)
}

/// The tables `changes` write, each once, in a fixed order (so concurrent sets lock them in
/// the same order).
fn tables(changes: &[DataChange]) -> Vec<&ObjectName> {
    let mut tables: Vec<&ObjectName> = changes.iter().map(DataChange::table).collect();
    tables.sort_by(|a, b| (&a.schema, &a.name).cmp(&(&b.schema, &b.name)));
    tables.dedup();
    tables
}

/// Reads each table written, once.
async fn targets(
    connection: &mut PgConnection,
    changes: &[DataChange],
) -> Result<Result<HashMap<ObjectName, Target>, DbError>, sqlx::Error> {
    let mut targets = HashMap::new();
    for table in tables(changes) {
        match table_data::target(&mut *connection, table).await? {
            Ok(target) => {
                targets.insert(table.clone(), target);
            }
            Err(error) => return Ok(Err(error)),
        }
    }
    Ok(Ok(targets))
}

/// Plans `changes` in one read-only transaction that is rolled back. Errors name the change
/// they come from.
pub(super) async fn plan(
    connection: &mut PgConnection,
    changes: &[DataChange],
    classify: impl Fn(sqlx::Error) -> DbError,
) -> Result<Vec<DataChangePlan>, DbError> {
    let total = changes.len();
    let mut transaction = connection
        .begin_with("BEGIN READ ONLY")
        .await
        .map_err(&classify)?;
    let targets = targets(&mut transaction, changes)
        .await
        .map_err(&classify)??;
    let mut plans = Vec::with_capacity(total);
    let mut rows_in_all = 0_u64;
    for (index, change) in changes.iter().enumerate() {
        let label = |error| label(error, index, total);
        let target = &targets[change.table()];
        let built = build(change, target).map_err(label)?;
        let rows = match &built.count {
            Some(count) => count_rows(&mut transaction, count, false)
                .await
                .map_err(|e| label(classify(e)))?,
            None => built.insert_rows,
        };
        if rows > MAX_CHANGED_ROWS {
            return Err(label(too_many_rows()));
        }
        rows_in_all += rows;
        if rows_in_all > MAX_CHANGED_ROWS {
            return Err(invalid(format!(
                "These changes write more than {MAX_CHANGED_ROWS} rows in all. Apply fewer at \
                 once."
            )));
        }
        let values = stored_values(&mut transaction, &built)
            .await
            .map_err(|e| label(classify(e)))?
            .map_err(label)?;
        plans.push(DataChangePlan {
            statement: built.statement,
            parameters: built.parameters,
            rows,
            value_columns: built.value_columns,
            values,
            before: None,
        });
    }
    transaction.rollback().await.map_err(&classify)?;
    Ok(plans)
}

fn sorted(mut rows: Vec<Vec<Option<String>>>) -> Vec<Vec<Option<String>>> {
    rows.sort();
    rows
}

/// Applies `changes` in one transaction, in order, if each still plans to its `approved` plan;
/// see the module notes. Returns the rows written by all of them.
pub(super) async fn apply(
    connection: &mut PgConnection,
    changes: &[DataChange],
    approved: &[DataChangePlan],
    classify: impl Fn(sqlx::Error) -> DbError,
) -> Result<u64, ApplyError> {
    let failed = |error| ApplyError::RolledBack(classify(error));
    if changes.len() != approved.len() {
        return Err(ApplyError::RolledBack(invalid(
            "The changes do not match the approved plan.".to_owned(),
        )));
    }

    // A plain BEGIN: a read-only session keeps refusing writes.
    let mut transaction = connection.begin().await.map_err(failed)?;
    // The lock every write takes anyway, taken first on each table: it blocks structure
    // changes until the end, so the tables checked next are the tables written.
    let unlocked = targets(&mut transaction, changes)
        .await
        .map_err(failed)?
        .map_err(ApplyError::RolledBack)?;
    for table in tables(changes) {
        sqlx::query(AssertSqlSafe(format!(
            "LOCK TABLE ONLY {} IN ROW EXCLUSIVE MODE",
            unlocked[table].relation.quoted
        )))
        .execute(&mut *transaction)
        .await
        .map_err(failed)?;
    }
    let targets = targets(&mut transaction, changes)
        .await
        .map_err(failed)?
        .map_err(ApplyError::RolledBack)?;

    let total = changes.len();
    let mut written = 0;
    for (index, (change, plan)) in changes.iter().zip(approved).enumerate() {
        written += apply_one(
            &mut transaction,
            change,
            &targets[change.table()],
            plan,
            &classify,
        )
        .await
        .map_err(|error| match error {
            ApplyError::RolledBack(error) => ApplyError::RolledBack(label(error, index, total)),
            uncertain @ ApplyError::Uncertain(_) => uncertain,
        })?;
    }

    transaction.commit().await.map_err(|error| {
        let error = classify(error);
        // Without an answer from the server, the commit may or may not have happened.
        if error.sqlstate().is_none()
            || matches!(
                error.kind(),
                DbErrorKind::ConnectionLost | DbErrorKind::ServerUnavailable
            )
        {
            ApplyError::Uncertain(error)
        } else {
            ApplyError::RolledBack(error)
        }
    })?;
    Ok(written)
}

/// Checks one change against its approved plan, runs it and verifies what it wrote, inside the
/// write transaction. Changes earlier in the set have already run, so the recount sees them.
async fn apply_one(
    connection: &mut PgConnection,
    change: &DataChange,
    target: &Target,
    approved: &DataChangePlan,
    classify: &impl Fn(sqlx::Error) -> DbError,
) -> Result<u64, ApplyError> {
    let failed = |error| ApplyError::RolledBack(classify(error));
    let refused = |message: String| ApplyError::RolledBack(invalid(message));
    let built = build(change, target).map_err(ApplyError::RolledBack)?;
    if built.statement != approved.statement || built.parameters != approved.parameters {
        return Err(refused(
            "The table's structure changed since the change was approved. Nothing was changed."
                .to_owned(),
        ));
    }
    if let Some(count) = &built.count {
        let rows = count_rows(&mut *connection, count, true)
            .await
            .map_err(failed)?;
        if rows != approved.rows {
            return Err(refused(format!(
                "{} now match instead of the {} approved. Nothing was changed; review the \
                 change again.",
                rows_text(rows),
                approved.rows
            )));
        }
    }
    let values = stored_values(&mut *connection, &built)
        .await
        .map_err(failed)?
        .map_err(ApplyError::RolledBack)?;
    if values != approved.values {
        return Err(refused(
            "The values would be stored differently than approved. Nothing was changed.".to_owned(),
        ));
    }

    let query = bind_all(
        sqlx::query(AssertSqlSafe(built.statement.clone())),
        &built.parameters,
    );
    let (written, returned) = if change.kind() == ChangeKind::Delete {
        let result = query.execute(&mut *connection).await.map_err(failed)?;
        (result.rows_affected(), Vec::new())
    } else {
        let rows = query.fetch_all(&mut *connection).await.map_err(failed)?;
        let mut returned = Vec::with_capacity(rows.len());
        for row in &rows {
            let cells = (0..built.value_columns.len())
                .map(|index| decode_cell(row, index))
                .collect::<Result<Vec<_>, _>>()
                .map_err(failed)?;
            returned.push(cells);
        }
        (rows.len() as u64, returned)
    };
    if written != approved.rows {
        return Err(refused(format!(
            "The change wrote {} instead of the {} approved, so it was rolled back.",
            rows_text(written),
            approved.rows
        )));
    }
    let stored_as_planned = match change.kind() {
        ChangeKind::Delete => true,
        ChangeKind::Update => returned
            .iter()
            .all(|row| Some(row) == approved.values.first()),
        ChangeKind::Insert => sorted(returned) == sorted(approved.values.clone()),
    };
    if !stored_as_planned {
        return Err(refused(
            "The database stored different values than approved (a trigger or rule changed \
             them), so the change was rolled back."
                .to_owned(),
        ));
    }
    Ok(written)
}

fn rows_text(rows: u64) -> String {
    if rows == 1 {
        "1 row".to_owned()
    } else {
        format!("{rows} rows")
    }
}

#[cfg(test)]
mod tests;
