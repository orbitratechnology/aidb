//! Pages of rows from one table or view: what the UI asks for and what it gets back.
//!
//! A request names columns as plain strings. They are never put into SQL: the adapter matches
//! them against the live column list and uses the database's own quoting of that list. Filter
//! values are always bound as parameters.

use serde::{Deserialize, Serialize};

use super::catalog::ObjectName;
use super::database::OperationId;
use crate::errors::AppError;

/// Most filters in one request.
pub const MAX_FILTERS: usize = 20;
/// Most sort keys in one request.
pub const MAX_SORT_KEYS: usize = 8;
/// Longest filter value, in characters.
pub const MAX_FILTER_VALUE_CHARS: usize = 1_000;
/// Cells longer than this many characters are cut; the page says which ones.
pub const CELL_CHAR_LIMIT: usize = 2_000;
/// Highest row offset, within the range JavaScript numbers hold exactly.
pub const MAX_OFFSET: u64 = 1_000_000_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum FilterOperator {
    Equals,
    NotEquals,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    /// Case-insensitive match anywhere in the value's text.
    Contains,
    /// Case-insensitive match at the start of the value's text.
    StartsWith,
    IsNull,
    IsNotNull,
}

impl FilterOperator {
    /// Whether the operator compares against a value.
    #[must_use]
    pub const fn takes_value(self) -> bool {
        !matches!(self, Self::IsNull | Self::IsNotNull)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Filter {
    pub column: String,
    pub operator: FilterOperator,
    /// Text the database converts to the column's type; absent for the null checks.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SortKey {
    pub column: String,
    pub direction: SortDirection,
}

/// One page of a table or view, filtered and sorted.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PageRequest {
    pub object: ObjectName,
    /// All must match.
    #[serde(default)]
    #[cfg_attr(test, ts(as = "Option<Vec<Filter>>", optional))]
    pub filters: Vec<Filter>,
    #[serde(default)]
    #[cfg_attr(test, ts(as = "Option<Vec<SortKey>>", optional))]
    pub sort: Vec<SortKey>,
    /// Rows to skip.
    #[serde(default)]
    #[cfg_attr(test, ts(type = "number", optional))]
    pub offset: u64,
    /// Rows wanted; capped by the Safety setting, which is also the default.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub limit: Option<u32>,
    /// Lets the caller cancel the read while it runs.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub operation_id: Option<OperationId>,
}

/// Why a request was refused before reaching the database.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RequestError {
    #[error("Use at most {MAX_FILTERS} filters.")]
    TooManyFilters,
    #[error("Sort by at most {MAX_SORT_KEYS} columns.")]
    TooManySortKeys,
    #[error("The column {0} is sorted more than once.")]
    DuplicateSort(String),
    #[error("The filter on {0} needs a value.")]
    MissingValue(String),
    #[error("The filter on {0} does not take a value.")]
    UnexpectedValue(String),
    #[error("Filter values can be at most {MAX_FILTER_VALUE_CHARS} characters.")]
    ValueTooLong,
    #[error("Filter values cannot contain NUL characters.")]
    NulInValue,
    #[error("Pages must hold at least one row.")]
    ZeroLimit,
    #[error("That page is too far into the table.")]
    OffsetTooLarge,
}

impl From<RequestError> for AppError {
    fn from(error: RequestError) -> Self {
        Self::invalid_input(error.to_string())
    }
}

impl PageRequest {
    /// Checks the request's shape. Column names are checked later, against the live table.
    ///
    /// # Errors
    ///
    /// The first problem found.
    pub fn validate(&self) -> Result<(), RequestError> {
        if self.filters.len() > MAX_FILTERS {
            return Err(RequestError::TooManyFilters);
        }
        if self.sort.len() > MAX_SORT_KEYS {
            return Err(RequestError::TooManySortKeys);
        }
        for (index, key) in self.sort.iter().enumerate() {
            if self.sort[..index].iter().any(|k| k.column == key.column) {
                return Err(RequestError::DuplicateSort(key.column.clone()));
            }
        }
        for filter in &self.filters {
            match (&filter.value, filter.operator.takes_value()) {
                (None, true) => return Err(RequestError::MissingValue(filter.column.clone())),
                (Some(_), false) => {
                    return Err(RequestError::UnexpectedValue(filter.column.clone()));
                }
                (Some(value), true) => {
                    if value.chars().count() > MAX_FILTER_VALUE_CHARS {
                        return Err(RequestError::ValueTooLong);
                    }
                    if value.contains('\0') {
                        return Err(RequestError::NulInValue);
                    }
                }
                (None, false) => {}
            }
        }
        if self.limit == Some(0) {
            return Err(RequestError::ZeroLimit);
        }
        if self.offset > MAX_OFFSET {
            return Err(RequestError::OffsetTooLarge);
        }
        Ok(())
    }
}

