//! A read's plan for the query advisor, and what the database knows about the tables it reads:
//! row estimates, when they were last analyzed, columns and indexes. All in one `READ ONLY`
//! transaction that is rolled back; with `analyze`, the read runs inside it.

use serde_json::Value;
use sqlx::{AssertSqlSafe, Connection, FromRow, PgConnection};

use crate::adapters::database::DbError;
use crate::domain::advisor::{IndexFacts, TableFacts};

#[derive(FromRow)]
struct TableRow {
    schema: String,
    name: String,
    rows: Option<f64>,
    last_analyzed: Option<String>,
    columns: Vec<String>,
}

#[derive(FromRow)]
struct IndexRow {
    schema: String,
    table_name: String,
    name: String,
    columns: Vec<Option<String>>,
    partial: bool,
}

const TABLES: &str = r"
SELECT n.nspname::text AS schema, c.relname::text AS name,
       CASE WHEN c.reltuples >= 0 THEN c.reltuples::float8 END AS rows,
       to_char(greatest(s.last_analyze, s.last_autoanalyze), 'YYYY-MM-DD HH24:MI:SS TZ')
           AS last_analyzed,
       ARRAY(SELECT a.attname::text FROM pg_attribute a
             WHERE a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped
             ORDER BY a.attnum) AS columns
FROM unnest($1::text[], $2::text[]) AS wanted(schema, name)
JOIN pg_namespace n ON n.nspname = wanted.schema
JOIN pg_class c ON c.relnamespace = n.oid AND c.relname = wanted.name
LEFT JOIN pg_stat_all_tables s ON s.relid = c.oid";

/// Key columns of each index, in order; an expression is NULL.
const INDEXES: &str = r"
SELECT n.nspname::text AS schema, t.relname::text AS table_name, i.relname::text AS name,
       ARRAY(SELECT a.attname::text
             FROM unnest(x.indkey::int2[]) WITH ORDINALITY AS k(num, ord)
             LEFT JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = k.num
             WHERE k.ord <= x.indnkeyatts
             ORDER BY k.ord) AS columns,
       x.indpred IS NOT NULL AS partial
FROM unnest($1::text[], $2::text[]) AS wanted(schema, name)
JOIN pg_namespace n ON n.nspname = wanted.schema
JOIN pg_class t ON t.relnamespace = n.oid AND t.relname = wanted.name
JOIN pg_index x ON x.indrelid = t.oid
JOIN pg_class i ON i.oid = x.indexrelid
ORDER BY 1, 2, 3";

/// `(schema, table)` of every relation the plan scans, once each.
fn relations(plan: &Value) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut stack = vec![plan];
    while let Some(value) = stack.pop() {
        match value {
            Value::Array(items) => stack.extend(items),
            Value::Object(map) => {
                if let (Some(Value::String(schema)), Some(Value::String(name))) =
                    (map.get("Schema"), map.get("Relation Name"))
                {
                    let pair = (schema.clone(), name.clone());
                    if !found.contains(&pair) {
                        found.push(pair);
                    }
                }
                stack.extend(map.values());
            }
            _ => {}
        }
    }
    found
}

/// Explains `sql`, one read statement already checked, and reads the facts of its tables.
pub(super) async fn explain(
    connection: &mut PgConnection,
    sql: &str,
    analyze: bool,
    classify: impl Fn(sqlx::Error) -> DbError,
) -> Result<(Value, Vec<TableFacts>), DbError> {
    let mut transaction = connection
        .begin_with("BEGIN READ ONLY")
        .await
        .map_err(&classify)?;
    let options = if analyze {
        "FORMAT JSON, VERBOSE, ANALYZE, BUFFERS"
    } else {
        "FORMAT JSON, VERBOSE"
    };
    // One statement, already split and parsed; the extended protocol would refuse a second.
    let plan: sqlx::types::Json<Value> =
        sqlx::query_scalar(AssertSqlSafe(format!("EXPLAIN ({options}) {sql}")))
            .fetch_one(&mut *transaction)
            .await
            .map_err(&classify)?;
    let wanted = relations(&plan.0);
    let schemas: Vec<&str> = wanted.iter().map(|(schema, _)| schema.as_str()).collect();
    let names: Vec<&str> = wanted.iter().map(|(_, name)| name.as_str()).collect();
    let tables: Vec<TableRow> = sqlx::query_as(TABLES)
        .bind(&schemas)
        .bind(&names)
        .fetch_all(&mut *transaction)
        .await
        .map_err(&classify)?;
    let indexes: Vec<IndexRow> = sqlx::query_as(INDEXES)
        .bind(&schemas)
        .bind(&names)
        .fetch_all(&mut *transaction)
        .await
        .map_err(&classify)?;
    transaction.rollback().await.map_err(&classify)?;
    let facts = tables
        .into_iter()
        .map(|table| TableFacts {
            indexes: indexes
                .iter()
                .filter(|index| index.schema == table.schema && index.table_name == table.name)
                .map(|index| IndexFacts {
                    name: index.name.clone(),
                    columns: index.columns.clone(),
                    partial: index.partial,
                })
                .collect(),
            schema: table.schema,
            name: table.name,
            rows: table.rows,
            last_analyzed: table.last_analyzed,
            columns: table.columns,
        })
        .collect();
    Ok((plan.0, facts))
}
