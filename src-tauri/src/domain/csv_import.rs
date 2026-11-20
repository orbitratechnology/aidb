//! CSV files read for import: the encoding, delimiter and header are detected (and can be
//! overridden), the records parsed, and the file's columns matched to a table's.
//!
//! Nothing here touches a database. The parsed rows stay in the core; the webview sees the
//! columns, a short sample and counts.

use encoding_rs::{Encoding, UTF_16BE, UTF_16LE};
use serde::{Deserialize, Serialize};

use crate::domain::table_data::truncate_cell;

/// Largest file read for import.
pub const MAX_IMPORT_BYTES: u64 = 64 * 1024 * 1024;
/// Most data rows one import may hold.
pub const MAX_IMPORT_ROWS: usize = 100_000;
/// Most columns a file may have.
pub const MAX_IMPORT_COLUMNS: usize = 200;
/// Rows of the file shown while mapping.
pub const SAMPLE_ROWS: usize = 10;
/// Records read to detect the delimiter and the header.
const DETECTION_RECORDS: usize = 100;

/// Encodings offered for import, by their WHATWG label.
pub const ENCODINGS: [&str; 9] = [
    "utf-8",
    "utf-16le",
    "utf-16be",
    "windows-1252",
    "iso-8859-2",
    "iso-8859-15",
    "shift_jis",
    "euc-kr",
    "gbk",
];

/// Delimiters offered and detected.
pub const DELIMITERS: [char; 4] = [',', ';', '\t', '|'];

/// How a file is read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CsvOptions {
    /// One of [`ENCODINGS`].
    pub encoding: String,
    /// One of [`DELIMITERS`], as a one-character string.
    pub delimiter: String,
    /// The first record names the columns.
    pub has_header: bool,
}

/// Why a file cannot be read for import.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CsvError {
    #[error("Choose one of the offered encodings.")]
    UnknownEncoding,
    #[error("Choose one of the offered delimiters.")]
    UnknownDelimiter,
    #[error("The file is empty.")]
    Empty,
    #[error("The file has no data rows.")]
    NoRows,
    #[error("The file has more than {MAX_IMPORT_ROWS} rows. Split it and import the parts.")]
    TooManyRows,
    #[error("The file has more than {MAX_IMPORT_COLUMNS} columns.")]
    TooManyColumns,
    #[error("The file could not be read as CSV: {0}")]
    Malformed(String),
}

impl From<CsvError> for crate::errors::AppError {
    fn from(error: CsvError) -> Self {
        Self::invalid_input(error.to_string())
    }
}

/// A file read with some options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCsv {
    pub options: CsvOptions,
    /// The file's column names: the header's, or "Column 1", "Column 2", …
    pub columns: Vec<String>,
    /// Data rows, without the header.
    pub rows: Vec<Vec<String>>,
    /// Things worth knowing that do not stop the import.
    pub warnings: Vec<String>,
}

fn encoding(label: &str) -> Result<&'static Encoding, CsvError> {
    if !ENCODINGS.contains(&label) {
        return Err(CsvError::UnknownEncoding);
    }
    Encoding::for_label(label.as_bytes()).ok_or(CsvError::UnknownEncoding)
}

fn delimiter(text: &str) -> Result<u8, CsvError> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if DELIMITERS.contains(&c) => Ok(c as u8),
        _ => Err(CsvError::UnknownDelimiter),
    }
}

/// The encoding a file is most likely in: its byte-order mark, else UTF-8 when the bytes are
/// valid UTF-8, else Windows-1252 (the usual encoding of spreadsheet exports on Windows).
#[must_use]
pub fn detect_encoding(bytes: &[u8]) -> &'static str {
    match Encoding::for_bom(bytes) {
        Some((found, _)) if found == UTF_16LE => "utf-16le",
        Some((found, _)) if found == UTF_16BE => "utf-16be",
        Some(_) => "utf-8",
        None if std::str::from_utf8(bytes).is_ok() => "utf-8",
        None => "windows-1252",
    }
}

/// The text of the file in `label`, without a byte-order mark, and whether any bytes could not
/// be decoded (they become U+FFFD).
fn decode(bytes: &[u8], label: &str) -> Result<(String, bool), CsvError> {
    let encoding = encoding(label)?;
    let (text, _, had_errors) = encoding.decode(bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned();
    Ok((text, had_errors))
}

fn reader(text: &str, delimiter: u8) -> csv::Reader<&[u8]> {
    csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes())
}