/// How the UI should present a column's values. Every value arrives as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ValueKind {
    Boolean,
    /// Exact decimal text; never converted to a floating-point number.
    Number,
    Text,
    /// ISO 8601 dates and times, with the offset for `timestamptz`.
    DateTime,
    /// JSON text: `json`/`jsonb` columns.
    Json,
    /// JSON text of the array.
    Array,
    Uuid,
    /// PostgreSQL hex text (`\x…`).
    Binary,
    Other,
}

impl ValueKind {
    /// Classifies a type by its base type's `typcategory` and name.
    #[must_use]
    pub fn classify(category: &str, type_name: &str) -> Self {
        match type_name {
            "json" | "jsonb" => return Self::Json,
            "uuid" => return Self::Uuid,
            "bytea" => return Self::Binary,
            _ => {}
        }
        match category {
            "A" => Self::Array,
            "B" => Self::Boolean,
            "N" => Self::Number,
            "D" => Self::DateTime,
            "S" | "E" => Self::Text,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct PageColumn {
    pub name: String,
    /// The type as PostgreSQL prints it.
    pub data_type: String,
    pub kind: ValueKind,
}

/// A cell, by row and column index within the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct CellRef {
    pub row: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct TablePage {
    pub object: ObjectName,
    pub columns: Vec<PageColumn>,
    /// Columns the role may not read; they are left out of `columns` and `rows`.
    pub omitted_columns: Vec<String>,
    /// Values as text, `null` for SQL NULL, in `columns` order.
    pub rows: Vec<Vec<Option<String>>>,
    /// Cells cut at [`CELL_CHAR_LIMIT`] characters.
    pub truncated_cells: Vec<CellRef>,
    #[cfg_attr(test, ts(type = "number"))]
    pub offset: u64,
    /// More rows follow this page.
    pub has_more: bool,
    /// The page stopped early because it reached the result size limit.
    pub byte_limit_reached: bool,
    /// The order applied, including primary key columns added to keep pages stable.
    pub order: Vec<SortKey>,
    /// Whether the order is total, so pages neither repeat nor skip rows. False when nothing
    /// is sorted and there is no primary key to fall back on.
    pub stable_order: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub cell_char_limit: usize,
}

/// The size limits for one page, from the Safety settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageLimits {
    pub max_rows: u32,
    pub max_bytes: usize,
}

/// Escapes `%`, `_` and `\` so a `LIKE` pattern matches them literally.
#[must_use]
pub fn escape_like(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

/// Cuts `text` to [`CELL_CHAR_LIMIT`] characters; returns whether it was longer.
pub fn truncate_cell(text: &mut String) -> bool {
    match text.char_indices().nth(CELL_CHAR_LIMIT) {
        Some((end, _)) => {
            text.truncate(end);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> PageRequest {
        serde_json::from_str(r#"{"object": {"schema": "public", "name": "orders"}}"#)
            .expect("parses")
    }

    fn filter(operator: FilterOperator, value: Option<&str>) -> Filter {
        Filter {
            column: "id".into(),
            operator,
            value: value.map(Into::into),
        }
    }

    #[test]
    fn minimal_requests_are_valid_and_reject_unknown_fields() {
        let minimal = request();
        assert_eq!((minimal.offset, minimal.limit), (0, None));
        assert_eq!(minimal.validate(), Ok(()));

        let sneaky = serde_json::from_str::<PageRequest>(
            r#"{"object": {"schema": "s", "name": "t"}, "sql": "DROP TABLE t"}"#,
        );
        assert!(sneaky.is_err());
    }

    #[test]
    fn filters_need_values_exactly_when_they_compare() {
        let mut request = request();
        request.filters = vec![filter(FilterOperator::Equals, None)];
        assert_eq!(
            request.validate(),
            Err(RequestError::MissingValue("id".into()))
        );

        request.filters = vec![filter(FilterOperator::IsNull, Some("x"))];
        assert_eq!(
            request.validate(),
            Err(RequestError::UnexpectedValue("id".into()))
        );

        request.filters = vec![
            filter(FilterOperator::IsNotNull, None),
            filter(FilterOperator::Contains, Some("50%")),
        ];
        assert_eq!(request.validate(), Ok(()));
    }

    #[test]
    fn sizes_and_values_are_bounded() {
        let mut request = request();
        request.filters = vec![filter(FilterOperator::IsNull, None); MAX_FILTERS + 1];
        assert_eq!(request.validate(), Err(RequestError::TooManyFilters));

        request.filters = vec![filter(
            FilterOperator::Equals,
            Some(&"x".repeat(MAX_FILTER_VALUE_CHARS + 1)),
        )];
        assert_eq!(request.validate(), Err(RequestError::ValueTooLong));

        request.filters = vec![filter(FilterOperator::Equals, Some("a\0b"))];
        assert_eq!(request.validate(), Err(RequestError::NulInValue));

        request.filters.clear();
        request.limit = Some(0);
        assert_eq!(request.validate(), Err(RequestError::ZeroLimit));

        request.limit = None;
        request.offset = MAX_OFFSET + 1;
        assert_eq!(request.validate(), Err(RequestError::OffsetTooLarge));
    }

    #[test]
    fn sort_keys_are_bounded_and_unique() {
        let key = |column: &str| SortKey {
            column: column.into(),
            direction: SortDirection::Asc,
        };
        let mut request = request();
        request.sort = vec![key("a"), key("b"), key("a")];
        assert_eq!(
            request.validate(),
            Err(RequestError::DuplicateSort("a".into()))
        );

        request.sort = (0..=MAX_SORT_KEYS).map(|i| key(&i.to_string())).collect();
        assert_eq!(request.validate(), Err(RequestError::TooManySortKeys));
    }

    #[test]
    fn types_are_classified_by_category_and_name() {
        let cases = [
            ("U", "jsonb", ValueKind::Json),
            ("U", "json", ValueKind::Json),
            ("U", "uuid", ValueKind::Uuid),
            ("U", "bytea", ValueKind::Binary),
            ("A", "_int4", ValueKind::Array),
            ("B", "bool", ValueKind::Boolean),
            ("N", "numeric", ValueKind::Number),
            ("D", "timestamptz", ValueKind::DateTime),
            ("S", "text", ValueKind::Text),
            ("E", "mood", ValueKind::Text),
            ("T", "interval", ValueKind::Other),
            ("G", "point", ValueKind::Other),
        ];
        for (category, name, kind) in cases {
            assert_eq!(ValueKind::classify(category, name), kind, "{name}");
        }
    }

    #[test]
    fn like_patterns_match_wildcards_literally() {
        assert_eq!(escape_like(r"50%_off\now"), r"50\%\_off\\now");
        assert_eq!(escape_like("plain"), "plain");
    }

    /// The data grid builds requests within the same limits; CI fails if this copy is stale.
    #[test]
    fn export_table_data_limits() {
        let ts = format!(
            "// This file was generated by `cargo test` from src-tauri/src/domain/table_data.rs. \
             Do not edit this file manually.\n\n\
             /**\n * Limits on table page requests, as checked by the native core.\n */\n\
             export const TABLE_DATA_LIMITS = {{\n  \
             maxFilters: {MAX_FILTERS},\n  maxSortKeys: {MAX_SORT_KEYS},\n  \
             maxFilterValueChars: {MAX_FILTER_VALUE_CHARS},\n  \
             cellCharLimit: {CELL_CHAR_LIMIT},\n  maxOffset: {MAX_OFFSET},\n\
             }} as const;\n"
        );
        let dir = std::env::var_os("TS_RS_EXPORT_DIR").expect("set in .cargo/config.toml");
        std::fs::write(std::path::Path::new(&dir).join("TableDataLimits.ts"), ts).expect("writes");
    }

    #[test]
    fn cells_are_cut_on_character_boundaries() {
        let mut short = "é".repeat(CELL_CHAR_LIMIT);
        assert!(!truncate_cell(&mut short));
        assert_eq!(short.chars().count(), CELL_CHAR_LIMIT);

        let mut long = "é".repeat(CELL_CHAR_LIMIT + 1);
        assert!(truncate_cell(&mut long));
        assert_eq!(long.chars().count(), CELL_CHAR_LIMIT);
    }
}
