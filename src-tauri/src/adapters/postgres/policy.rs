//! What the policy engine asks the database before classifying a change: how many rows the
//! planner expects it to write, and what the role may do to its target tables. Nothing here
//! runs the change itself.

use serde_json::Value;
use sqlx::{AssertSqlSafe, Connection, FromRow, PgConnection};

use crate::adapters::database::{DbError, DbErrorKind};
use crate::domain::policy::facts::TargetTable;

/// Plans `sql` with `EXPLAIN (FORMAT JSON)` (no ANALYZE) in a read-only transaction that is
/// rolled back, and returns the rows its data-changing nodes are fed.
pub(super) async fn estimate_written_rows(
    connection: &mut PgConnection,
    sql: &str,
    classify: impl Fn(sqlx::Error) -> DbError,
) -> Result<u64, DbError> {
    let mut transaction = connection
        .begin_with("BEGIN READ ONLY")
        .await
        .map_err(&classify)?;
    // One statement, already split and parsed; the extended protocol would refuse a second.
    let plan: sqlx::types::Json<Value> =
        sqlx::query_scalar(AssertSqlSafe(format!("EXPLAIN (FORMAT JSON) {sql}")))
            .fetch_one(&mut *transaction)
            .await
            .map_err(&classify)?;
    transaction.rollback().await.map_err(&classify)?;
    written_rows(&plan.0).ok_or_else(|| {
        DbError::new(
            DbErrorKind::Other,
            "The plan has no data-changing step to estimate.",
        )
    })
}

/// Rows fed to every `ModifyTable` node of a JSON plan, rounded up.
fn written_rows(plan: &Value) -> Option<u64> {
    fn visit(node: &Value, total: &mut Option<f64>) {
        if node.get("Node Type").and_then(Value::as_str) == Some("ModifyTable") {
            let fed = node
                .get("Plans")
                .and_then(Value::as_array)
                .and_then(|children| children.first())
                .and_then(|child| child.get("Plan Rows"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            *total = Some(total.unwrap_or(0.0) + fed);
        }
        if let Some(children) = node.get("Plans").and_then(Value::as_array) {
            for child in children {
                visit(child, total);
            }
        }
    }
    let mut total = None;
    for entry in plan.as_array()? {
        if let Some(root) = entry.get("Plan") {
            visit(root, &mut total);
        }
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "planner row estimates are non-negative and far below u64::MAX"
    )]
    total.map(|rows| rows.max(0.0).ceil() as u64)
}

#[derive(FromRow)]
struct TargetRow {
    target: String,
    resolved: Option<String>,
    can_insert: bool,
    can_update: bool,
    can_delete: bool,
    can_truncate: bool,
    cascades_to: Vec<String>,
}

const TARGETS_SQL: &str = "
SELECT t.target,
       CASE WHEN c.oid IS NULL THEN NULL ELSE format('%I.%I', n.nspname, c.relname) END AS resolved,
       COALESCE(has_table_privilege(c.oid, 'INSERT'), false) AS can_insert,
       COALESCE(has_table_privilege(c.oid, 'UPDATE'), false) AS can_update,
       COALESCE(has_table_privilege(c.oid, 'DELETE'), false) AS can_delete,
       COALESCE(has_table_privilege(c.oid, 'TRUNCATE'), false) AS can_truncate,
       COALESCE((
           SELECT array_agg(DISTINCT format('%I.%I', cn.nspname, cc.relname))
           FROM pg_constraint k
           JOIN pg_class cc ON cc.oid = k.conrelid
           JOIN pg_namespace cn ON cn.oid = cc.relnamespace
           WHERE k.confrelid = c.oid AND k.contype = 'f' AND k.confdeltype = 'c'
       ), '{}') AS cascades_to
FROM unnest($1::text[]) WITH ORDINALITY AS t(target, position)
LEFT JOIN pg_class c ON c.oid = to_regclass(t.target)
LEFT JOIN pg_namespace n ON n.oid = c.relnamespace
ORDER BY t.position";

pub(super) async fn write_targets(
    connection: &mut PgConnection,
    targets: &[String],
    classify: impl Fn(sqlx::Error) -> DbError,
) -> Result<Vec<TargetTable>, DbError> {
    let mut transaction = connection
        .begin_with("BEGIN READ ONLY")
        .await
        .map_err(&classify)?;
    let rows: Vec<TargetRow> = sqlx::query_as(TARGETS_SQL)
        .bind(targets)
        .fetch_all(&mut *transaction)
        .await
        .map_err(&classify)?;
    transaction.rollback().await.map_err(&classify)?;
    Ok(rows
        .into_iter()
        .map(|row| TargetTable {
            target: row.target,
            resolved: row.resolved,
            can_insert: row.can_insert,
            can_update: row.can_update,
            can_delete: row.can_delete,
            can_truncate: row.can_truncate,
            cascades_to: row.cascades_to,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::written_rows;

    #[test]
    fn rows_come_from_the_nodes_that_change_data() {
        let delete = json!([{ "Plan": {
            "Node Type": "ModifyTable", "Operation": "Delete", "Plan Rows": 0,
            "Plans": [{ "Node Type": "Seq Scan", "Plan Rows": 12.4 }]
        }}]);
        assert_eq!(written_rows(&delete), Some(13));

        // A data-changing CTE under a read: both writes count.
        let cte = json!([{ "Plan": {
            "Node Type": "Aggregate", "Plan Rows": 1,
            "Plans": [
                { "Node Type": "ModifyTable", "Plans": [{ "Node Type": "Seq Scan", "Plan Rows": 5 }] },
                { "Node Type": "ModifyTable", "Plans": [{ "Node Type": "Result", "Plan Rows": 2 }] }
            ]
        }}]);
        assert_eq!(written_rows(&cte), Some(7));

        let read = json!([{ "Plan": { "Node Type": "Seq Scan", "Plan Rows": 100 } }]);
        assert_eq!(written_rows(&read), None);
        assert_eq!(written_rows(&json!({})), None);
    }
}
