//! Running one read statement from the SQL editor: the request and its outcomes.
//!
//! Database errors and refusals are outcomes, not failures of the app: they come back as data,
//! with the position in the submitted text, so the editor can point at them.

use serde::{Deserialize, Serialize};

use super::database::OperationId;
use super::sql::{MAX_SQL_BYTES, SourcePosition, SourceSpan, StatementKind};
use super::table_data::{CellRef, MAX_OFFSET, PageColumn};
use crate::errors::{AppError, ErrorCode};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryRequest {
    /// Exactly one statement (a trailing `;` and comments are fine).
    pub sql: String,
    /// Rows to skip, for later pages of the same result.
    #[serde(default)]
    #[cfg_attr(test, ts(type = "number", optional))]
    pub offset: u64,
    /// Lets the caller cancel the statement while it runs.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub operation_id: Option<OperationId>,
}

impl QueryRequest {
    /// # Errors
    ///
    /// `invalid_input` for text over [`MAX_SQL_BYTES`] or an offset out of range.
    pub fn validate(&self) -> Result<(), AppError> {
        if self.sql.len() > MAX_SQL_BYTES {
            return Err(AppError::invalid_input(
                "That SQL is too long to run (1 MB at most).",
            ));
        }
        if self.offset > MAX_OFFSET {
            return Err(AppError::invalid_input(
                "That page is too far into the result.",
            ));
        }
        Ok(())
    }
}

/// What running a statement led to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum QueryRun {
    Completed(QueryResult),
    /// aidb did not send the statement: not a read, several statements, or unreadable.
    Refused(QueryRefusal),
    /// The database rejected or stopped the statement.
    Failed(QueryFailure),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    /// The statement that ran, within the submitted text.
    pub statement: SourceSpan,
    pub columns: Vec<PageColumn>,
    /// Values as text, `null` for SQL NULL.
    pub rows: Vec<Vec<Option<String>>>,
    pub truncated_cells: Vec<CellRef>,
    #[cfg_attr(test, ts(type = "number"))]
    pub offset: u64,
    pub has_more: bool,
    pub byte_limit_reached: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub cell_char_limit: usize,
    #[cfg_attr(test, ts(type = "number"))]
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct QueryRefusal {
    pub message: String,
    /// The statement's kind, when there was exactly one statement.
    pub kind: Option<StatementKind>,
    pub position: Option<SourcePosition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct QueryFailure {
    /// The same categories as app errors: `invalid_input`, `timeout`, `cancelled`, ...
    pub code: ErrorCode,
    /// The database's own message.
    pub message: String,
    pub sqlstate: Option<String>,
    pub position: Option<SourcePosition>,
    /// The statement that ran, within the submitted text.
    pub statement: SourceSpan,
}

/// Rows of a read, as an adapter returns them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryRows {
    pub columns: Vec<PageColumn>,
    pub rows: Vec<Vec<Option<String>>>,
    pub truncated_cells: Vec<CellRef>,
    pub has_more: bool,
    pub byte_limit_reached: bool,
}