/// The delimiter that splits the first records into the same number of fields (more than one)
/// most often; a comma when none does.
#[must_use]
pub fn detect_delimiter(text: &str) -> char {
    let mut best = (',', 0_usize);
    for candidate in DELIMITERS {
        let mut counts: Vec<usize> = Vec::new();
        for record in reader(text, candidate as u8)
            .records()
            .take(DETECTION_RECORDS)
            .map_while(Result::ok)
        {
            counts.push(record.len());
        }
        let Some(&first) = counts.first() else {
            continue;
        };
        if first < 2 {
            continue;
        }
        let consistent = counts.iter().filter(|&&count| count == first).count();
        if consistent > best.1 {
            best = (candidate, consistent);
        }
    }
    best.0
}

fn looks_numeric(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && value.parse::<f64>().is_ok()
}

/// Folds a name for matching: lowercase, without spaces, underscores and dashes.
fn fold(name: &str) -> String {
    name.chars()
        .filter(|c| !matches!(c, ' ' | '_' | '-'))
        .flat_map(char::to_lowercase)
        .collect()
}

/// Whether the first record names the columns: its values are all filled in and different,
/// and either one of them names a table column or a column holds text there and numbers below.
#[must_use]
pub fn detect_header(records: &[Vec<String>], table_columns: &[String]) -> bool {
    let Some(first) = records.first() else {
        return false;
    };
    let names: Vec<String> = first.iter().map(|value| fold(value)).collect();
    let mut unique = names.clone();
    unique.sort();
    unique.dedup();
    if names.iter().any(String::is_empty) || unique.len() != names.len() {
        return false;
    }
    let table: Vec<String> = table_columns.iter().map(|name| fold(name)).collect();
    if names.iter().any(|name| table.contains(name)) {
        return true;
    }
    let rest = &records[1..records.len().min(DETECTION_RECORDS)];
    if rest.is_empty() {
        return false;
    }
    (0..first.len()).any(|column| {
        let numbers = rest
            .iter()
            .filter(|row| row.get(column).is_some_and(|value| looks_numeric(value)))
            .count();
        !looks_numeric(&first[column]) && numbers * 5 >= rest.len() * 4
    })
}

/// Reads the file with `options`.
///
/// # Errors
///
/// [`CsvError`] for unknown options, an empty or malformed file, or one over the limits.
pub fn parse(bytes: &[u8], options: CsvOptions) -> Result<ParsedCsv, CsvError> {
    let (text, had_errors) = decode(bytes, &options.encoding)?;
    let delimiter = delimiter(&options.delimiter)?;
    if text.trim().is_empty() {
        return Err(CsvError::Empty);
    }
    let mut records: Vec<Vec<String>> = Vec::new();
    for record in reader(&text, delimiter).records() {
        let record = record.map_err(|error| CsvError::Malformed(error.to_string()))?;
        // A blank line is not a row.
        if record.len() == 1 && record.get(0).is_some_and(str::is_empty) {
            continue;
        }
        if records.len() > MAX_IMPORT_ROWS {
            return Err(CsvError::TooManyRows);
        }
        records.push(record.iter().map(str::to_owned).collect());
    }
    let mut warnings = Vec::new();
    if had_errors {
        warnings.push(format!(
            "Some bytes are not valid {}; they show as �. Choose the file's encoding.",
            options.encoding
        ));
    }
    let width = records.iter().map(Vec::len).max().unwrap_or(0);
    if width > MAX_IMPORT_COLUMNS {
        return Err(CsvError::TooManyColumns);
    }
    let columns: Vec<String> = if options.has_header {
        let header = records.remove(0);
        (0..width)
            .map(|index| {
                header
                    .get(index)
                    .map(|name| name.trim().to_owned())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| format!("Column {}", index + 1))
            })
            .collect()
    } else {
        (1..=width).map(|index| format!("Column {index}")).collect()
    };
    if records.is_empty() {
        return Err(CsvError::NoRows);
    }
    if records.len() > MAX_IMPORT_ROWS {
        return Err(CsvError::TooManyRows);
    }
    let ragged = records.iter().filter(|row| row.len() != width).count();
    if ragged > 0 {
        let first = records
            .iter()
            .position(|row| row.len() != width)
            .map_or(0, |index| index + 1);
        warnings.push(format!(
            "{ragged} {} fewer fields than the widest (row {first} is the first); missing \
             fields are treated as empty.",
            if ragged == 1 { "row has" } else { "rows have" },
        ));
        for row in &mut records {
            row.resize(width, String::new());
        }
    }
    Ok(ParsedCsv {
        options,
        columns,
        rows: records,
        warnings,
    })
}

