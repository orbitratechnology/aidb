//! Changes to a table's rows (INSERT, UPDATE, DELETE), described as data rather than SQL.
//!
//! A change names its table and columns as plain strings and carries its values as text. None
//! of it is put into SQL as written: the adapter matches the names against the live table,
//! writes the statement from the database's own quoting of them, and binds every value as a
//! parameter that the database converts to the column's type. An UPDATE or DELETE must have at
//! least one filter, so a change can never address a whole table by omission.

use serde::{Deserialize, Serialize};

use super::audit::Actor;
use super::catalog::ObjectName;
use super::connections::ConnectionId;
use super::policy::approval::ApprovalId;
use super::policy::risk::{ApprovalLevel, RiskLevel};
use super::table_data::{
    CellRef, Filter, RequestError, TablePage, truncate_cell, validate_filters,
};
use crate::errors::AppError;

/// Most rows one INSERT may add.
pub const MAX_INSERT_ROWS: usize = 1_000;
/// Most columns one change may set.
pub const MAX_CHANGE_COLUMNS: usize = 200;
/// Most values in one change (rows × columns), well within the protocol's parameter limit.
pub const MAX_CHANGE_VALUES: usize = 10_000;
/// Largest single value, in bytes.
pub const MAX_VALUE_BYTES: usize = 1024 * 1024;
/// Largest total of all values, in bytes.
pub const MAX_CHANGE_BYTES: usize = 8 * 1024 * 1024;
/// Most rows an UPDATE or DELETE may match. Larger changes are refused at preview, so every
/// change aidb applies is counted exactly and verified row by row.
pub const MAX_CHANGED_ROWS: u64 = 10_000;
/// Most changes applied together, in one transaction under one approval.
pub const MAX_SET_CHANGES: usize = 200;
/// Changes whose matching rows a preview reads as they are now; later ones show counts only.
pub const PREVIEW_CHANGES: usize = 20;
/// Limits on a set of changes applied together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetLimits {
    pub max_changes: usize,
    pub max_values: usize,
    pub max_bytes: usize,
    /// Rows written by all the changes together.
    pub max_rows: u64,
}

/// Limits for edits made in the grid or proposed by the assistant.
pub const EDIT_LIMITS: SetLimits = SetLimits {
    max_changes: MAX_SET_CHANGES,
    max_values: MAX_CHANGE_VALUES,
    max_bytes: MAX_CHANGE_BYTES,
    max_rows: MAX_CHANGED_ROWS,
};

/// Limits for a file imported in batches of inserts.
pub const IMPORT_LIMITS: SetLimits = SetLimits {
    max_changes: 2_000,
    max_values: 5_000_000,
    max_bytes: 64 * 1024 * 1024,
    max_rows: 100_000,
};

/// Rows shown in a preview, before and after.
pub const PREVIEW_ROWS: u32 = 20;
/// Size limit of the "before" rows read for a preview.
pub const PREVIEW_BYTES: usize = 1024 * 1024;

/// What kind of change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Insert,
    Update,
    Delete,
}

impl ChangeKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Insert => "insert",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }

    /// "Insert", "Update" or "Delete", for messages.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Insert => "Insert",
            Self::Update => "Update",
            Self::Delete => "Delete",
        }
    }
}

/// One column and the value to store in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColumnValue {
    pub column: String,
    /// Text the database converts to the column's type; `null` stores SQL NULL.
    pub value: Option<String>,
}

/// A change to a table's rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DataChange {
    /// Adds `rows`, each with one value per entry of `columns`. Columns left out get their
    /// defaults. With `upsert_key`, a row whose key (a primary key or unique constraint) already
    /// exists updates that row's other given columns instead.
    Insert {
        table: ObjectName,
        columns: Vec<String>,
        rows: Vec<Vec<Option<String>>>,
        #[serde(default, rename = "upsertKey", skip_serializing_if = "Option::is_none")]
        #[cfg_attr(test, ts(optional))]
        upsert_key: Option<Vec<String>>,
    },
    /// Sets columns in every row matching all `filters`.
    Update {
        table: ObjectName,
        set: Vec<ColumnValue>,
        filters: Vec<Filter>,
    },
    /// Deletes every row matching all `filters`.
    Delete {
        table: ObjectName,
        filters: Vec<Filter>,
    },
}

