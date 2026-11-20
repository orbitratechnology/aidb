//! A schema's tables and views as a [`SchemaModel`], for comparison. Read in a `READ ONLY`
//! transaction with `pg_catalog` alone on the search path, so the server prints every name
//! outside `pg_catalog` (types, sequences, functions, referenced tables) with its schema, the
//! same way on every database.

use std::collections::HashMap;

use sqlx::postgres::types::Oid;
use sqlx::{Connection, FromRow, PgConnection};

use crate::adapters::database::{DbError, DbErrorKind};
use crate::domain::catalog::{Identity, ReferentialAction};
use crate::domain::schema_diff::{
    Column, Constraint, ConstraintKind, ForeignKeyDefinition, Index, MAX_SCHEMA_OBJECTS,
    SchemaModel, Table, View,
};

#[derive(FromRow)]
struct RelationRow {
    oid: Oid,
    name: String,
    kind: String,
    partition: bool,
    rows: Option<i64>,
}

#[derive(FromRow)]
struct ColumnRow {
    oid: Oid,
    name: String,
    data_type: String,
    nullable: bool,
    default_value: Option<String>,
    generated: Option<String>,
    identity: String,
    serial: bool,
}

#[derive(FromRow)]
struct ConstraintRow {
    oid: Oid,
    name: String,
    kind: String,
    columns: Vec<String>,
    expression: Option<String>,
    ref_schema: Option<String>,
    ref_table: Option<String>,
    referenced: Vec<String>,
    on_update: String,
    on_delete: String,
}

#[derive(FromRow)]
struct IndexRow {
    oid: Oid,
    name: String,
    unique: bool,
    definition: String,
}

#[derive(FromRow)]
struct ViewRow {
    oid: Oid,
    definition: Option<String>,
}

#[derive(FromRow)]
struct OtherRow {
    functions: i64,
    types: i64,
    sequences: i64,
    triggers: i64,
    policies: i64,
    exclusions: i64,
}

const RELATIONS: &str = r"
SELECT c.oid, c.relname::text AS name, c.relkind::text AS kind, c.relispartition AS partition,
       CASE WHEN c.reltuples >= 0 THEN c.reltuples::int8 END AS rows
FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE n.nspname = $1 AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
ORDER BY c.oid
LIMIT $2";

const COLUMNS: &str = r"
SELECT a.attrelid AS oid, a.attname::text AS name,
       format_type(a.atttypid, a.atttypmod) AS data_type,
       NOT a.attnotnull AS nullable,
       CASE WHEN a.attgenerated = '' THEN pg_get_expr(d.adbin, d.adrelid) END AS default_value,
       CASE WHEN a.attgenerated <> '' THEN pg_get_expr(d.adbin, d.adrelid) END AS generated,
       a.attidentity::text AS identity,
       EXISTS (SELECT 1 FROM pg_depend dep
               JOIN pg_class s ON s.oid = dep.objid AND s.relkind = 'S'
               WHERE dep.classid = 'pg_class'::regclass AND dep.refobjid = a.attrelid
                 AND dep.refobjsubid = a.attnum AND dep.deptype = 'a') AS serial
FROM pg_attribute a
LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
WHERE a.attrelid = ANY($1) AND a.attnum > 0 AND NOT a.attisdropped
ORDER BY a.attrelid, a.attnum";

const CONSTRAINTS: &str = r"
SELECT con.conrelid AS oid, con.conname::text AS name, con.contype::text AS kind,
       ARRAY(SELECT a.attname::text
             FROM unnest(con.conkey) WITH ORDINALITY AS k(num, ord)
             JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.num
             ORDER BY k.ord) AS columns,
       CASE WHEN con.contype = 'c' THEN pg_get_expr(con.conbin, con.conrelid) END AS expression,
       fn.nspname::text AS ref_schema, fc.relname::text AS ref_table,
       ARRAY(SELECT a.attname::text
             FROM unnest(con.confkey) WITH ORDINALITY AS k(num, ord)
             JOIN pg_attribute a ON a.attrelid = con.confrelid AND a.attnum = k.num
             ORDER BY k.ord) AS referenced,
       con.confupdtype::text AS on_update, con.confdeltype::text AS on_delete
FROM pg_constraint con
LEFT JOIN pg_class fc ON fc.oid = con.confrelid
LEFT JOIN pg_namespace fn ON fn.oid = fc.relnamespace
WHERE con.conrelid = ANY($1) AND con.contype IN ('p', 'u', 'c', 'f')
ORDER BY con.conrelid, con.conname";

/// Indexes that are not a constraint's own.
const INDEXES: &str = r"
SELECT i.indrelid AS oid, c.relname::text AS name, i.indisunique AS unique,
       pg_get_indexdef(i.indexrelid) AS definition
FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid
WHERE i.indrelid = ANY($1)
  AND NOT EXISTS (SELECT 1 FROM pg_constraint con
                  WHERE con.conindid = i.indexrelid AND con.conrelid = i.indrelid
                    AND con.contype IN ('p', 'u', 'x'))
ORDER BY i.indrelid, c.relname";

