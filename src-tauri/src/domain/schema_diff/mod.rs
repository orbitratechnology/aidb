//! Schema comparison: two schemas, each read from a database or from a `.sql` file, as a model of
//! tables (columns, constraints, indexes) and views; the steps that turn one (the target) into the
//! other (the source), as SQL; and what each step risks: lost data, long locks, failures, and
//! statements that cannot run in a transaction.
//!
//! Nothing here runs SQL. The steps are text for review; applying them is a separate, approved
//! operation.

pub mod generate;
pub mod normalize;
pub mod parse;

use serde::{Deserialize, Serialize};

use crate::domain::catalog::{Identity, ReferentialAction};
use crate::domain::policy::risk::RiskLevel;

/// The most tables and views a schema may have to be compared.
pub const MAX_SCHEMA_OBJECTS: usize = 5_000;

/// The largest `.sql` file read for comparison.
pub const MAX_SCHEMA_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// A schema's tables and views, as compared. Expressions and types are kept as written where
/// they were read (the database's own text, or the file's), with names in the schema itself
/// either qualified or not; comparing removes that difference.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SchemaModel {
    pub schema: String,
    pub tables: Vec<Table>,
    /// In creation order (the database) or file order, so dependent views come later.
    pub views: Vec<View>,
    /// What the model leaves out, in words ("2 functions", "materialized view totals").
    pub not_compared: Vec<String>,
    /// Whether view queries and check conditions are as PostgreSQL stores them (read from a
    /// database). False for a file: PostgreSQL rewrites them when it stores them, so a file's
    /// text cannot be compared with a database's.
    pub stored_text: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Table {
    pub name: String,
    pub columns: Vec<Column>,
    pub constraints: Vec<Constraint>,
    /// Indexes that are not a constraint's own (primary key and unique constraints have one).
    pub indexes: Vec<Index>,
    /// The planner's row estimate, when read from a database that has one.
    pub rows: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    /// As `format_type` prints it (see [`normalize::canonical_type`]).
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub identity: Option<Identity>,
    /// The expression of a stored generated column.
    pub generated: Option<String>,
    /// Filled from a sequence the column owns (`serial`): its default is that sequence's
    /// `nextval`.
    pub serial: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constraint {
    /// Absent for a file's unnamed constraint (PostgreSQL names it when it is created).
    pub name: Option<String>,
    pub kind: ConstraintKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstraintKind {
    PrimaryKey(Vec<String>),
    Unique(Vec<String>),
    /// The condition, without the `CHECK` keyword.
    Check(String),
    ForeignKey(ForeignKeyDefinition),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignKeyDefinition {
    pub columns: Vec<String>,
    /// The referenced table's schema, as written; `None` for the same schema, unqualified.
    pub schema: Option<String>,
    pub table: String,
    pub referenced: Vec<String>,
    pub on_update: ReferentialAction,
    pub on_delete: ReferentialAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
    /// Absent for a file's unnamed index.
    pub name: Option<String>,
    pub unique: bool,
    /// Everything from `USING`: method, keys, `INCLUDE`, `WHERE`.
    pub definition: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub name: String,
    /// The query, without a trailing semicolon.
    pub definition: String,
    /// Tables and views of the same schema it reads, when known (read from a database).
    pub depends_on: Vec<String>,
    /// Their columns it reads, as `table.column`, when known.
    pub reads: Vec<String>,
}

/// Where the source of a comparison comes from.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CompareSource {
    /// A schema on the open connection.
    Schema { schema: String },
    /// A schema on another saved connection, read through a short read-only session.
    Connection {
        #[serde(rename = "connectionId")]
        connection_id: crate::domain::connections::ConnectionId,
        schema: String,
    },
    /// A schema in a `.sql` file opened for comparison.
    File {
        #[serde(rename = "fileId")]
        #[cfg_attr(test, ts(type = "string"))]
        file_id: uuid::Uuid,
        schema: String,
    },
}

/// What to compare: a schema on the open connection (the target, which the steps change) with
/// a source it should come to match.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompareRequest {
    pub target_schema: String,
    pub source: CompareSource,
    #[serde(default)]
    pub options: GenerateOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GenerateOptions {
    /// Create and drop indexes `CONCURRENTLY`: no write lock, but outside the transaction.
    pub concurrent_indexes: bool,
}

/// A `.sql` file read for comparison, as the webview sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SchemaFile {
    #[cfg_attr(test, ts(type = "string"))]
    pub file_id: uuid::Uuid,
    pub file_name: String,
    /// Schemas the file names, in order of appearance; unqualified names count as `public`.
    pub schemas: Vec<String>,
    pub statements: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum StepAction {
    Add,
    Remove,
    Change,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum NoteKind {
    /// Deletes rows or values.
    DataLoss,
    /// Locks the table, or rewrites it, while it runs.
    Lock,
    /// Can fail on the data or objects that are there.
    MayFail,
    /// Cannot run inside a transaction, so it is not rolled back with the others.
    NonTransactional,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct StepNote {
    pub kind: NoteKind,
    pub text: String,
}

/// One statement of the migration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct MigrationStep {
    /// "Add column price to products".
    pub summary: String,
    pub action: StepAction,
    /// The table or view it changes.
    pub object: String,
    pub sql: String,
    /// False for `CONCURRENTLY` index statements, which run after the transaction.
    pub transactional: bool,
    /// The policy's risk for the statement on the target connection.
    pub risk: RiskLevel,
    pub notes: Vec<StepNote>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct DiffSummary {
    pub tables_added: usize,
    pub tables_removed: usize,
    pub tables_changed: usize,
    pub views_added: usize,
    pub views_removed: usize,
    pub views_changed: usize,
}

/// A comparison and the migration it gives. Held by the core under `diff_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SchemaDiff {
    #[cfg_attr(test, ts(type = "string"))]
    pub diff_id: uuid::Uuid,
    /// "public on Staging", "schema.sql (public)".
    pub source: String,
    /// "public on Production".
    pub target: String,
    pub target_schema: String,
    pub summary: DiffSummary,
    /// In the order they run: the transactional ones, then the others.
    pub steps: Vec<MigrationStep>,
    /// The whole migration as one script, empty when the schemas match.
    pub script: String,
    /// What was not compared, and file statements that were not read.
    pub notes: Vec<String>,
    /// The highest step risk; `R0` when there is nothing to do.
    pub risk: RiskLevel,
}

#[cfg(test)]
mod tests;
