//! Pages of rows from one table or view, and exports of all matching rows.
//!
//! The statement is built from fixed SQL fragments and from names as the database itself quotes
//! them (`quote_ident`) in the live column list. Names in the request are only compared with that
//! list; they never reach the SQL text. Filter values, the limit and the offset are bound
//! parameters. The read runs in a read-only transaction, whatever the session allows.
//!
//! Every value is converted to text by the server (arrays to JSON text). For pages it is cut to
//! [`CELL_CHAR_LIMIT`] characters before it is sent, so wide or huge values cost little; exports
//! stream complete values, one row at a time, to a [`RowSink`].
//!
//! In a `SQL_ASCII` database the server neither knows nor checks the bytes' encoding: `left()`
//! counts bytes there and can split a character, and stored invalid bytes would make the server
//! refuse to send the row to a UTF-8 client. Cells are read as raw bytes instead and decoded
//! here, with invalid sequences replaced, so such a table can always be read.

use futures_util::TryStreamExt;
use sqlx::postgres::PgConnection;
use sqlx::postgres::types::Oid;
use sqlx::{AssertSqlSafe, Connection, Row, ValueRef};

use crate::adapters::database::{DbError, DbErrorKind, ExportError};
use crate::domain::catalog::ObjectName;
use crate::domain::export::{ExportSummary, RowSink};
use crate::domain::table_data::{
    CELL_CHAR_LIMIT, CellRef, FilterOperator, PageColumn, PageLimits, PageRequest, SortDirection,
    SortKey, TablePage, ValueKind, escape_like, truncate_cell,
};

/// Rough per-cell cost of a value in the reply, on top of its text.
const CELL_OVERHEAD_BYTES: usize = 4;
/// Longest UTF-8 encoding of one character.
const MAX_CHAR_BYTES: usize = 4;

#[derive(sqlx::FromRow)]
struct RelationRow {
    oid: Oid,
    /// `schema.name`, quoted by the server.
    quoted: String,
    usable: bool,
    /// The server's `server_encoding`.
    encoding: String,
}

impl RelationRow {
    fn raw_bytes(&self) -> bool {
        self.encoding == "SQL_ASCII"
    }
}

#[derive(sqlx::FromRow)]
struct ColumnRow {
    name: String,
    /// The name quoted by the server.
    quoted: String,
    data_type: String,
    /// The base type (a domain's underlying type) as `schema.name`, quoted by the server.
    base_type: String,
    category: String,
    type_name: String,
    readable: bool,
    /// Position in the primary key, from 1.
    key_position: Option<i32>,
}

impl ColumnRow {
    fn kind(&self) -> ValueKind {
        ValueKind::classify(&self.category, &self.type_name)
    }
}

/// The object and its live columns, checked for read access.
struct Target {
    relation: RelationRow,
    columns: Vec<ColumnRow>,
}

impl Target {
    fn readable(&self) -> Vec<&ColumnRow> {
        self.columns.iter().filter(|c| c.readable).collect()
    }

    fn page_columns(&self) -> Vec<PageColumn> {
        self.readable()
            .iter()
            .map(|column| PageColumn {
                name: column.name.clone(),
                data_type: column.data_type.clone(),
                kind: column.kind(),
            })
            .collect()
    }
}