const VIEWS: &str = r"SELECT c.oid, pg_get_viewdef(c.oid) AS definition
FROM pg_class c WHERE c.oid = ANY($1) ORDER BY c.oid";

/// Tables and views of the same schema each view reads, and which of their columns, through
/// its rewrite rule.
const VIEW_DEPENDENCIES: &str = r"
SELECT DISTINCT r.ev_class AS oid, c.relname::text AS name, a.attname::text AS column_name
FROM pg_rewrite r
JOIN pg_depend d ON d.classid = 'pg_rewrite'::regclass AND d.objid = r.oid
                AND d.refclassid = 'pg_class'::regclass
JOIN pg_class c ON c.oid = d.refobjid
JOIN pg_class v ON v.oid = r.ev_class
LEFT JOIN pg_attribute a ON a.attrelid = d.refobjid AND a.attnum = d.refobjsubid
                         AND d.refobjsubid > 0
WHERE r.ev_class = ANY($1) AND c.oid <> r.ev_class AND c.relnamespace = v.relnamespace
ORDER BY 1, 2, 3";

#[derive(FromRow)]
struct DependencyRow {
    oid: Oid,
    name: String,
    column_name: Option<String>,
}

/// What is in the schema but not compared, counted.
const OTHER: &str = r"
SELECT
  (SELECT count(*) FROM pg_proc p WHERE p.pronamespace = n.oid) AS functions,
  (SELECT count(*) FROM pg_type t
   WHERE t.typnamespace = n.oid
     AND (t.typtype IN ('e', 'd', 'r', 'm')
          OR (t.typtype = 'c' AND EXISTS (SELECT 1 FROM pg_class tc
                                          WHERE tc.oid = t.typrelid AND tc.relkind = 'c'))))
    AS types,
  (SELECT count(*) FROM pg_class s
   WHERE s.relnamespace = n.oid AND s.relkind = 'S'
     AND NOT EXISTS (SELECT 1 FROM pg_depend d
                     WHERE d.classid = 'pg_class'::regclass AND d.objid = s.oid
                       AND d.deptype IN ('a', 'i'))) AS sequences,
  (SELECT count(*) FROM pg_trigger tg JOIN pg_class tc ON tc.oid = tg.tgrelid
   WHERE tc.relnamespace = n.oid AND NOT tg.tgisinternal) AS triggers,
  (SELECT count(*) FROM pg_policy pol JOIN pg_class pc ON pc.oid = pol.polrelid
   WHERE pc.relnamespace = n.oid) AS policies,
  (SELECT count(*) FROM pg_constraint x JOIN pg_class xc ON xc.oid = x.conrelid
   WHERE xc.relnamespace = n.oid AND x.contype = 'x') AS exclusions
FROM pg_namespace n WHERE n.nspname = $1";

fn referential(code: &str) -> ReferentialAction {
    match code {
        "r" => ReferentialAction::Restrict,
        "c" => ReferentialAction::Cascade,
        "n" => ReferentialAction::SetNull,
        "d" => ReferentialAction::SetDefault,
        _ => ReferentialAction::NoAction,
    }
}

fn counted(count: i64, one: &str, many: &str) -> Option<String> {
    match count {
        0 => None,
        1 => Some(format!("1 {one}")),
        n => Some(format!("{n} {many}")),
    }
}

