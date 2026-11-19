//! Driver-neutral description of a database's structure: schemas, tables and views, their
//! columns, keys and indexes. Never row data.

use serde::{Deserialize, Serialize};

/// Most objects listed in an overview; larger catalogs are cut short and say so.
pub const MAX_OVERVIEW_OBJECTS: usize = 20_000;
/// Longest view definition returned, in bytes.
pub const MAX_DEFINITION_BYTES: usize = 100 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Table,
    PartitionedTable,
    View,
    MaterializedView,
    ForeignTable,
}

impl ObjectKind {
    /// Views are stored queries; everything else holds rows.
    #[must_use]
    pub const fn is_view(self) -> bool {
        matches!(self, Self::View | Self::MaterializedView)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SchemaSummary {
    pub name: String,
    pub comment: Option<String>,
    /// The session's role may look inside the schema.
    pub accessible: bool,
}

/// One table or view, as listed in the object tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ObjectSummary {
    pub schema: String,
    pub name: String,
    pub kind: ObjectKind,
    /// The planner's row estimate; `None` before the table was first analyzed.
    #[cfg_attr(test, ts(type = "number | null"))]
    pub row_estimate: Option<u64>,
    pub comment: Option<String>,
    /// The session's role may read it.
    pub readable: bool,
}

/// Everything listed in the object tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct CatalogOverview {
    pub database: String,
    /// Changes whenever the structure changes (not when statistics are refreshed).
    pub fingerprint: String,
    pub schemas: Vec<SchemaSummary>,
    pub objects: Vec<ObjectSummary>,
    /// More than [`MAX_OVERVIEW_OBJECTS`] objects exist; the rest are not listed.
    pub truncated: bool,
}

/// Names a table or view.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectName {
    pub schema: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum Identity {
    Always,
    ByDefault,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub name: String,
    /// 1-based position in the table.
    pub position: u32,
    /// The type as PostgreSQL prints it, such as `character varying(80)`.
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub identity: Option<Identity>,
    /// Expression of a generated column.
    pub generated: Option<String>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct KeyConstraint {
    pub name: String,
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ReferentialAction {
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

/// A foreign key between two tables.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ForeignKey {
    pub name: String,
    /// The table holding the key.
    pub from: ObjectName,
    pub columns: Vec<String>,
    /// The table it points to.
    pub to: ObjectName,
    pub referenced_columns: Vec<String>,
    pub on_update: ReferentialAction,
    pub on_delete: ReferentialAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct Index {
    pub name: String,
    /// `CREATE INDEX …` as PostgreSQL prints it.
    pub definition: String,
    pub unique: bool,
    pub primary: bool,
    /// Access method, such as `btree` or `gin`.
    pub method: String,
}

/// A table's or view's full definition and relationships.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ObjectDetails {
    pub summary: ObjectSummary,
    pub columns: Vec<Column>,
    pub primary_key: Option<KeyConstraint>,
    pub unique_constraints: Vec<KeyConstraint>,
    /// Keys in this table that point elsewhere.
    pub foreign_keys: Vec<ForeignKey>,
    /// Keys in other tables that point here.
    pub referenced_by: Vec<ForeignKey>,
    pub indexes: Vec<Index>,
    /// The query behind a view, cut to [`MAX_DEFINITION_BYTES`].
    pub definition: Option<String>,
}

/// Cuts `text` to at most `max` bytes on a character boundary. Returns whether it was cut.
#[must_use]
pub fn truncate_on_char_boundary(text: &mut String, max: usize) -> bool {
    if text.len() <= max {
        return false;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_keeps_whole_characters() {
        let mut text = "ééé".to_owned();

        assert!(truncate_on_char_boundary(&mut text, 3));
        assert_eq!(text, "é");

        let mut short = "abc".to_owned();
        assert!(!truncate_on_char_boundary(&mut short, 3));
        assert_eq!(short, "abc");
    }

    #[test]
    fn object_names_reject_unknown_fields() {
        let parsed = serde_json::from_value::<ObjectName>(serde_json::json!({
            "schema": "public", "name": "orders", "sql": "drop table orders"
        }));

        assert!(parsed.is_err());
    }
}