/// Why a change was refused before reaching the database.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChangeError {
    #[error("Choose at least one column to {0}.")]
    NoColumns(&'static str),
    #[error("Change at most {MAX_CHANGE_COLUMNS} columns at once.")]
    TooManyColumns,
    #[error("The column {0} is listed more than once.")]
    DuplicateColumn(String),
    #[error("Add at least one row.")]
    NoRows,
    #[error("Insert at most {MAX_INSERT_ROWS} rows at once.")]
    TooManyRows,
    #[error("Row {0} has {1} values for {2} columns.")]
    RowWidth(usize, usize, usize),
    #[error(
        "{0} needs at least one filter, so it cannot change every row of the table by \
         accident."
    )]
    NoFilter(&'static str),
    #[error("A change can hold at most {MAX_CHANGE_VALUES} values.")]
    TooManyValues,
    #[error("A value can be at most {} MB.", MAX_VALUE_BYTES / (1024 * 1024))]
    ValueTooLarge,
    #[error("The values are too large to change at once.")]
    ChangeTooLarge,
    #[error("Values cannot contain NUL characters.")]
    NulInValue,
    #[error(transparent)]
    Filter(#[from] RequestError),
    #[error("Add at least one change.")]
    NoChanges,
    #[error("Apply at most {0} changes at once.")]
    TooManyChanges(usize),
    #[error("The key to update by must be a list of the inserted columns, each once.")]
    BadUpsertKey,
    #[error("An update by key needs at least one inserted column that is not part of the key.")]
    UpsertUpdatesNothing,
    #[error("Change {0} of {1}: {2}")]
    InChange(usize, usize, Box<ChangeError>),
}

impl From<ChangeError> for AppError {
    fn from(error: ChangeError) -> Self {
        Self::invalid_input(error.to_string())
    }
}

impl DataChange {
    #[must_use]
    pub const fn kind(&self) -> ChangeKind {
        match self {
            Self::Insert { .. } => ChangeKind::Insert,
            Self::Update { .. } => ChangeKind::Update,
            Self::Delete { .. } => ChangeKind::Delete,
        }
    }

    #[must_use]
    pub const fn table(&self) -> &ObjectName {
        match self {
            Self::Insert { table, .. }
            | Self::Update { table, .. }
            | Self::Delete { table, .. } => table,
        }
    }

    /// Every value the change stores (not its filters' values).
    fn values(&self) -> Box<dyn Iterator<Item = &Option<String>> + '_> {
        match self {
            Self::Insert { rows, .. } => Box::new(rows.iter().flatten()),
            Self::Update { set, .. } => Box::new(set.iter().map(|value| &value.value)),
            Self::Delete { .. } => Box::new(std::iter::empty()),
        }
    }

    /// The filters rows must match; empty for an INSERT.
    #[must_use]
    pub fn filters(&self) -> &[Filter] {
        match self {
            Self::Insert { .. } => &[],
            Self::Update { filters, .. } | Self::Delete { filters, .. } => filters,
        }
    }

    /// Checks the change's shape: columns, rows, filters and the size of its values. Names are
    /// checked later, against the live table.
    ///
    /// # Errors
    ///
    /// The first problem found.
    pub fn validate(&self) -> Result<(), ChangeError> {
        match self {
            Self::Insert {
                columns,
                rows,
                upsert_key,
                ..
            } => {
                check_columns(columns.iter().map(String::as_str), "insert")?;
                if let Some(key) = upsert_key {
                    let mut unique = key.clone();
                    unique.sort();
                    unique.dedup();
                    if key.is_empty()
                        || unique.len() != key.len()
                        || key.iter().any(|column| !columns.contains(column))
                    {
                        return Err(ChangeError::BadUpsertKey);
                    }
                    if columns.iter().all(|column| key.contains(column)) {
                        return Err(ChangeError::UpsertUpdatesNothing);
                    }
                }
                if rows.is_empty() {
                    return Err(ChangeError::NoRows);
                }
                if rows.len() > MAX_INSERT_ROWS {
                    return Err(ChangeError::TooManyRows);
                }
                if let Some((index, row)) = rows
                    .iter()
                    .enumerate()
                    .find(|(_, row)| row.len() != columns.len())
                {
                    return Err(ChangeError::RowWidth(index + 1, row.len(), columns.len()));
                }
                if rows.len() * columns.len() > MAX_CHANGE_VALUES {
                    return Err(ChangeError::TooManyValues);
                }
                check_values(rows.iter().flatten(), MAX_CHANGE_BYTES)
            }
            Self::Update { set, filters, .. } => {
                check_columns(set.iter().map(|value| value.column.as_str()), "update")?;
                check_values(set.iter().map(|value| &value.value), MAX_CHANGE_BYTES)?;
                check_filters(filters, "An UPDATE")
            }
            Self::Delete { filters, .. } => check_filters(filters, "A DELETE"),
        }
    }
}

