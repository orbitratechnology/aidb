//! Catalog queries. Every query reads `pg_catalog` only, with object names bound as parameters.
//!
//! User objects are everything outside `pg_catalog`, `information_schema` and the `pg_*` internal
//! schemas (TOAST, temporary). Partitions are left out of the overview: their parent is listed.

use sqlx::postgres::types::Oid;
use sqlx::{FromRow, PgPool};

use crate::adapters::database::{DbError, DbErrorKind};
use crate::domain::catalog::{
    CatalogOverview, Column, ForeignKey, Identity, Index, KeyConstraint, MAX_DEFINITION_BYTES,
    MAX_OVERVIEW_OBJECTS, ObjectDetails, ObjectKind, ObjectName, ObjectSummary, ReferentialAction,
    SchemaSummary, truncate_on_char_boundary,
};

/// A digest of names, types, nullability, constraints, defaults, view queries, indexes and
/// comments. It uses no row versions, so `ANALYZE` and `VACUUM` do not change it.
const FINGERPRINT: &str = r"
WITH schemas AS (
    SELECT n.oid, n.nspname FROM pg_namespace n
    WHERE n.nspname <> 'information_schema' AND n.nspname NOT LIKE 'pg\_%'
), rels AS (
    SELECT c.oid, c.relname, c.relkind, s.nspname
    FROM pg_class c JOIN schemas s ON s.oid = c.relnamespace
    WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f', 'i')
)
SELECT md5(concat_ws('|',
    (SELECT string_agg(s.oid::text || ':' || s.nspname, ',' ORDER BY s.oid) FROM schemas s),
    (SELECT string_agg(r.oid::text || ':' || r.nspname || '.' || r.relname || ':' || r.relkind::text,
                       ',' ORDER BY r.oid) FROM rels r),
    (SELECT string_agg(a.attrelid::text || ':' || a.attnum || ':' || a.attname || ':'
                       || a.atttypid || ':' || a.atttypmod || ':' || a.attnotnull || ':'
                       || a.attisdropped || ':' || a.attidentity::text || ':' || a.attgenerated::text,
                       ',' ORDER BY a.attrelid, a.attnum)
       FROM pg_attribute a JOIN rels r ON r.oid = a.attrelid WHERE a.attnum > 0),
    (SELECT string_agg(con.oid::text || ':' || md5(pg_get_constraintdef(con.oid)),
                       ',' ORDER BY con.oid)
       FROM pg_constraint con JOIN rels r ON r.oid = con.conrelid),
    (SELECT string_agg(d.adrelid::text || ':' || d.adnum || ':' || md5(pg_get_expr(d.adbin, d.adrelid)),
                       ',' ORDER BY d.adrelid, d.adnum)
       FROM pg_attrdef d JOIN rels r ON r.oid = d.adrelid),
    (SELECT string_agg(r.oid::text || ':' || md5(pg_get_viewdef(r.oid)), ',' ORDER BY r.oid)
       FROM rels r WHERE r.relkind IN ('v', 'm')),
    (SELECT string_agg(ds.classoid::text || ':' || ds.objoid || ':' || ds.objsubid || ':'
                       || md5(ds.description), ',' ORDER BY ds.classoid, ds.objoid, ds.objsubid)
       FROM pg_description ds
       WHERE (ds.classoid = 'pg_class'::regclass AND ds.objoid IN (SELECT oid FROM rels))
          OR (ds.classoid = 'pg_namespace'::regclass AND ds.objoid IN (SELECT oid FROM schemas)))
)) AS fingerprint";

pub(super) async fn fingerprint(pool: &PgPool) -> Result<String, sqlx::Error> {
    sqlx::query_scalar(FINGERPRINT).fetch_one(pool).await
}

#[derive(FromRow)]
struct SchemaRow {
    name: String,
    comment: Option<String>,
    accessible: bool,
}

#[derive(FromRow)]
struct ObjectRow {
    schema: String,
    name: String,
    kind: String,
    row_estimate: Option<i64>,
    comment: Option<String>,
    readable: bool,
}