/// The options a file is most likely read with, for a table with `table_columns`.
///
/// # Errors
///
/// When the file is empty.
pub fn detect(bytes: &[u8], table_columns: &[String]) -> Result<CsvOptions, CsvError> {
    let encoding = detect_encoding(bytes);
    let (text, _) = decode(bytes, encoding)?;
    if text.trim().is_empty() {
        return Err(CsvError::Empty);
    }
    let delimiter = detect_delimiter(&text);
    let records: Vec<Vec<String>> = reader(&text, delimiter as u8)
        .records()
        .take(DETECTION_RECORDS)
        .map_while(Result::ok)
        .map(|record| record.iter().map(str::to_owned).collect())
        .collect();
    Ok(CsvOptions {
        encoding: encoding.to_owned(),
        delimiter: delimiter.to_string(),
        has_header: detect_header(&records, table_columns),
    })
}

/// For each file column, the table column it most likely fills: the one with the same name
/// (ignoring case, spaces, underscores and dashes), or, when the file has no header and as many
/// columns as `table_columns`, the one in the same position.
#[must_use]
pub fn suggest_mapping(parsed: &ParsedCsv, table_columns: &[String]) -> Vec<Option<String>> {
    if !parsed.options.has_header {
        return if parsed.columns.len() == table_columns.len() {
            table_columns.iter().cloned().map(Some).collect()
        } else {
            vec![None; parsed.columns.len()]
        };
    }
    let mut taken: Vec<&String> = Vec::new();
    parsed
        .columns
        .iter()
        .map(|name| {
            let found = table_columns
                .iter()
                .find(|column| fold(column) == fold(name) && !taken.contains(column));
            if let Some(column) = found {
                taken.push(column);
            }
            found.cloned()
        })
        .collect()
}

/// The first rows, cut like table cells, for review.
#[must_use]
pub fn sample(parsed: &ParsedCsv) -> Vec<Vec<String>> {
    parsed
        .rows
        .iter()
        .take(SAMPLE_ROWS)
        .map(|row| {
            row.iter()
                .map(|value| {
                    let mut value = value.clone();
                    truncate_cell(&mut value);
                    value
                })
                .collect()
        })
        .collect()
}

/// A file read for import, as the webview sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct CsvImportDraft {
    #[cfg_attr(test, ts(type = "string"))]
    pub import_id: uuid::Uuid,
    pub file_name: String,
    pub options: CsvOptions,
    /// The file's column names.
    pub columns: Vec<String>,
    /// For each file column, the table column suggested for it.
    pub suggested: Vec<Option<String>>,
    /// The first rows, cut at [`CELL_CHAR_LIMIT`](crate::domain::table_data::CELL_CHAR_LIMIT)
    /// characters.
    pub sample: Vec<Vec<String>>,
    #[cfg_attr(test, ts(type = "number"))]
    pub total_rows: usize,
    pub warnings: Vec<String>,
}

/// Which table column a file column fills.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColumnMapping {
    /// The file column, by position from 0.
    #[cfg_attr(test, ts(type = "number"))]
    pub file_column: usize,
    pub column: String,
}

/// How to import a file into its table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CsvImportRequest {
    /// Table columns not mapped get their defaults.
    pub mapping: Vec<ColumnMapping>,
    /// Insert only (the default), or with a key (the primary key or a unique constraint),
    /// update the rows whose key already exists.
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub upsert_key: Option<Vec<String>>,
}

/// What a file import would do, or why it cannot be done yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct CsvImportPreview {
    /// What to fix first; empty when the import is ready.
    pub problems: Vec<String>,
    pub ready: Option<CsvImportReady>,
}

/// An import planned against the live table and held for approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct CsvImportReady {
    #[cfg_attr(test, ts(type = "string"))]
    pub approval_id: crate::domain::policy::approval::ApprovalId,
    #[cfg_attr(test, ts(type = "number"))]
    pub rows: u64,
    /// Statements it runs, in one transaction.
    #[cfg_attr(test, ts(type = "number"))]
    pub batches: usize,
    /// The first rows as they will be stored.
    pub after: crate::domain::data_change::ValueSample,
    /// The first statement, cut.
    pub statement: String,
    pub risk: crate::domain::policy::risk::RiskLevel,
    pub approval: crate::domain::policy::risk::ApprovalLevel,
    pub reasons: Vec<String>,
    pub confirmation: Option<String>,
    pub expires_at: String,
}

/// Values per batch: well within the protocol's parameter limit and the change limits.
const BATCH_VALUES: usize = 10_000;
/// Bytes per batch.
const BATCH_BYTES: usize = 4 * 1024 * 1024;
/// Row numbers listed in a problem.
const LISTED_ROWS: usize = 5;