/// Checks a set of changes applied together: each one, and the limits on values across all.
///
/// # Errors
///
/// The first problem found, naming the change when there are several.
pub fn validate_set(changes: &[DataChange], limits: SetLimits) -> Result<(), ChangeError> {
    if changes.is_empty() {
        return Err(ChangeError::NoChanges);
    }
    if changes.len() > limits.max_changes {
        return Err(ChangeError::TooManyChanges(limits.max_changes));
    }
    let total = changes.len();
    for (index, change) in changes.iter().enumerate() {
        change.validate().map_err(|error| {
            if total == 1 {
                error
            } else {
                ChangeError::InChange(index + 1, total, Box::new(error))
            }
        })?;
    }
    let values = || changes.iter().flat_map(DataChange::values);
    if values().count() > limits.max_values {
        return Err(ChangeError::TooManyValues);
    }
    check_values(values(), limits.max_bytes)
}

fn check_columns<'a>(
    columns: impl ExactSizeIterator<Item = &'a str> + Clone,
    action: &'static str,
) -> Result<(), ChangeError> {
    if columns.len() == 0 {
        return Err(ChangeError::NoColumns(action));
    }
    if columns.len() > MAX_CHANGE_COLUMNS {
        return Err(ChangeError::TooManyColumns);
    }
    for (index, column) in columns.clone().enumerate() {
        if columns.clone().take(index).any(|earlier| earlier == column) {
            return Err(ChangeError::DuplicateColumn(column.to_owned()));
        }
    }
    Ok(())
}

fn check_values<'a>(
    values: impl Iterator<Item = &'a Option<String>>,
    max_bytes: usize,
) -> Result<(), ChangeError> {
    let mut total = 0_usize;
    for value in values.flatten() {
        if value.len() > MAX_VALUE_BYTES {
            return Err(ChangeError::ValueTooLarge);
        }
        if value.contains('\0') {
            return Err(ChangeError::NulInValue);
        }
        total += value.len();
        if total > max_bytes {
            return Err(ChangeError::ChangeTooLarge);
        }
    }
    Ok(())
}

fn check_filters(filters: &[Filter], statement: &'static str) -> Result<(), ChangeError> {
    if filters.is_empty() {
        return Err(ChangeError::NoFilter(statement));
    }
    validate_filters(filters)?;
    Ok(())
}

/// A change made ready to run: the exact statement, its parameters and what the database says
/// it would do now. Built from the live table; nothing in it has run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataChangePlan {
    /// The statement exactly as it will run, with `$n` placeholders.
    pub statement: String,
    /// The statement's parameters, in `$n` order.
    pub parameters: Vec<Option<String>>,
    /// Rows the change writes: the rows given for an INSERT, the rows matching now (counted,
    /// not estimated) for an UPDATE or DELETE.
    pub rows: u64,
    /// The columns of `values`.
    pub value_columns: Vec<String>,
    /// The values as the database will store them, as text: one row for an UPDATE, every row
    /// for an INSERT, none for a DELETE.
    pub values: Vec<Vec<Option<String>>>,
    /// Some of the rows an UPDATE or DELETE matches, as they are now, when asked for.
    pub before: Option<TablePage>,
}

/// Values shown in a preview: the first rows, cut like table cells.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ValueSample {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    pub truncated_cells: Vec<CellRef>,
    /// Rows in the change; more than `rows` when the sample was cut.
    #[cfg_attr(test, ts(type = "number"))]
    pub total_rows: u64,
}

impl ValueSample {
    /// The first [`PREVIEW_ROWS`] rows of `values`, with long values cut.
    #[must_use]
    pub fn of(columns: &[String], values: &[Vec<Option<String>>]) -> Self {
        let mut truncated_cells = Vec::new();
        let rows = values
            .iter()
            .take(PREVIEW_ROWS as usize)
            .enumerate()
            .map(|(row, cells)| {
                cells
                    .iter()
                    .enumerate()
                    .map(|(column, cell)| {
                        let mut cell = cell.clone();
                        if let Some(text) = &mut cell
                            && truncate_cell(text)
                        {
                            truncated_cells.push(CellRef {
                                row: u32::try_from(row).unwrap_or(u32::MAX),
                                column: u32::try_from(column).unwrap_or(u32::MAX),
                            });
                        }
                        cell
                    })
                    .collect()
            })
            .collect();
        Self {
            columns: columns.to_vec(),
            rows,
            truncated_cells,
            total_rows: values.len() as u64,
        }
    }
}