/// Reads `schema` for comparison.
pub(super) async fn read(
    connection: &mut PgConnection,
    schema: &str,
) -> Result<Result<SchemaModel, DbError>, sqlx::Error> {
    let mut transaction = connection.begin_with("BEGIN READ ONLY").await?;
    sqlx::query("SET LOCAL search_path = pg_catalog")
        .execute(&mut *transaction)
        .await?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = $1)")
            .bind(schema)
            .fetch_one(&mut *transaction)
            .await?;
    if !exists {
        return Ok(Err(DbError::new(
            DbErrorKind::ObjectNotFound,
            format!("There is no schema named {schema}."),
        )));
    }
    let limit = i64::try_from(MAX_SCHEMA_OBJECTS + 1).unwrap_or(i64::MAX);
    let relations: Vec<RelationRow> = sqlx::query_as(RELATIONS)
        .bind(schema)
        .bind(limit)
        .fetch_all(&mut *transaction)
        .await?;
    if relations.len() > MAX_SCHEMA_OBJECTS {
        return Ok(Err(DbError::new(
            DbErrorKind::InvalidRequest,
            format!(
                "{schema} has more than {MAX_SCHEMA_OBJECTS} tables and views, more than aidb \
                 compares."
            ),
        )));
    }

    let mut not_compared = Vec::new();
    let mut tables: Vec<Table> = Vec::new();
    let mut table_oids: Vec<Oid> = Vec::new();
    let mut view_oids: Vec<(Oid, String)> = Vec::new();
    for relation in relations {
        match (relation.kind.as_str(), relation.partition) {
            (_, true) => {}
            ("r", false) => {
                table_oids.push(relation.oid);
                tables.push(Table {
                    name: relation.name,
                    rows: relation.rows.and_then(|rows| u64::try_from(rows).ok()),
                    ..Table::default()
                });
            }
            ("v", false) => view_oids.push((relation.oid, relation.name)),
            ("p", false) => not_compared.push(format!("partitioned table {}", relation.name)),
            ("m", false) => not_compared.push(format!("materialized view {}", relation.name)),
            ("f", false) => not_compared.push(format!("foreign table {}", relation.name)),
            _ => {}
        }
    }
    let position: HashMap<Oid, usize> = table_oids
        .iter()
        .enumerate()
        .map(|(index, oid)| (*oid, index))
        .collect();

    let columns: Vec<ColumnRow> = sqlx::query_as(COLUMNS)
        .bind(&table_oids)
        .fetch_all(&mut *transaction)
        .await?;
    for row in columns {
        let Some(&index) = position.get(&row.oid) else {
            continue;
        };
        let serial = row.serial
            && row
                .default_value
                .as_deref()
                .is_some_and(|d| d.starts_with("nextval("));
        tables[index].columns.push(Column {
            name: row.name,
            data_type: row.data_type,
            nullable: row.nullable,
            default: row.default_value,
            identity: match row.identity.as_str() {
                "a" => Some(Identity::Always),
                "d" => Some(Identity::ByDefault),
                _ => None,
            },
            generated: row.generated,
            serial,
        });
    }

    let constraints: Vec<ConstraintRow> = sqlx::query_as(CONSTRAINTS)
        .bind(&table_oids)
        .fetch_all(&mut *transaction)
        .await?;
    for row in constraints {
        let Some(&index) = position.get(&row.oid) else {
            continue;
        };
        let kind = match row.kind.as_str() {
            "p" => ConstraintKind::PrimaryKey(row.columns),
            "u" => ConstraintKind::Unique(row.columns),
            "c" => ConstraintKind::Check(row.expression.unwrap_or_default()),
            _ => ConstraintKind::ForeignKey(ForeignKeyDefinition {
                columns: row.columns,
                schema: row.ref_schema,
                table: row.ref_table.unwrap_or_default(),
                referenced: row.referenced,
                on_update: referential(&row.on_update),
                on_delete: referential(&row.on_delete),
            }),
        };
        tables[index].constraints.push(Constraint {
            name: Some(row.name),
            kind,
        });
    }

    let indexes: Vec<IndexRow> = sqlx::query_as(INDEXES)
        .bind(&table_oids)
        .fetch_all(&mut *transaction)
        .await?;
    for row in indexes {
        let Some(&index) = position.get(&row.oid) else {
            continue;
        };
        // `CREATE INDEX name ON schema.table USING ...`: keep what follows the table.
        let definition = row
            .definition
            .find(" USING ")
            .map_or(row.definition.as_str(), |at| &row.definition[at + 1..])
            .to_owned();
        tables[index].indexes.push(Index {
            name: Some(row.name),
            unique: row.unique,
            definition,
        });
    }

    let oids: Vec<Oid> = view_oids.iter().map(|(oid, _)| *oid).collect();
    let definitions: HashMap<Oid, Option<String>> = sqlx::query_as::<_, ViewRow>(VIEWS)
        .bind(&oids)
        .fetch_all(&mut *transaction)
        .await?
        .into_iter()
        .map(|row| (row.oid, row.definition))
        .collect();
    let mut depends_on: HashMap<Oid, (Vec<String>, Vec<String>)> = HashMap::new();
    for row in sqlx::query_as::<_, DependencyRow>(VIEW_DEPENDENCIES)
        .bind(&oids)
        .fetch_all(&mut *transaction)
        .await?
    {
        let (relations, columns) = depends_on.entry(row.oid).or_default();
        if !relations.contains(&row.name) {
            relations.push(row.name.clone());
        }
        if let Some(column) = row.column_name {
            columns.push(format!("{}.{column}", row.name));
        }
    }
    let views = view_oids
        .into_iter()
        .map(|(oid, name)| {
            let (depends_on, reads) = depends_on.remove(&oid).unwrap_or_default();
            (oid, name, depends_on, reads)
        })
        .map(|(oid, name, depends_on, reads)| View {
            depends_on,
            reads,
            name,
            definition: definitions
                .get(&oid)
                .cloned()
                .flatten()
                .unwrap_or_default()
                .trim_end()
                .trim_end_matches(';')
                .to_owned(),
        })
        .collect();

    let other: OtherRow = sqlx::query_as(OTHER)
        .bind(schema)
        .fetch_one(&mut *transaction)
        .await?;
    not_compared.extend(
        [
            counted(other.functions, "function", "functions"),
            counted(other.types, "type", "types"),
            counted(
                other.sequences,
                "sequence not owned by a column",
                "sequences not owned by a column",
            ),
            counted(other.triggers, "trigger", "triggers"),
            counted(
                other.policies,
                "row security policy",
                "row security policies",
            ),
            counted(
                other.exclusions,
                "exclusion constraint",
                "exclusion constraints",
            ),
        ]
        .into_iter()
        .flatten(),
    );
    transaction.rollback().await?;

    tables.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Ok(SchemaModel {
        schema: schema.to_owned(),
        tables,
        views,
        not_compared,
        stored_text: true,
    }))
}