/// "rows 3, 9 and 12", or "rows 3, 9, 12, 20, 21 and 40 more".
fn row_list(rows: &[usize]) -> String {
    let shown: Vec<String> = rows
        .iter()
        .take(LISTED_ROWS)
        .map(ToString::to_string)
        .collect();
    let word = if rows.len() == 1 { "row" } else { "rows" };
    match rows.len() {
        0 => String::new(),
        1 => format!("{word} {}", shown[0]),
        n if n <= LISTED_ROWS => {
            format!("{word} {} and {}", shown[..n - 1].join(", "), shown[n - 1])
        }
        n => format!("{word} {} and {} more", shown.join(", "), n - LISTED_ROWS),
    }
}

/// Why the file cannot be imported into the table with this mapping: unknown or doubly mapped
/// columns, columns the database fills itself, required columns left out or empty, and an
/// update key that is not a key of the table or is left out or empty.
#[must_use]
pub fn import_problems(
    parsed: &ParsedCsv,
    details: &crate::domain::catalog::ObjectDetails,
    request: &CsvImportRequest,
) -> Vec<String> {
    let mut problems = Vec::new();
    if request.mapping.is_empty() {
        problems.push("Choose at least one file column to import.".to_owned());
    }
    let mut seen: Vec<&str> = Vec::new();
    for mapping in &request.mapping {
        let Some(column) = details.columns.iter().find(|c| c.name == mapping.column) else {
            problems.push(format!("The table has no column {}.", mapping.column));
            continue;
        };
        if mapping.file_column >= parsed.columns.len() {
            problems.push(format!(
                "The file has no column {}.",
                mapping.file_column + 1
            ));
        }
        if seen.contains(&column.name.as_str()) {
            problems.push(format!(
                "{} is filled from more than one file column.",
                column.name
            ));
        }
        seen.push(&column.name);
        if column.generated.is_some() {
            problems.push(format!("{} is generated by the database.", column.name));
        } else if column.identity == Some(crate::domain::catalog::Identity::Always) {
            problems.push(format!(
                "{} is filled in by the database (identity).",
                column.name
            ));
        }
    }
    if !problems.is_empty() {
        return problems;
    }
    let file_column = |name: &str| {
        request
            .mapping
            .iter()
            .find(|mapping| mapping.column == name)
            .map(|mapping| mapping.file_column)
    };
    let empty_rows = |index: usize| -> Vec<usize> {
        parsed
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.get(index).is_none_or(|value| value.is_empty()))
            .map(|(number, _)| number + 1)
            .collect()
    };
    // An empty value is stored as NULL, never as the default, so any NOT NULL column filled
    // from the file needs a value in every row; one left out needs a default.
    for column in details.columns.iter().filter(|column| !column.nullable) {
        let required =
            column.default.is_none() && column.identity.is_none() && column.generated.is_none();
        match file_column(&column.name) {
            None if !required => {}
            None => problems.push(format!(
                "{} needs a value (it cannot be empty and has no default), but no file column \
                 fills it.",
                column.name
            )),
            Some(index) => {
                let empty = empty_rows(index);
                if !empty.is_empty() {
                    problems.push(format!(
                        "{} cannot be empty, but it is in {}.",
                        column.name,
                        row_list(&empty)
                    ));
                }
            }
        }
    }
    if let Some(key) = &request.upsert_key {
        let mut wanted = key.clone();
        wanted.sort();
        let keys = details
            .primary_key
            .iter()
            .chain(&details.unique_constraints)
            .map(|constraint| {
                let mut columns = constraint.columns.clone();
                columns.sort();
                columns
            });
        if !keys.into_iter().any(|columns| columns == wanted) {
            problems.push(format!(
                "{} is not the primary key or a unique constraint of the table.",
                key.join(", ")
            ));
        }
        for column in key {
            match file_column(column) {
                None => problems.push(format!(
                    "{column} is part of the key to update by, so a file column must fill it."
                )),
                Some(index) => {
                    let empty = empty_rows(index);
                    if !empty.is_empty() {
                        problems.push(format!(
                            "{column} is part of the key to update by, but it is empty in {}.",
                            row_list(&empty)
                        ));
                    }
                }
            }
        }
        if request
            .mapping
            .iter()
            .all(|mapping| key.contains(&mapping.column))
        {
            problems
                .push("Updating by key needs at least one file column besides the key.".to_owned());
        }
    }
    problems
}

