//! Table exports: what the UI asks for, and how rows are written as CSV or JSON.
//!
//! Exports hold complete values (no cell limit); only the number of rows is capped.

use std::io::{self, Write};

use serde::{Deserialize, Serialize};

use super::catalog::ObjectName;
use super::database::OperationId;
use super::table_data::{Filter, PageColumn, PageRequest, RequestError, SortKey, ValueKind};

/// Most rows one export writes. The outcome says when more matched.
pub const EXPORT_MAX_ROWS: u32 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Csv,
    Json,
}

impl ExportFormat {
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }
}

/// Every row of a table or view that matches `filters`, in `sort` order.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportRequest {
    pub object: ObjectName,
    #[serde(default)]
    #[cfg_attr(test, ts(as = "Option<Vec<Filter>>", optional))]
    pub filters: Vec<Filter>,
    #[serde(default)]
    #[cfg_attr(test, ts(as = "Option<Vec<SortKey>>", optional))]
    pub sort: Vec<SortKey>,
    pub format: ExportFormat,
    /// Lets the caller cancel the export while it runs.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub operation_id: Option<OperationId>,
}

impl ExportRequest {
    /// The same rows as a page request, from the start and without a page limit.
    #[must_use]
    pub fn query(&self) -> PageRequest {
        PageRequest {
            object: self.object.clone(),
            filters: self.filters.clone(),
            sort: self.sort.clone(),
            offset: 0,
            limit: None,
            operation_id: self.operation_id,
        }
    }

    /// Checks the request's shape, with the same rules as a page request.
    ///
    /// # Errors
    ///
    /// The first problem found.
    pub fn validate(&self) -> Result<(), RequestError> {
        self.query().validate()
    }

    /// A suggested file name: the object's name with unusual characters replaced.
    #[must_use]
    pub fn file_name(&self) -> String {
        let stem: String = self
            .object
            .name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                    c
                } else {
                    '_'
                }
            })
            .take(100)
            .collect();
        let stem = stem.trim_matches('.');
        let stem = if stem.is_empty() { "export" } else { stem };
        format!("{stem}.{}", self.format.extension())
    }
}

/// What an export wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportSummary {
    pub rows: u64,
    /// More rows matched than the export may write.
    pub truncated: bool,
}

/// The result shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct TableExportOutcome {
    pub file_name: String,
    #[cfg_attr(test, ts(type = "number"))]
    pub rows: u64,
    /// Stopped at [`EXPORT_MAX_ROWS`]; more rows matched.
    pub truncated: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub max_rows: u32,
}

/// Receives an export's columns, then its rows, as the database returns them.
pub trait RowSink {
    /// Called once, before any row.
    ///
    /// # Errors
    ///
    /// When writing fails.
    fn columns(&mut self, columns: &[PageColumn]) -> io::Result<()>;

    /// Values as text, `None` for SQL NULL, in `columns` order.
    ///
    /// # Errors
    ///
    /// When writing fails.
    fn row(&mut self, values: &[Option<String>]) -> io::Result<()>;
}

/// Writes rows as CSV (RFC 4180) or as a JSON array of objects.
///
/// CSV: a header row of column names, CRLF line ends, fields quoted when they hold a comma,
/// quote or line break. SQL NULL is an empty field; empty text is `""`, so the two stay distinct.
///
/// JSON: one object per row, keyed by column name. NULL is `null`; booleans are `true`/`false`;
/// numbers are written exactly as PostgreSQL printed them when that is valid JSON (`NaN` and
/// `Infinity` become strings); `json`, `jsonb` and arrays are embedded as JSON; everything else
/// is a string.
#[derive(Debug)]
pub struct ExportWriter<W: Write> {
    out: W,
    format: ExportFormat,
    columns: Vec<PageColumn>,
    rows: u64,
}

impl<W: Write> ExportWriter<W> {
    pub const fn new(out: W, format: ExportFormat) -> Self {
        Self {
            out,
            format,
            columns: Vec::new(),
            rows: 0,
        }
    }