/// One change of a preview, as planned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct PlannedChange {
    pub kind: ChangeKind,
    pub table: ObjectName,
    /// The statement that will run, with `$n` placeholders for the values.
    pub statement: String,
    /// Rows it writes: counted now for an UPDATE or DELETE.
    #[cfg_attr(test, ts(type = "number"))]
    pub rows: u64,
    /// Matching rows as they are now (UPDATE, DELETE), for the first [`PREVIEW_CHANGES`].
    pub before: Option<TablePage>,
    /// The values as they will be stored (INSERT, UPDATE).
    pub after: Option<ValueSample>,
}

/// What the user reviews before approving a set of changes, applied together. The approval it
/// names is held by the core; approving it is the only way to apply the changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct DataChangePreview {
    #[cfg_attr(test, ts(type = "string"))]
    pub approval_id: ApprovalId,
    /// In the order they will run.
    pub changes: Vec<PlannedChange>,
    /// Rows written by all of them.
    #[cfg_attr(test, ts(type = "number"))]
    pub rows: u64,
    pub risk: RiskLevel,
    pub approval: ApprovalLevel,
    /// Why it has its risk, in words.
    pub reasons: Vec<String>,
    /// What the user must type to approve (the database name), for elevated approvals.
    pub confirmation: Option<String>,
    /// When the preview lapses, RFC 3339.
    pub expires_at: String,
}

/// Changes held for the user's approval: what the review shows, who proposed them and where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct PendingChange {
    pub connection_id: ConnectionId,
    /// The user (grid edits) or the assistant (a proposal).
    pub actor: Actor,
    pub preview: DataChangePreview,
    /// The changes as requested, in the order they run.
    pub changes: Vec<DataChange>,
}