impl ObjectRow {
    fn into_summary(self) -> Option<ObjectSummary> {
        Some(ObjectSummary {
            kind: object_kind(&self.kind)?,
            schema: self.schema,
            name: self.name,
            row_estimate: self.row_estimate.and_then(|rows| u64::try_from(rows).ok()),
            comment: self.comment,
            readable: self.readable,
        })
    }
}

fn object_kind(relkind: &str) -> Option<ObjectKind> {
    Some(match relkind {
        "r" => ObjectKind::Table,
        "p" => ObjectKind::PartitionedTable,
        "v" => ObjectKind::View,
        "m" => ObjectKind::MaterializedView,
        "f" => ObjectKind::ForeignTable,
        _ => return None,
    })
}

pub(super) async fn overview(pool: &PgPool) -> Result<CatalogOverview, sqlx::Error> {
    let mut connection = pool.acquire().await?;
    let database: String = sqlx::query_scalar("SELECT current_database()::text")
        .fetch_one(&mut *connection)
        .await?;
    let fingerprint: String = sqlx::query_scalar(FINGERPRINT)
        .fetch_one(&mut *connection)
        .await?;
    let schemas: Vec<SchemaRow> = sqlx::query_as(
        r"SELECT n.nspname::text AS name,
                 obj_description(n.oid, 'pg_namespace') AS comment,
                 has_schema_privilege(n.oid, 'USAGE') AS accessible
          FROM pg_namespace n
          WHERE n.nspname <> 'information_schema' AND n.nspname NOT LIKE 'pg\_%'
          ORDER BY n.nspname",
    )
    .fetch_all(&mut *connection)
    .await?;
    let limit = i64::try_from(MAX_OVERVIEW_OBJECTS + 1).unwrap_or(i64::MAX);
    let rows: Vec<ObjectRow> = sqlx::query_as(
        r"SELECT n.nspname::text AS schema, c.relname::text AS name, c.relkind::text AS kind,
                 CASE WHEN c.reltuples >= 0 THEN c.reltuples::int8 END AS row_estimate,
                 obj_description(c.oid, 'pg_class') AS comment,
                 has_schema_privilege(n.oid, 'USAGE')
                     AND has_table_privilege(c.oid, 'SELECT') AS readable
          FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f') AND NOT c.relispartition
            AND n.nspname <> 'information_schema' AND n.nspname NOT LIKE 'pg\_%'
          ORDER BY n.nspname, c.relname
          LIMIT $1",
    )
    .bind(limit)
    .fetch_all(&mut *connection)
    .await?;

    let truncated = rows.len() > MAX_OVERVIEW_OBJECTS;
    Ok(CatalogOverview {
        database,
        fingerprint,
        schemas: schemas
            .into_iter()
            .map(|row| SchemaSummary {
                name: row.name,
                comment: row.comment,
                accessible: row.accessible,
            })
            .collect(),
        objects: rows
            .into_iter()
            .take(MAX_OVERVIEW_OBJECTS)
            .filter_map(ObjectRow::into_summary)
            .collect(),
        truncated,
    })
}

#[derive(FromRow)]
struct TargetRow {
    oid: Oid,
    #[sqlx(flatten)]
    object: ObjectRow,
}

#[derive(FromRow)]
struct ColumnRow {
    name: String,
    position: i32,
    data_type: String,
    nullable: bool,
    default_expr: Option<String>,
    identity: String,
    generated: String,
    comment: Option<String>,
}

#[derive(FromRow)]
struct ConstraintRow {
    name: String,
    kind: String,
    from_schema: String,
    from_table: String,
    columns: Vec<String>,
    to_schema: Option<String>,
    to_table: Option<String>,
    referenced_columns: Vec<String>,
    on_update: String,
    on_delete: String,
}

#[derive(FromRow)]
struct IndexRow {
    name: String,
    definition: String,
    unique: bool,
    primary: bool,
    method: String,
}