/// Looks up the object and its columns. The outer error is a driver failure for the caller to
/// classify; the inner one a problem with the request itself.
async fn target(
    connection: &mut PgConnection,
    object: &ObjectName,
) -> Result<Result<Target, DbError>, sqlx::Error> {
    let relation: Option<RelationRow> = sqlx::query_as(
        r"SELECT c.oid, quote_ident(n.nspname) || '.' || quote_ident(c.relname) AS quoted,
                 has_schema_privilege(n.oid, 'USAGE') AS usable,
                 current_setting('server_encoding') AS encoding
          FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind IN ('r', 'p', 'v', 'm', 'f')",
    )
    .bind(&object.schema)
    .bind(&object.name)
    .fetch_optional(&mut *connection)
    .await?;
    let Some(relation) = relation else {
        return Ok(Err(DbError::new(
            DbErrorKind::ObjectNotFound,
            format!("{}.{} no longer exists.", object.schema, object.name),
        )));
    };

    let columns: Vec<ColumnRow> = sqlx::query_as(
        r"SELECT a.attname::text AS name, quote_ident(a.attname) AS quoted,
                 format_type(a.atttypid, a.atttypmod) AS data_type,
                 quote_ident(bn.nspname) || '.' || quote_ident(bt.typname) AS base_type,
                 bt.typcategory::text AS category, bt.typname::text AS type_name,
                 has_column_privilege(a.attrelid, a.attnum, 'SELECT') AS readable,
                 array_position(pk.conkey, a.attnum)::int4 AS key_position
          FROM pg_attribute a
          JOIN pg_type t ON t.oid = a.atttypid
          JOIN pg_type bt ON bt.oid = CASE WHEN t.typtype = 'd' THEN t.typbasetype ELSE t.oid END
          JOIN pg_namespace bn ON bn.oid = bt.typnamespace
          LEFT JOIN pg_constraint pk ON pk.conrelid = a.attrelid AND pk.contype = 'p'
          WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped
          ORDER BY a.attnum",
    )
    .bind(relation.oid)
    .fetch_all(&mut *connection)
    .await?;

    let readable = columns.iter().filter(|c| c.readable).count();
    if !relation.usable || (readable == 0 && !columns.is_empty()) {
        return Ok(Err(DbError::new(
            DbErrorKind::PermissionDenied,
            format!(
                "Your database role cannot read {}.{}.",
                object.schema, object.name
            ),
        )));
    }
    Ok(Ok(Target { relation, columns }))
}

/// Reads one page. The outer error is a driver failure for the caller to classify; the inner
/// one a problem with the request itself.
pub(super) async fn read_page(
    connection: &mut PgConnection,
    request: &PageRequest,
    limits: PageLimits,
) -> Result<Result<TablePage, DbError>, sqlx::Error> {
    let mut transaction = connection.begin_with("BEGIN READ ONLY").await?;
    let object = &request.object;
    let target = match target(&mut transaction, object).await? {
        Ok(target) => target,
        Err(error) => return Ok(Err(error)),
    };
    let readable = target.readable();
    let limit = request
        .limit
        .map_or(limits.max_rows, |limit| limit.min(limits.max_rows));
    let statement = match Statement::build(
        request,
        limit,
        CellLength::Cut,
        &target.relation,
        &target.columns,
        &readable,
    ) {
        Ok(statement) => statement,
        Err(error) => return Ok(Err(error)),
    };

    let mut query = sqlx::query(AssertSqlSafe(statement.sql));
    for value in statement.values {
        query = query.bind(value);
    }
    let fetch_rows = i64::from(statement.limit) + 1;
    let offset = i64::try_from(request.offset).unwrap_or(i64::MAX);
    let stream = query.bind(fetch_rows).bind(offset).fetch(&mut *transaction);
    let page = collect(stream, readable.len(), statement.limit, limits.max_bytes).await?;
    transaction.rollback().await?;

    Ok(Ok(TablePage {
        object: object.clone(),
        columns: target.page_columns(),
        omitted_columns: target
            .columns
            .iter()
            .filter(|c| !c.readable)
            .map(|c| c.name.clone())
            .collect(),
        rows: page.rows,
        truncated_cells: page.truncated,
        offset: request.offset,
        has_more: page.has_more,
        byte_limit_reached: page.byte_limit_reached,
        order: statement.order,
        stable_order: statement.stable_order,
        cell_char_limit: CELL_CHAR_LIMIT,
    }))
}