/// How an approved change ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ApplyStatus {
    /// Written, checked and committed.
    Applied,
    /// Rolled back: nothing was written.
    RolledBack,
    /// The connection was lost while committing, so it may or may not have been written.
    Uncertain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct DataChangeResult {
    pub status: ApplyStatus,
    /// Rows written, when applied.
    #[cfg_attr(test, ts(type = "number | null"))]
    pub rows_affected: Option<u64>,
    /// Why it was rolled back or is uncertain.
    pub message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::table_data::FilterOperator;

    fn table() -> ObjectName {
        ObjectName {
            schema: "public".into(),
            name: "orders".into(),
        }
    }

    fn equals(column: &str, value: &str) -> Filter {
        Filter {
            column: column.into(),
            operator: FilterOperator::Equals,
            value: Some(value.into()),
        }
    }

    fn set(column: &str, value: Option<&str>) -> ColumnValue {
        ColumnValue {
            column: column.into(),
            value: value.map(Into::into),
        }
    }

    #[test]
    fn updates_and_deletes_need_a_filter() {
        let update = DataChange::Update {
            table: table(),
            set: vec![set("status", Some("shipped"))],
            filters: Vec::new(),
        };
        assert_eq!(update.validate(), Err(ChangeError::NoFilter("An UPDATE")));
        let delete = DataChange::Delete {
            table: table(),
            filters: Vec::new(),
        };
        assert_eq!(delete.validate(), Err(ChangeError::NoFilter("A DELETE")));

        let filtered = DataChange::Delete {
            table: table(),
            filters: vec![equals("id", "7")],
        };
        assert_eq!(filtered.validate(), Ok(()));
        // Filters are checked like the grid's.
        let missing = DataChange::Delete {
            table: table(),
            filters: vec![Filter {
                column: "id".into(),
                operator: FilterOperator::Equals,
                value: None,
            }],
        };
        assert_eq!(
            missing.validate(),
            Err(ChangeError::Filter(RequestError::MissingValue("id".into())))
        );
    }

    #[test]
    fn columns_and_rows_must_line_up() {
        let insert = |columns: &[&str], rows: Vec<Vec<Option<String>>>| DataChange::Insert {
            table: table(),
            columns: columns.iter().map(|c| (*c).to_owned()).collect(),
            rows,
            upsert_key: None,
        };
        let row = |values: &[&str]| values.iter().map(|v| Some((*v).to_owned())).collect();
        assert_eq!(
            insert(&["id", "name"], vec![row(&["1", "a"]), vec![None, None]]).validate(),
            Ok(())
        );
        assert_eq!(
            insert(&[], vec![row(&[])]).validate(),
            Err(ChangeError::NoColumns("insert"))
        );
        assert_eq!(
            insert(&["id"], Vec::new()).validate(),
            Err(ChangeError::NoRows)
        );
        assert_eq!(
            insert(&["id", "id"], vec![row(&["1", "2"])]).validate(),
            Err(ChangeError::DuplicateColumn("id".into()))
        );
        assert_eq!(
            insert(&["id", "name"], vec![row(&["1", "a"]), row(&["2"])]).validate(),
            Err(ChangeError::RowWidth(2, 1, 2))
        );
        assert_eq!(
            insert(&["id"], vec![row(&["1"]); MAX_INSERT_ROWS + 1]).validate(),
            Err(ChangeError::TooManyRows)
        );
        let wide: Vec<String> = (0..20).map(|i| format!("c{i}")).collect();
        let wide: Vec<&str> = wide.iter().map(String::as_str).collect();
        assert_eq!(
            insert(&wide, vec![vec![None; 20]; MAX_INSERT_ROWS]).validate(),
            Err(ChangeError::TooManyValues)
        );

        let update = DataChange::Update {
            table: table(),
            set: Vec::new(),
            filters: vec![equals("id", "1")],
        };
        assert_eq!(update.validate(), Err(ChangeError::NoColumns("update")));
    }

    #[test]
    fn values_are_bounded() {
        let update = |value: String| DataChange::Update {
            table: table(),
            set: vec![set("note", Some(&value))],
            filters: vec![equals("id", "1")],
        };
        assert_eq!(
            update("x".repeat(MAX_VALUE_BYTES + 1)).validate(),
            Err(ChangeError::ValueTooLarge)
        );
        assert_eq!(
            update("a\0b".into()).validate(),
            Err(ChangeError::NulInValue)
        );
        let big = Some("x".repeat(MAX_VALUE_BYTES));
        let insert = DataChange::Insert {
            table: table(),
            columns: vec!["note".into()],
            rows: vec![vec![big]; 9],
            upsert_key: None,
        };
        assert_eq!(insert.validate(), Err(ChangeError::ChangeTooLarge));
    }

    #[test]
    fn upserts_name_a_key_among_the_inserted_columns() {
        let upsert = |key: Vec<&str>| DataChange::Insert {
            table: table(),
            columns: vec!["id".into(), "name".into()],
            rows: vec![vec![Some("1".into()), Some("a".into())]],
            upsert_key: Some(key.into_iter().map(Into::into).collect()),
        };
        assert_eq!(upsert(vec!["id"]).validate(), Ok(()));
        assert_eq!(upsert(vec![]).validate(), Err(ChangeError::BadUpsertKey));
        assert_eq!(
            upsert(vec!["sku"]).validate(),
            Err(ChangeError::BadUpsertKey)
        );
        assert_eq!(
            upsert(vec!["id", "id"]).validate(),
            Err(ChangeError::BadUpsertKey)
        );
        assert_eq!(
            upsert(vec!["id", "name"]).validate(),
            Err(ChangeError::UpsertUpdatesNothing)
        );
        // Sent only when set.
        let json = serde_json::to_value(upsert(vec!["id"])).expect("json");
        assert_eq!(json["upsertKey"], serde_json::json!(["id"]));
        let plain: DataChange = serde_json::from_value(serde_json::json!({
            "kind": "insert", "table": {"schema": "public", "name": "orders"},
            "columns": ["id"], "rows": [["1"]]
        }))
        .expect("parses");
        assert!(
            serde_json::to_value(plain)
                .expect("json")
                .get("upsertKey")
                .is_none()
        );
    }

    #[test]
    fn imports_get_larger_limits() {
        let rows: Vec<Vec<Option<String>>> = vec![vec![Some("1".into())]; MAX_INSERT_ROWS];
        let batch = DataChange::Insert {
            table: table(),
            columns: vec!["id".into()],
            rows,
            upsert_key: None,
        };
        let batches = vec![batch; 50];
        assert_eq!(
            validate_set(&batches, EDIT_LIMITS),
            Err(ChangeError::TooManyValues)
        );
        assert_eq!(validate_set(&batches, IMPORT_LIMITS), Ok(()));
    }

    #[test]
    fn requests_are_tagged_by_kind() {
        let change: DataChange = serde_json::from_value(serde_json::json!({
            "kind": "update",
            "table": { "schema": "public", "name": "orders" },
            "set": [{ "column": "status", "value": null }],
            "filters": [{ "column": "id", "operator": "equals", "value": "1" }]
        }))
        .expect("parses");
        assert_eq!(change.kind(), ChangeKind::Update);
        assert_eq!(change.filters().len(), 1);

        // Unknown fields and kinds are refused.
        for bad in [
            serde_json::json!({ "kind": "delete", "table": { "schema": "s", "name": "t" },
                                "filters": [], "sql": "DROP TABLE t" }),
            serde_json::json!({ "kind": "truncate", "table": { "schema": "s", "name": "t" } }),
        ] {
            assert!(serde_json::from_value::<DataChange>(bad).is_err());
        }
    }

    #[test]
    fn sets_name_the_change_that_is_wrong_and_share_the_limits() {
        let delete = |filters: Vec<Filter>| DataChange::Delete {
            table: table(),
            filters,
        };
        assert_eq!(validate_set(&[], EDIT_LIMITS), Err(ChangeError::NoChanges));
        assert_eq!(
            validate_set(
                &vec![delete(vec![equals("id", "1")]); MAX_SET_CHANGES + 1],
                EDIT_LIMITS
            ),
            Err(ChangeError::TooManyChanges(MAX_SET_CHANGES))
        );
        assert_eq!(
            validate_set(
                &[delete(vec![equals("id", "1")]), delete(Vec::new())],
                EDIT_LIMITS
            ),
            Err(ChangeError::InChange(
                2,
                2,
                Box::new(ChangeError::NoFilter("A DELETE"))
            ))
        );
        assert_eq!(
            validate_set(&[delete(Vec::new())], EDIT_LIMITS),
            Err(ChangeError::NoFilter("A DELETE")),
            "a single change needs no number"
        );
        assert_eq!(
            ChangeError::InChange(2, 3, Box::new(ChangeError::NoRows)).to_string(),
            "Change 2 of 3: Add at least one row."
        );
        // The value limits hold across the set, not only per change.
        let big = Some("x".repeat(MAX_VALUE_BYTES));
        let update = DataChange::Update {
            table: table(),
            set: vec![set("note", big.as_deref())],
            filters: vec![equals("id", "1")],
        };
        assert_eq!(
            validate_set(&vec![update; 9], EDIT_LIMITS),
            Err(ChangeError::ChangeTooLarge)
        );
    }

    /// The UI builds changes within the same limits; CI fails if this copy is stale.
    #[test]
    fn export_data_change_limits() {
        let ts = format!(
            "// This file was generated by `cargo test` from src-tauri/src/domain/data_change.rs. \
             Do not edit this file manually.\n\n\
             /**\n * Limits on data changes, as checked by the native core.\n */\n\
             export const DATA_CHANGE_LIMITS = {{\n  \
             maxInsertRows: {MAX_INSERT_ROWS},\n  maxChangeColumns: {MAX_CHANGE_COLUMNS},\n  \
             maxChangeValues: {MAX_CHANGE_VALUES},\n  maxValueBytes: {MAX_VALUE_BYTES},\n  \
             maxChangeBytes: {MAX_CHANGE_BYTES},\n  maxChangedRows: {MAX_CHANGED_ROWS},\n  \
             previewRows: {PREVIEW_ROWS},\n  maxSetChanges: {MAX_SET_CHANGES},\n  \
             previewChanges: {PREVIEW_CHANGES},\n\
             }} as const;\n"
        );
        let dir = std::env::var_os("TS_RS_EXPORT_DIR").expect("set in .cargo/config.toml");
        std::fs::write(std::path::Path::new(&dir).join("DataChangeLimits.ts"), ts).expect("writes");
    }

    #[test]
    fn samples_show_the_first_rows_cut_like_cells() {
        let long = "y".repeat(crate::domain::table_data::CELL_CHAR_LIMIT + 5);
        let values: Vec<Vec<Option<String>>> = (0..30)
            .map(|i| vec![Some(i.to_string()), (i == 1).then(|| long.clone())])
            .collect();
        let sample = ValueSample::of(&["id".into(), "note".into()], &values);
        assert_eq!(sample.rows.len(), PREVIEW_ROWS as usize);
        assert_eq!(sample.total_rows, 30);
        assert_eq!(sample.truncated_cells, vec![CellRef { row: 1, column: 1 }]);
        assert_eq!(
            sample.rows[1][1].as_ref().map(|t| t.chars().count()),
            Some(crate::domain::table_data::CELL_CHAR_LIMIT)
        );
    }
}