/// Columns of a constraint's keys, in key order, as text arrays.
macro_rules! constraint_select {
    ($filter:literal) => {
        concat!(
            r"SELECT con.conname::text AS name, con.contype::text AS kind,
                     fn.nspname::text AS from_schema, fc.relname::text AS from_table,
                     ARRAY(SELECT a.attname::text
                           FROM unnest(con.conkey) WITH ORDINALITY AS k(attnum, ord)
                           JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.attnum
                           ORDER BY k.ord) AS columns,
                     tn.nspname::text AS to_schema, tc.relname::text AS to_table,
                     ARRAY(SELECT a.attname::text
                           FROM unnest(coalesce(con.confkey, '{}')) WITH ORDINALITY AS k(attnum, ord)
                           JOIN pg_attribute a ON a.attrelid = con.confrelid AND a.attnum = k.attnum
                           ORDER BY k.ord) AS referenced_columns,
                     con.confupdtype::text AS on_update, con.confdeltype::text AS on_delete
              FROM pg_constraint con
              JOIN pg_class fc ON fc.oid = con.conrelid
              JOIN pg_namespace fn ON fn.oid = fc.relnamespace
              LEFT JOIN pg_class tc ON tc.oid = con.confrelid
              LEFT JOIN pg_namespace tn ON tn.oid = tc.relnamespace
              WHERE ",
            $filter,
            " ORDER BY con.contype, con.conname"
        )
    };
}

pub(super) async fn describe(
    pool: &PgPool,
    object: &ObjectName,
) -> Result<Result<ObjectDetails, DbError>, sqlx::Error> {
    let mut connection = pool.acquire().await?;
    let target: Option<TargetRow> = sqlx::query_as(
        r"SELECT c.oid, n.nspname::text AS schema, c.relname::text AS name,
                 c.relkind::text AS kind,
                 CASE WHEN c.reltuples >= 0 THEN c.reltuples::int8 END AS row_estimate,
                 obj_description(c.oid, 'pg_class') AS comment,
                 has_schema_privilege(n.oid, 'USAGE')
                     AND has_table_privilege(c.oid, 'SELECT') AS readable
          FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind IN ('r', 'p', 'v', 'm', 'f')",
    )
    .bind(&object.schema)
    .bind(&object.name)
    .fetch_optional(&mut *connection)
    .await?;
    let Some((oid, summary)) = target.and_then(|t| Some((t.oid, t.object.into_summary()?))) else {
        return Ok(Err(DbError::new(
            DbErrorKind::ObjectNotFound,
            format!("{}.{} no longer exists.", object.schema, object.name),
        )));
    };

    let columns: Vec<ColumnRow> = sqlx::query_as(
        r"SELECT a.attname::text AS name, a.attnum::int4 AS position,
                 format_type(a.atttypid, a.atttypmod) AS data_type,
                 NOT a.attnotnull AS nullable,
                 pg_get_expr(d.adbin, d.adrelid) AS default_expr,
                 a.attidentity::text AS identity, a.attgenerated::text AS generated,
                 col_description(a.attrelid, a.attnum) AS comment
          FROM pg_attribute a
          LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
          WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped
          ORDER BY a.attnum",
    )
    .bind(oid)
    .fetch_all(&mut *connection)
    .await?;
    let own: Vec<ConstraintRow> = sqlx::query_as(constraint_select!(
        "con.conrelid = $1 AND con.contype IN ('p', 'u', 'f')"
    ))
    .bind(oid)
    .fetch_all(&mut *connection)
    .await?;
    let incoming: Vec<ConstraintRow> = sqlx::query_as(constraint_select!(
        "con.confrelid = $1 AND con.contype = 'f'"
    ))
    .bind(oid)
    .fetch_all(&mut *connection)
    .await?;
    let indexes: Vec<IndexRow> = sqlx::query_as(
        r"SELECT ic.relname::text AS name, pg_get_indexdef(i.indexrelid) AS definition,
                 i.indisunique AS unique, i.indisprimary AS primary, am.amname::text AS method
          FROM pg_index i
          JOIN pg_class ic ON ic.oid = i.indexrelid
          JOIN pg_am am ON am.oid = ic.relam
          WHERE i.indrelid = $1
          ORDER BY ic.relname",
    )
    .bind(oid)
    .fetch_all(&mut *connection)
    .await?;
    let definition = if summary.kind.is_view() {
        let mut text: String = sqlx::query_scalar("SELECT pg_get_viewdef($1, true)")
            .bind(oid)
            .fetch_one(&mut *connection)
            .await?;
        if truncate_on_char_boundary(&mut text, MAX_DEFINITION_BYTES) {
            text.push_str("\n-- (cut short)");
        }
        Some(text)
    } else {
        None
    };

    let mut primary_key = None;
    let mut unique_constraints = Vec::new();
    let mut foreign_keys = Vec::new();
    for row in own {
        match row.kind.as_str() {
            "p" => primary_key = Some(key(row)),
            "u" => unique_constraints.push(key(row)),
            _ => foreign_keys.extend(foreign_key(row)),
        }
    }

    Ok(Ok(ObjectDetails {
        summary,
        columns: columns.into_iter().map(column).collect(),
        primary_key,
        unique_constraints,
        foreign_keys,
        referenced_by: incoming.into_iter().filter_map(foreign_key).collect(),
        indexes: indexes
            .into_iter()
            .map(|row| Index {
                name: row.name,
                definition: row.definition,
                unique: row.unique,
                primary: row.primary,
                method: row.method,
            })
            .collect(),
        definition,
    }))
}