    /// Ends the document and returns the writer.
    ///
    /// # Errors
    ///
    /// When writing fails.
    pub fn finish(mut self) -> io::Result<W> {
        if self.format == ExportFormat::Json {
            self.out
                .write_all(if self.rows == 0 { b"[]\n" } else { b"\n]\n" })?;
        }
        self.out.flush()?;
        Ok(self.out)
    }

    fn csv_field(&mut self, field: &str) -> io::Result<()> {
        if field.contains([',', '"', '\n', '\r']) || field.is_empty() {
            self.out.write_all(b"\"")?;
            self.out.write_all(field.replace('"', "\"\"").as_bytes())?;
            self.out.write_all(b"\"")
        } else {
            self.out.write_all(field.as_bytes())
        }
    }

    fn json_value(&mut self, kind: ValueKind, value: Option<&str>) -> io::Result<()> {
        let raw = match (kind, value) {
            (_, None) => Some("null"),
            (ValueKind::Boolean, Some(text @ ("true" | "false"))) => Some(text),
            (ValueKind::Number, Some(text)) if is_json_number(text) => Some(text),
            (ValueKind::Json | ValueKind::Array, Some(text)) if is_json(text) => Some(text),
            _ => None,
        };
        match raw {
            Some(raw) => self.out.write_all(raw.as_bytes()),
            None => serde_json::to_writer(&mut self.out, value.unwrap_or_default())
                .map_err(io::Error::other),
        }
    }
}

impl<W: Write> RowSink for ExportWriter<W> {
    fn columns(&mut self, columns: &[PageColumn]) -> io::Result<()> {
        self.columns = columns.to_vec();
        match self.format {
            ExportFormat::Csv => {
                for (index, column) in columns.iter().enumerate() {
                    if index > 0 {
                        self.out.write_all(b",")?;
                    }
                    // A header is never empty, so it is quoted only when it needs to be.
                    self.csv_field(&column.name)?;
                }
                self.out.write_all(b"\r\n")
            }
            ExportFormat::Json => Ok(()),
        }
    }

    fn row(&mut self, values: &[Option<String>]) -> io::Result<()> {
        match self.format {
            ExportFormat::Csv => {
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        self.out.write_all(b",")?;
                    }
                    if let Some(value) = value {
                        self.csv_field(value)?;
                    }
                }
                self.out.write_all(b"\r\n")?;
            }
            ExportFormat::Json => {
                self.out
                    .write_all(if self.rows == 0 { b"[\n  {" } else { b",\n  {" })?;
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        self.out.write_all(b", ")?;
                    }
                    let column = self.columns.get(index);
                    let name = column.map_or("", |c| c.name.as_str());
                    let kind = column.map_or(ValueKind::Other, |c| c.kind);
                    serde_json::to_writer(&mut self.out, name).map_err(io::Error::other)?;
                    self.out.write_all(b": ")?;
                    self.json_value(kind, value.as_deref())?;
                }
                self.out.write_all(b"}")?;
            }
        }
        self.rows += 1;
        Ok(())
    }
}

/// Whether `text` is a JSON number: `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`.
fn is_json_number(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    let digits = |i: &mut usize| {
        let start = *i;
        while bytes.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        *i > start
    };
    if bytes.get(i) == Some(&b'-') {
        i += 1;
    }
    match bytes.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            digits(&mut i);
        }
        _ => return false,
    }
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        if !digits(&mut i) {
            return false;
        }
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if !digits(&mut i) {
            return false;
        }
    }
    i == bytes.len()
}