/// Streams every matching row, up to `max_rows`, to `sink`, in a read-only transaction. Driver
/// errors are turned into [`DbError`]s by `classify`.
pub(super) async fn export_rows(
    connection: &mut PgConnection,
    request: &PageRequest,
    max_rows: u32,
    sink: &mut (dyn RowSink + Send),
    classify: impl Fn(sqlx::Error) -> DbError,
) -> Result<ExportSummary, ExportError> {
    let database = ExportError::Database;
    let driver = |error| ExportError::Database(classify(error));
    let mut transaction = connection
        .begin_with("BEGIN READ ONLY")
        .await
        .map_err(driver)?;
    let target = target(&mut transaction, &request.object)
        .await
        .map_err(driver)?
        .map_err(database)?;
    let readable = target.readable();
    let statement = Statement::build(
        request,
        max_rows,
        CellLength::Full,
        &target.relation,
        &target.columns,
        &readable,
    )
    .map_err(database)?;
    sink.columns(&target.page_columns())
        .map_err(ExportError::Write)?;

    let mut query = sqlx::query(AssertSqlSafe(statement.sql));
    for value in statement.values {
        query = query.bind(value);
    }
    let mut stream = query
        .bind(i64::from(max_rows) + 1)
        .bind(0_i64)
        .fetch(&mut *transaction);
    let mut summary = ExportSummary {
        rows: 0,
        truncated: false,
    };
    let mut cells = Vec::with_capacity(readable.len());
    while let Some(row) = stream.try_next().await.map_err(driver)? {
        if summary.rows == u64::from(max_rows) {
            summary.truncated = true;
            break;
        }
        cells.clear();
        for index in 0..readable.len() {
            cells.push(decode_cell(&row, index).map_err(driver)?);
        }
        sink.row(&cells).map_err(ExportError::Write)?;
        summary.rows += 1;
    }
    drop(stream);
    transaction.rollback().await.map_err(driver)?;
    Ok(summary)
}

/// One cell as text. Every value aidb selects arrives as text, JSON or bytes (`SQL_ASCII`
/// databases), so the wire bytes are decoded as UTF-8, with invalid sequences replaced.
pub(super) fn decode_cell(
    row: &sqlx::postgres::PgRow,
    index: usize,
) -> Result<Option<String>, sqlx::Error> {
    let value = row.try_get_raw(index)?;
    if value.is_null() {
        return Ok(None);
    }
    let bytes = value.as_bytes().map_err(sqlx::Error::Decode)?;
    Ok(Some(String::from_utf8_lossy(bytes).into_owned()))
}

/// Whether values are cut for display or sent whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellLength {
    /// One character past [`CELL_CHAR_LIMIT`], so a cut value can be flagged.
    Cut,
    Full,
}

/// Rows collected for one page or query result.
#[derive(Default)]
pub(super) struct Collected {
    pub rows: Vec<Vec<Option<String>>>,
    pub truncated: Vec<CellRef>,
    bytes: usize,
    pub has_more: bool,
    pub byte_limit_reached: bool,
}

/// Reads up to `limit` rows of `width` text cells, stopping early at `max_bytes` (the first row
/// is always kept). Cells longer than [`CELL_CHAR_LIMIT`] are cut and flagged. Expects the
/// statement to fetch one row more than `limit`, so `has_more` can be told apart.
pub(super) async fn collect<'s>(
    mut stream: futures_util::stream::BoxStream<'s, Result<sqlx::postgres::PgRow, sqlx::Error>>,
    width: usize,
    limit: u32,
    max_bytes: usize,
) -> Result<Collected, sqlx::Error> {
    let mut page = Collected::default();
    while let Some(row) = stream.try_next().await? {
        if page.rows.len() == limit as usize {
            page.has_more = true;
            break;
        }
        let row_index = u32::try_from(page.rows.len()).unwrap_or(u32::MAX);
        let mut cells = Vec::with_capacity(width);
        let mut truncated = Vec::new();
        let mut row_bytes = 0;
        for index in 0..width {
            let mut cell = decode_cell(&row, index)?;
            if let Some(text) = &mut cell {
                if truncate_cell(text) {
                    truncated.push(CellRef {
                        row: row_index,
                        column: u32::try_from(index).unwrap_or(u32::MAX),
                    });
                }
                row_bytes += text.len();
            }
            row_bytes += CELL_OVERHEAD_BYTES;
            cells.push(cell);
        }
        if !page.rows.is_empty() && page.bytes + row_bytes > max_bytes {
            page.has_more = true;
            page.byte_limit_reached = true;
            break;
        }
        page.bytes += row_bytes;
        page.rows.push(cells);
        page.truncated.extend(truncated);
    }
    Ok(page)
}