fn column(row: ColumnRow) -> Column {
    let generated = row.generated == "s";
    Column {
        name: row.name,
        position: u32::try_from(row.position).unwrap_or_default(),
        data_type: row.data_type,
        nullable: row.nullable,
        identity: match row.identity.as_str() {
            "a" => Some(Identity::Always),
            "d" => Some(Identity::ByDefault),
            _ => None,
        },
        // A generated column's expression is stored where defaults are.
        generated: if generated {
            row.default_expr.clone()
        } else {
            None
        },
        default: if generated { None } else { row.default_expr },
        comment: row.comment,
    }
}

fn key(row: ConstraintRow) -> KeyConstraint {
    KeyConstraint {
        name: row.name,
        columns: row.columns,
    }
}

fn foreign_key(row: ConstraintRow) -> Option<ForeignKey> {
    Some(ForeignKey {
        name: row.name,
        from: ObjectName {
            schema: row.from_schema,
            name: row.from_table,
        },
        columns: row.columns,
        to: ObjectName {
            schema: row.to_schema?,
            name: row.to_table?,
        },
        referenced_columns: row.referenced_columns,
        on_update: action(&row.on_update),
        on_delete: action(&row.on_delete),
    })
}

fn action(code: &str) -> ReferentialAction {
    match code {
        "r" => ReferentialAction::Restrict,
        "c" => ReferentialAction::Cascade,
        "n" => ReferentialAction::SetNull,
        "d" => ReferentialAction::SetDefault,
        _ => ReferentialAction::NoAction,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relation_kinds_map_and_others_are_skipped() {
        assert_eq!(object_kind("r"), Some(ObjectKind::Table));
        assert_eq!(object_kind("p"), Some(ObjectKind::PartitionedTable));
        assert_eq!(object_kind("m"), Some(ObjectKind::MaterializedView));
        assert_eq!(object_kind("i"), None);
    }

    #[test]
    fn referential_actions_map_from_catalog_codes() {
        assert_eq!(action("a"), ReferentialAction::NoAction);
        assert_eq!(action("c"), ReferentialAction::Cascade);
        assert_eq!(action("n"), ReferentialAction::SetNull);
    }

    #[test]
    fn generated_columns_report_their_expression_not_a_default() {
        let generated = column(ColumnRow {
            name: "total".into(),
            position: 3,
            data_type: "numeric".into(),
            nullable: true,
            default_expr: Some("(price * qty)".into()),
            identity: String::new(),
            generated: "s".into(),
            comment: None,
        });

        assert_eq!(generated.generated.as_deref(), Some("(price * qty)"));
        assert_eq!(generated.default, None);
    }
}