fn is_json(text: &str) -> bool {
    serde_json::from_str::<serde::de::IgnoredAny>(text).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str, kind: ValueKind) -> PageColumn {
        PageColumn {
            name: name.into(),
            data_type: String::new(),
            kind,
        }
    }

    fn write(format: ExportFormat, columns: &[PageColumn], rows: &[Vec<Option<&str>>]) -> String {
        let mut writer = ExportWriter::new(Vec::new(), format);
        writer.columns(columns).expect("columns");
        for row in rows {
            let row: Vec<Option<String>> = row.iter().map(|v| v.map(Into::into)).collect();
            writer.row(&row).expect("row");
        }
        String::from_utf8(writer.finish().expect("finish")).expect("utf-8")
    }

    #[test]
    fn csv_quotes_only_what_needs_it_and_keeps_null_apart_from_empty() {
        let columns = [
            column("id", ValueKind::Number),
            column("note, \"x\"", ValueKind::Text),
        ];
        let csv = write(
            ExportFormat::Csv,
            &columns,
            &[
                vec![Some("1"), Some("plain")],
                vec![Some("2"), Some("a,b \"c\"\nd")],
                vec![Some("3"), None],
                vec![Some("4"), Some("")],
            ],
        );

        assert_eq!(
            csv,
            "id,\"note, \"\"x\"\"\"\r\n1,plain\r\n2,\"a,b \"\"c\"\"\nd\"\r\n3,\r\n4,\"\"\r\n"
        );
    }

    #[test]
    fn json_keeps_types_and_exact_numbers() {
        let columns = [
            column("n", ValueKind::Number),
            column("ok", ValueKind::Boolean),
            column("doc", ValueKind::Json),
            column("tags", ValueKind::Array),
            column("label", ValueKind::Text),
        ];
        let json = write(
            ExportFormat::Json,
            &columns,
            &[
                vec![
                    Some("12345678901234567890.123456789"),
                    Some("true"),
                    Some(r#"{"a": [1, 2]}"#),
                    Some(r#"["x","y"]"#),
                    Some("say \"hi\""),
                ],
                vec![Some("NaN"), None, Some("not json"), None, Some("")],
            ],
        );

        assert_eq!(
            json,
            "[\n  {\"n\": 12345678901234567890.123456789, \"ok\": true, \"doc\": {\"a\": [1, 2]}, \
             \"tags\": [\"x\",\"y\"], \"label\": \"say \\\"hi\\\"\"},\n  \
             {\"n\": \"NaN\", \"ok\": null, \"doc\": \"not json\", \"tags\": null, \"label\": \"\"}\n]\n"
        );
        serde_json::from_str::<serde_json::Value>(&json).expect("valid JSON");
    }

    #[test]
    fn empty_exports_are_still_valid_documents() {
        let columns = [column("id", ValueKind::Number)];
        assert_eq!(write(ExportFormat::Json, &columns, &[]), "[]\n");
        assert_eq!(write(ExportFormat::Csv, &columns, &[]), "id\r\n");
    }

    #[test]
    fn json_numbers_follow_the_grammar() {
        for valid in ["0", "-0.5", "12.50", "1e+20", "-1E-7", "42"] {
            assert!(is_json_number(valid), "{valid}");
        }
        for invalid in [
            "", "-", "01", "1.", ".5", "1e", "NaN", "Infinity", "$1.00", "1 ",
        ] {
            assert!(!is_json_number(invalid), "{invalid}");
        }
    }

    #[test]
    fn file_names_are_safe_and_carry_the_extension() {
        let request = |name: &str, format| ExportRequest {
            object: ObjectName {
                schema: "public".into(),
                name: name.into(),
            },
            filters: Vec::new(),
            sort: Vec::new(),
            format,
            operation_id: None,
        };
        assert_eq!(
            request("orders", ExportFormat::Csv).file_name(),
            "orders.csv"
        );
        assert_eq!(
            request("../odd/name with space", ExportFormat::Json).file_name(),
            "_odd_name_with_space.json"
        );
        assert_eq!(request("..", ExportFormat::Csv).file_name(), "export.csv");
    }

    /// The UI states the same cap; CI fails if this copy is stale.
    #[test]
    fn export_limits() {
        let ts = format!(
            "// This file was generated by `cargo test` from src-tauri/src/domain/export.rs. \
             Do not edit this file manually.\n\n\
             /**\n * Limits on table exports, as enforced by the native core.\n */\n\
             export const EXPORT_LIMITS = {{\n  maxRows: {EXPORT_MAX_ROWS},\n}} as const;\n"
        );
        let dir = std::env::var_os("TS_RS_EXPORT_DIR").expect("set in .cargo/config.toml");
        std::fs::write(std::path::Path::new(&dir).join("ExportLimits.ts"), ts).expect("writes");
    }
}