/// The select-list expression that sends `value` (any SQL expression) as text: arrays as JSON,
/// cut for display when `cut`, and as bytes for `SQL_ASCII` databases (see the module notes).
pub(super) fn text_expression(value: &str, kind: ValueKind, cut: bool, raw_bytes: bool) -> String {
    let text = match kind {
        ValueKind::Array => format!("to_json({value})::text"),
        _ => format!("({value})::text"),
    };
    let text = if cut {
        // One character past the limit shows that a value was cut. Without a known encoding,
        // `left()` counts bytes, so take enough for the limit in any UTF-8 text.
        let length = if raw_bytes {
            (CELL_CHAR_LIMIT + 1) * MAX_CHAR_BYTES
        } else {
            CELL_CHAR_LIMIT + 1
        };
        format!("left({text}, {length})")
    } else {
        text
    };
    if raw_bytes {
        format!("convert_to({text}, 'SQL_ASCII')")
    } else {
        text
    }
}

/// The SELECT for one page, with its bound values in `$n` order. The limit and offset are bound
/// last, after `values`.
#[derive(Debug)]
struct Statement {
    sql: String,
    values: Vec<String>,
    limit: u32,
    order: Vec<SortKey>,
    stable_order: bool,
}

impl Statement {
    fn build(
        request: &PageRequest,
        limit: u32,
        cell_length: CellLength,
        relation: &RelationRow,
        columns: &[ColumnRow],
        readable: &[&ColumnRow],
    ) -> Result<Self, DbError> {
        let find = |name: &str| -> Result<&ColumnRow, DbError> {
            match columns.iter().find(|c| c.name == name) {
                Some(column) if column.readable => Ok(column),
                Some(_) => Err(DbError::new(
                    DbErrorKind::PermissionDenied,
                    format!("Your database role cannot read the column {name}."),
                )),
                None => Err(DbError::new(
                    DbErrorKind::InvalidRequest,
                    format!(
                        "{}.{} has no column {name}.",
                        request.object.schema, request.object.name
                    ),
                )),
            }
        };

        let raw_bytes = relation.raw_bytes();
        let select = readable
            .iter()
            .map(|column| {
                text_expression(
                    &column.quoted,
                    column.kind(),
                    cell_length == CellLength::Cut,
                    raw_bytes,
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let mut sql = format!("SELECT {select} FROM {}", relation.quoted);

        let mut values = Vec::new();
        let mut conditions = Vec::new();
        for filter in &request.filters {
            let column = find(&filter.column)?;
            let value = filter.value.clone().unwrap_or_default();
            let comparison = match filter.operator {
                FilterOperator::IsNull => {
                    conditions.push(format!("{} IS NULL", column.quoted));
                    continue;
                }
                FilterOperator::IsNotNull => {
                    conditions.push(format!("{} IS NOT NULL", column.quoted));
                    continue;
                }
                FilterOperator::Contains | FilterOperator::StartsWith => {
                    let pattern = escape_like(&value);
                    values.push(if filter.operator == FilterOperator::Contains {
                        format!("%{pattern}%")
                    } else {
                        format!("{pattern}%")
                    });
                    conditions.push(format!("({})::text ILIKE ${}", column.quoted, values.len()));
                    continue;
                }
                FilterOperator::Equals => "=",
                FilterOperator::NotEquals => "<>",
                FilterOperator::Less => "<",
                FilterOperator::LessOrEqual => "<=",
                FilterOperator::Greater => ">",
                FilterOperator::GreaterOrEqual => ">=",
            };
            values.push(value);
            conditions.push(format!(
                "{} {comparison} CAST(${} AS {})",
                column.quoted,
                values.len(),
                column.base_type
            ));
        }
        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }

        let mut order = request.sort.clone();
        let mut key: Vec<&ColumnRow> = columns
            .iter()
            .filter(|c| c.key_position.is_some())
            .collect();
        key.sort_by_key(|c| c.key_position);
        let stable_order = !key.is_empty() && key.iter().all(|c| c.readable);
        if stable_order {
            for column in key {
                if !order.iter().any(|k| k.column == column.name) {
                    order.push(SortKey {
                        column: column.name.clone(),
                        direction: SortDirection::Asc,
                    });
                }
            }
        }
        let terms = order
            .iter()
            .map(|key| {
                let column = find(&key.column)?;
                let direction = match key.direction {
                    SortDirection::Asc => "ASC",
                    SortDirection::Desc => "DESC",
                };
                // Qualified, so the name cannot resolve to an output column of the same name
                // (PostgreSQL matches ORDER BY names against the select list first).
                Ok(format!("{}.{} {direction}", relation.quoted, column.quoted))
            })
            .collect::<Result<Vec<_>, DbError>>()?;
        if !terms.is_empty() {
            sql.push_str(" ORDER BY ");
            sql.push_str(&terms.join(", "));
        }

        sql.push_str(&format!(
            " LIMIT ${} OFFSET ${}",
            values.len() + 1,
            values.len() + 2
        ));
        Ok(Self {
            sql,
            values,
            limit,
            order,
            stable_order,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::catalog::ObjectName;
    use crate::domain::table_data::Filter;

    fn column(name: &str, quoted: &str, key_position: Option<i32>) -> ColumnRow {
        ColumnRow {
            name: name.into(),
            quoted: quoted.into(),
            data_type: "integer".into(),
            base_type: "pg_catalog.int4".into(),
            category: "N".into(),
            type_name: "int4".into(),
            readable: true,
            key_position,
        }
    }

    fn relation() -> RelationRow {
        RelationRow {
            oid: Oid(1),
            quoted: r#"public."Order Items""#.into(),
            usable: true,
            encoding: "UTF8".into(),
        }
    }

    fn request() -> PageRequest {
        serde_json::from_str(r#"{"object": {"schema": "public", "name": "Order Items"}}"#)
            .expect("parses")
    }

    const LIMITS: PageLimits = PageLimits {
        max_rows: 100,
        max_bytes: 1_000_000,
    };

    fn build(request: &PageRequest, columns: &[ColumnRow]) -> Result<Statement, DbError> {
        let readable: Vec<&ColumnRow> = columns.iter().filter(|c| c.readable).collect();
        let limit = request
            .limit
            .map_or(LIMITS.max_rows, |limit| limit.min(LIMITS.max_rows));
        Statement::build(
            request,
            limit,
            CellLength::Cut,
            &relation(),
            columns,
            &readable,
        )
    }

    #[test]
    fn statements_use_server_quoted_names_and_bound_values() {
        let mut tags = column("tags", "tags", None);
        tags.category = "A".into();
        let columns = [
            column("id", "id", Some(1)),
            column("Weird \"name\"", r#""Weird ""name""""#, None),
            tags,
        ];
        let mut request = request();
        request.filters = vec![
            Filter {
                column: "Weird \"name\"".into(),
                operator: FilterOperator::GreaterOrEqual,
                value: Some("1; DROP TABLE x".into()),
            },
            Filter {
                column: "id".into(),
                operator: FilterOperator::Contains,
                value: Some("5%".into()),
            },
            Filter {
                column: "tags".into(),
                operator: FilterOperator::IsNull,
                value: None,
            },
        ];
        request.sort = vec![SortKey {
            column: "Weird \"name\"".into(),
            direction: SortDirection::Desc,
        }];
        request.limit = Some(500);

        let statement = build(&request, &columns).expect("builds");

        assert_eq!(
            statement.sql,
            r#"SELECT left((id)::text, 2001), left(("Weird ""name""")::text, 2001), left(to_json(tags)::text, 2001) FROM public."Order Items" WHERE "Weird ""name""" >= CAST($1 AS pg_catalog.int4) AND (id)::text ILIKE $2 AND tags IS NULL ORDER BY public."Order Items"."Weird ""name""" DESC, public."Order Items".id ASC LIMIT $3 OFFSET $4"#
        );
        assert_eq!(statement.values, ["1; DROP TABLE x", r"%5\%%"]);
        assert_eq!(statement.limit, LIMITS.max_rows, "capped by the setting");
        assert!(statement.stable_order);
        assert_eq!(statement.order.len(), 2);
    }

    #[test]
    fn sql_ascii_databases_send_raw_bytes() {
        let mut ascii = relation();
        ascii.encoding = "SQL_ASCII".into();
        let columns = [column("id", "id", None)];
        let readable: Vec<&ColumnRow> = columns.iter().collect();

        let statement = Statement::build(
            &request(),
            LIMITS.max_rows,
            CellLength::Cut,
            &ascii,
            &columns,
            &readable,
        )
        .expect("builds");
        let full = Statement::build(
            &request(),
            LIMITS.max_rows,
            CellLength::Full,
            &ascii,
            &columns,
            &readable,
        )
        .expect("builds");

        assert!(
            statement
                .sql
                .starts_with("SELECT convert_to(left((id)::text, 8004), 'SQL_ASCII') FROM"),
            "{}",
            statement.sql
        );
        assert!(
            full.sql
                .starts_with("SELECT convert_to((id)::text, 'SQL_ASCII') FROM"),
            "exports send whole values: {}",
            full.sql
        );
    }

    #[test]
    fn unknown_and_unreadable_columns_are_refused() {
        let mut secret = column("salary", "salary", None);
        secret.readable = false;
        let columns = [column("id", "id", None), secret];
        let mut request = request();
        request.sort = vec![SortKey {
            column: "id; DROP TABLE x".into(),
            direction: SortDirection::Asc,
        }];
        let unknown = build(&request, &columns).expect_err("unknown");
        assert_eq!(unknown.kind(), DbErrorKind::InvalidRequest);

        request.sort.clear();
        request.filters = vec![Filter {
            column: "salary".into(),
            operator: FilterOperator::IsNotNull,
            value: None,
        }];
        let hidden = build(&request, &columns).expect_err("unreadable");
        assert_eq!(hidden.kind(), DbErrorKind::PermissionDenied);
    }

    #[test]
    fn order_is_stable_only_with_a_readable_primary_key() {
        let without_key = build(&request(), &[column("id", "id", None)]).expect("builds");
        assert!(!without_key.stable_order);
        assert!(!without_key.sql.contains("ORDER BY"));

        let mut hidden_key = column("id", "id", Some(1));
        hidden_key.readable = false;
        let hidden = build(&request(), &[hidden_key, column("x", "x", None)]).expect("builds");
        assert!(!hidden.stable_order);

        let composite = build(
            &request(),
            &[column("b", "b", Some(2)), column("a", "a", Some(1))],
        )
        .expect("builds");
        assert!(
            composite
                .sql
                .contains(r#"ORDER BY public."Order Items".a ASC, public."Order Items".b ASC"#)
        );
    }

    #[test]
    fn the_object_name_is_only_echoed_in_messages() {
        let mut request = request();
        request.object = ObjectName {
            schema: "x".into(),
            name: "y".into(),
        };
        request.sort = vec![SortKey {
            column: "missing".into(),
            direction: SortDirection::Asc,
        }];
        let error = build(&request, &[column("id", "id", None)]).expect_err("unknown");
        assert_eq!(error.message(), "x.y has no column missing.");
    }
}