/// The file's rows as batches of inserts into `table`, in file order. Empty fields are NULL.
/// Returns the changes and the first file row (from 0) of each.
#[must_use]
pub fn import_changes(
    parsed: &ParsedCsv,
    table: &crate::domain::catalog::ObjectName,
    request: &CsvImportRequest,
) -> (Vec<crate::domain::data_change::DataChange>, Vec<usize>) {
    let columns: Vec<String> = request.mapping.iter().map(|m| m.column.clone()).collect();
    let max_rows =
        (BATCH_VALUES / columns.len().max(1)).clamp(1, crate::domain::data_change::MAX_INSERT_ROWS);
    let mut changes = Vec::new();
    let mut starts = Vec::new();
    let mut batch: Vec<Vec<Option<String>>> = Vec::new();
    let mut bytes = 0;
    let mut start = 0;
    let flush = |batch: &mut Vec<Vec<Option<String>>>,
                 changes: &mut Vec<crate::domain::data_change::DataChange>| {
        changes.push(crate::domain::data_change::DataChange::Insert {
            table: table.clone(),
            columns: columns.clone(),
            rows: std::mem::take(batch),
            upsert_key: request.upsert_key.clone(),
        });
    };
    for (index, row) in parsed.rows.iter().enumerate() {
        let values: Vec<Option<String>> = request
            .mapping
            .iter()
            .map(|mapping| {
                row.get(mapping.file_column)
                    .filter(|value| !value.is_empty())
                    .cloned()
            })
            .collect();
        let size: usize = values.iter().flatten().map(String::len).sum();
        if !batch.is_empty() && (batch.len() == max_rows || bytes + size > BATCH_BYTES) {
            flush(&mut batch, &mut changes);
            starts.push(start);
            start = index;
            bytes = 0;
        }
        bytes += size;
        batch.push(values);
    }
    if !batch.is_empty() {
        flush(&mut batch, &mut changes);
        starts.push(start);
    }
    (changes, starts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::table_data::CELL_CHAR_LIMIT;

    fn options(delimiter: &str, has_header: bool) -> CsvOptions {
        CsvOptions {
            encoding: "utf-8".into(),
            delimiter: delimiter.into(),
            has_header,
        }
    }

    fn columns(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn encodings_come_from_the_bom_or_the_bytes() {
        assert_eq!(detect_encoding(b"\xEF\xBB\xBFid,name"), "utf-8");
        assert_eq!(detect_encoding(b"\xFF\xFEi\0d\0"), "utf-16le");
        assert_eq!(detect_encoding(b"\xFE\xFF\0i\0d"), "utf-16be");
        assert_eq!(detect_encoding("id,café".as_bytes()), "utf-8");
        assert_eq!(detect_encoding(b"id,caf\xE9"), "windows-1252");

        let parsed = parse(
            b"id,name\n1,caf\xE9\n",
            CsvOptions {
                encoding: "windows-1252".into(),
                ..options(",", true)
            },
        )
        .expect("parses");
        assert_eq!(parsed.rows, vec![vec!["1".to_owned(), "café".to_owned()]]);
        // UTF-16 with its BOM, which is not part of the first column's name.
        let utf16: Vec<u8> = "\u{feff}id;name\n1;Ada\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let parsed = parse(
            &utf16,
            CsvOptions {
                encoding: "utf-16le".into(),
                ..options(";", true)
            },
        )
        .expect("parses");
        assert_eq!(parsed.columns, columns(&["id", "name"]));
    }

    #[test]
    fn the_wrong_encoding_is_reported_not_hidden() {
        let parsed = parse(b"id,name\n1,caf\xE9\n", options(",", true)).expect("parses");
        assert_eq!(parsed.rows[0][1], "caf\u{fffd}");
        assert!(parsed.warnings[0].contains("not valid utf-8"));
    }

    #[test]
    fn delimiters_are_the_ones_that_split_consistently() {
        assert_eq!(detect_delimiter("a,b,c\n1,2,3\n"), ',');
        assert_eq!(detect_delimiter("a;b;c\n1,5;2;3\n4;5,5;6\n"), ';');
        assert_eq!(detect_delimiter("a\tb\n1\t2\n"), '\t');
        assert_eq!(detect_delimiter("a|b\n1|2\n"), '|');
        assert_eq!(detect_delimiter("single column\nvalue\n"), ',');
        // Quoted delimiters do not count.
        assert_eq!(detect_delimiter("name;note\n\"a,b\";c\n\"d,e\";f\n"), ';');
    }

    #[test]
    fn headers_name_table_columns_or_head_numbers() {
        let records = |rows: &[&[&str]]| -> Vec<Vec<String>> {
            rows.iter()
                .map(|row| row.iter().map(|v| (*v).to_owned()).collect())
                .collect()
        };
        let table = columns(&["id", "unit_price"]);
        assert!(detect_header(
            &records(&[&["ID", "Unit Price"], &["1", "2"]]),
            &table
        ));
        assert!(detect_header(
            &records(&[&["code", "amount"], &["7", "1.5"], &["8", "2"]]),
            &[]
        ));
        assert!(!detect_header(
            &records(&[&["1", "2"], &["3", "4"]]),
            &table
        ));
        assert!(!detect_header(&records(&[&["a", "a"], &["1", "2"]]), &[]));
        assert!(!detect_header(&records(&[&["a", ""], &["1", "2"]]), &[]));
    }

    #[test]
    fn files_parse_with_quotes_newlines_and_blank_lines() {
        let file = "id,note\n1,\"multi\nline, with comma\"\n\n2,\"say \"\"hi\"\"\"\n";
        let parsed = parse(file.as_bytes(), options(",", true)).expect("parses");
        assert_eq!(parsed.columns, columns(&["id", "note"]));
        assert_eq!(
            parsed.rows,
            vec![
                vec!["1".to_owned(), "multi\nline, with comma".to_owned()],
                vec!["2".to_owned(), "say \"hi\"".to_owned()],
            ]
        );
        let unnamed = parse(b"1,a\n2,b\n", options(",", false)).expect("parses");
        assert_eq!(unnamed.columns, columns(&["Column 1", "Column 2"]));
        assert_eq!(unnamed.rows.len(), 2);
    }

    #[test]
    fn short_rows_are_padded_and_reported() {
        let parsed = parse(b"a,b,c\n1,2,3\n4,5\n", options(",", true)).expect("parses");
        assert_eq!(
            parsed.rows[1],
            vec!["4".to_owned(), "5".to_owned(), String::new()]
        );
        assert!(parsed.warnings[0].starts_with("1 row has fewer fields"));
    }

    #[test]
    fn bad_input_is_refused() {
        assert_eq!(parse(b"", options(",", true)), Err(CsvError::Empty));
        assert_eq!(parse(b"a,b\n", options(",", true)), Err(CsvError::NoRows));
        assert_eq!(
            parse(
                b"a\n",
                CsvOptions {
                    encoding: "ebcdic".into(),
                    ..options(",", true)
                }
            ),
            Err(CsvError::UnknownEncoding)
        );
        assert_eq!(
            parse(b"a\n", options(":", true)),
            Err(CsvError::UnknownDelimiter)
        );
        let wide = (0..=MAX_IMPORT_COLUMNS)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            parse(format!("{wide}\n{wide}\n").as_bytes(), options(",", true)),
            Err(CsvError::TooManyColumns)
        );
        let mut many = String::from("id\n");
        for i in 0..=MAX_IMPORT_ROWS {
            many.push_str(&format!("{i}\n"));
        }
        assert_eq!(
            parse(many.as_bytes(), options(",", true)),
            Err(CsvError::TooManyRows)
        );
    }

    #[test]
    fn detection_and_mapping_work_together() {
        let table = columns(&["id", "unit_price", "name"]);
        let file = b"Name;Unit price;Extra\nKeyboard;50,00;x\n";
        let detected = detect(file, &table).expect("detects");
        assert_eq!(detected, options(";", true));
        let parsed = parse(file, detected).expect("parses");
        assert_eq!(
            suggest_mapping(&parsed, &table),
            vec![Some("name".to_owned()), Some("unit_price".to_owned()), None]
        );
        // Without a header, by position only when the widths match.
        let positional = parse(b"1,2.5,Mouse\n", options(",", false)).expect("parses");
        assert_eq!(
            suggest_mapping(&positional, &table),
            table.iter().cloned().map(Some).collect::<Vec<_>>()
        );
        let narrower = parse(b"1,2.5\n", options(",", false)).expect("parses");
        assert_eq!(suggest_mapping(&narrower, &table), vec![None, None]);
    }

    #[test]
    fn samples_are_short_and_cut() {
        let mut file = String::from("note\n");
        file.push_str(&"x".repeat(CELL_CHAR_LIMIT + 10));
        file.push('\n');
        for i in 0..20 {
            file.push_str(&format!("{i}\n"));
        }
        let parsed = parse(file.as_bytes(), options(",", true)).expect("parses");
        let rows = sample(&parsed);
        assert_eq!(rows.len(), SAMPLE_ROWS);
        assert_eq!(rows[0][0].chars().count(), CELL_CHAR_LIMIT);
    }

    fn details() -> crate::domain::catalog::ObjectDetails {
        use crate::domain::catalog::{
            Column, Identity, KeyConstraint, ObjectDetails, ObjectKind, ObjectSummary,
        };
        let column = |name: &str, nullable: bool, default: Option<&str>| Column {
            name: name.into(),
            position: 1,
            data_type: "text".into(),
            nullable,
            default: default.map(Into::into),
            identity: None,
            generated: None,
            comment: None,
        };
        ObjectDetails {
            summary: ObjectSummary {
                schema: "public".into(),
                name: "items".into(),
                kind: ObjectKind::Table,
                row_estimate: Some(0),
                comment: None,
                readable: true,
            },
            columns: vec![
                column("id", false, None),
                column("sku", false, None),
                column("name", false, None),
                column("status", false, Some("'new'")),
                column("note", true, None),
                Column {
                    identity: Some(Identity::Always),
                    ..column("serial", false, None)
                },
                Column {
                    generated: Some("upper(name)".into()),
                    ..column("loud", true, None)
                },
            ],
            primary_key: Some(KeyConstraint {
                name: "items_pkey".into(),
                columns: vec!["id".into()],
            }),
            unique_constraints: vec![KeyConstraint {
                name: "items_sku_key".into(),
                columns: vec!["sku".into()],
            }],
            foreign_keys: Vec::new(),
            referenced_by: Vec::new(),
            indexes: Vec::new(),
            definition: None,
        }
    }

    fn file(text: &str) -> ParsedCsv {
        parse(text.as_bytes(), options(",", true)).expect("parses")
    }

    fn map(pairs: &[(usize, &str)]) -> Vec<ColumnMapping> {
        pairs
            .iter()
            .map(|(file_column, column)| ColumnMapping {
                file_column: *file_column,
                column: (*column).into(),
            })
            .collect()
    }

    #[test]
    fn imports_need_every_required_column_filled() {
        let parsed = file("id,sku,name\n1,A,Ada\n2,B,\n3,C,\n");
        let request = |mapping| CsvImportRequest {
            mapping,
            upsert_key: None,
        };
        let full = request(map(&[(0, "id"), (1, "sku"), (2, "name")]));
        assert_eq!(
            import_problems(&parsed, &details(), &full),
            vec!["name cannot be empty, but it is in rows 2 and 3.".to_owned()]
        );
        let missing = request(map(&[(0, "id"), (2, "name")]));
        assert!(import_problems(&parsed, &details(), &missing)[0].starts_with("sku needs a value"));
        // A column with a default may be left out, but an empty value is NULL, not the default.
        let status = file("id,sku,name,status\n1,A,Ada,\n");
        let with_status = request(map(&[(0, "id"), (1, "sku"), (2, "name"), (3, "status")]));
        assert_eq!(
            import_problems(&status, &details(), &with_status),
            vec!["status cannot be empty, but it is in row 1.".to_owned()]
        );
        let without = request(map(&[(0, "id"), (1, "sku"), (2, "name")]));
        assert!(import_problems(&status, &details(), &without).is_empty());
        let wrong = request(map(&[
            (0, "id"),
            (1, "serial"),
            (2, "loud"),
            (2, "id"),
            (9, "nope"),
        ]));
        let problems = import_problems(&parsed, &details(), &wrong);
        assert!(problems.iter().any(|p| p.contains("identity")));
        assert!(problems.iter().any(|p| p.contains("generated")));
        assert!(
            problems
                .iter()
                .any(|p| p.contains("more than one file column"))
        );
        assert!(
            problems
                .iter()
                .any(|p| p == "The table has no column nope.")
        );
        assert_eq!(
            import_problems(&parsed, &details(), &request(Vec::new())),
            vec!["Choose at least one file column to import.".to_owned()]
        );
    }

    #[test]
    fn updating_by_key_needs_a_real_filled_key() {
        // id is required too, so its empty value is reported either way.
        let parsed = file("id,sku,name\n1,A,Ada\n,B,Bo\n");
        let mapping = map(&[(0, "id"), (1, "sku"), (2, "name")]);
        let by = |key: &[&str]| CsvImportRequest {
            mapping: mapping.clone(),
            upsert_key: Some(key.iter().map(|k| (*k).to_owned()).collect()),
        };
        let required = "id cannot be empty, but it is in row 2.".to_owned();
        assert_eq!(
            import_problems(&parsed, &details(), &by(&["sku"])),
            vec![required.clone()]
        );
        assert_eq!(
            import_problems(&parsed, &details(), &by(&["id"])),
            vec![
                required,
                "id is part of the key to update by, but it is empty in row 2.".to_owned()
            ]
        );
        assert!(
            import_problems(&parsed, &details(), &by(&["name"]))
                .iter()
                .any(|p| p.contains("not the primary key"))
        );
        let filled = file("id,sku,name\n1,A,Ada\n2,B,Bo\n");
        let only_key = CsvImportRequest {
            mapping: map(&[(1, "sku"), (0, "id"), (2, "name")]),
            upsert_key: Some(vec!["sku".into()]),
        };
        assert!(import_problems(&filled, &details(), &only_key).is_empty());
        let nothing_else = CsvImportRequest {
            mapping: map(&[(1, "sku")]),
            upsert_key: Some(vec!["sku".into()]),
        };
        assert!(
            import_problems(&filled, &details(), &nothing_else)
                .iter()
                .any(|p| p.contains("besides the key"))
        );
    }

    #[test]
    fn rows_become_batches_in_file_order() {
        let mut text = String::from("id,note\n");
        for i in 0..2_500 {
            text.push_str(&format!("{i},{}\n", if i % 2 == 0 { "x" } else { "" }));
        }
        let parsed = file(&text);
        let table = crate::domain::catalog::ObjectName {
            schema: "public".into(),
            name: "items".into(),
        };
        let request = CsvImportRequest {
            mapping: map(&[(1, "note"), (0, "id")]),
            upsert_key: Some(vec!["id".into()]),
        };
        let (changes, starts) = import_changes(&parsed, &table, &request);
        assert_eq!(starts, vec![0, 1_000, 2_000]);
        assert_eq!(changes.len(), 3);
        let crate::domain::data_change::DataChange::Insert {
            columns,
            rows,
            upsert_key,
            ..
        } = &changes[0]
        else {
            panic!("an insert");
        };
        assert_eq!(columns, &vec!["note".to_owned(), "id".to_owned()]);
        assert_eq!(rows[0], vec![Some("x".to_owned()), Some("0".to_owned())]);
        // Empty fields are NULL.
        assert_eq!(rows[1], vec![None, Some("1".to_owned())]);
        assert_eq!(upsert_key, &Some(vec!["id".to_owned()]));
        assert_eq!(
            crate::domain::data_change::validate_set(
                &changes,
                crate::domain::data_change::IMPORT_LIMITS
            ),
            Ok(())
        );
        // Wide files get smaller batches, within the values limit.
        let wide = format!(
            "{}\n{}\n",
            (0..100)
                .map(|i| format!("c{i}"))
                .collect::<Vec<_>>()
                .join(","),
            vec!["1"; 100].join(",")
        );
        let parsed = file(&wide);
        let request = CsvImportRequest {
            mapping: (0..100)
                .map(|i| ColumnMapping {
                    file_column: i,
                    column: format!("c{i}"),
                })
                .collect(),
            upsert_key: None,
        };
        let (changes, _) = import_changes(&parsed, &table, &request);
        assert_eq!(changes.len(), 1);
    }

    /// The import dialog offers the same choices and states the same limits; CI fails if
    /// this copy is stale.
    #[test]
    fn export_csv_import_limits() {
        let quoted = |items: Vec<String>| {
            items
                .iter()
                .map(|item| format!("{item:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let ts = format!(
            "// This file was generated by `cargo test` from src-tauri/src/domain/csv_import.rs. \
             Do not edit this file manually.\n\n\
             /**\n * What CSV imports accept, as checked by the native core.\n */\n\
             export const CSV_IMPORT_LIMITS = {{\n  \
             maxBytes: {MAX_IMPORT_BYTES},\n  maxRows: {MAX_IMPORT_ROWS},\n  \
             maxColumns: {MAX_IMPORT_COLUMNS},\n  \
             encodings: [{}],\n  delimiters: [{}],\n\
             }} as const;\n",
            quoted(ENCODINGS.iter().map(|e| (*e).to_owned()).collect()),
            quoted(DELIMITERS.iter().map(char::to_string).collect()),
        );
        let dir = std::env::var_os("TS_RS_EXPORT_DIR").expect("set in .cargo/config.toml");
        std::fs::write(std::path::Path::new(&dir).join("CsvImportLimits.ts"), ts).expect("writes");
    }

    #[test]
    fn row_lists_stay_short() {
        assert_eq!(row_list(&[3]), "row 3");
        assert_eq!(row_list(&[3, 9, 12]), "rows 3, 9 and 12");
        assert_eq!(
            row_list(&[1, 2, 3, 4, 5, 6, 7]),
            "rows 1, 2, 3, 4, 5 and 2 more"
        );
    }
}
