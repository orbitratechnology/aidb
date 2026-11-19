//! The versioned JSON file used to move connection profiles between machines.
//!
//! The file holds connection settings only: no passwords, no credential-store references and
//! no local identifiers. Unknown fields are ignored on import, so a hand-edited `password` never
//! reaches the app.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::connections::{ConnectionDraft, Engine, Environment, SslMode};
use crate::errors::AppError;

/// Identifies the file type.
pub const FORMAT: &str = "aidb.connections";
/// Version written by this build. Files from newer versions are refused.
pub const VERSION: u32 = 1;
/// Largest file accepted for import.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Most profiles accepted in one file.
pub const MAX_ENTRIES: usize = 1000;

/// One profile as written to the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileConnection {
    pub name: String,
    #[serde(default)]
    pub engine: Engine,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub database: String,
    pub username: String,
    #[serde(default)]
    pub ssl_mode: SslMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_cert_path: Option<String>,
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_secs: u32,
    #[serde(default)]
    pub environment: Environment,
    #[serde(default = "read_only_by_default")]
    pub read_only: bool,
}

const fn default_port() -> u16 {
    5432
}

const fn default_connect_timeout() -> u32 {
    10
}

const fn read_only_by_default() -> bool {
    true
}

impl From<&ConnectionDraft> for FileConnection {
    fn from(draft: &ConnectionDraft) -> Self {
        Self {
            name: draft.name.clone(),
            engine: Engine::Postgres,
            host: draft.host.clone(),
            port: draft.port,
            database: draft.database.clone(),
            username: draft.username.clone(),
            ssl_mode: draft.ssl_mode,
            ca_cert_path: draft.ca_cert_path.clone(),
            connect_timeout_secs: draft.connect_timeout_secs,
            environment: draft.environment,
            read_only: draft.read_only,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportFile<'a> {
    format: &'static str,
    version: u32,
    exported_at: &'a str,
    connections: Vec<FileConnection>,
}

/// Renders profiles as an export file (pretty JSON, trailing newline).
#[must_use]
pub fn render<'a>(
    profiles: impl IntoIterator<Item = &'a ConnectionDraft>,
    exported_at: &str,
) -> Vec<u8> {
    let file = ExportFile {
        format: FORMAT,
        version: VERSION,
        exported_at,
        connections: profiles.into_iter().map(FileConnection::from).collect(),
    };
    let mut bytes = serde_json::to_vec_pretty(&file).unwrap_or_default();
    bytes.push(b'\n');
    bytes
}

/// Why a whole file was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FileError {
    #[error("The file is larger than 1 MB, too large for a connections export.")]
    TooLarge,
    #[error("The file is not valid JSON.")]
    NotJson,
    #[error("The file is not an aidb connections export.")]
    WrongFormat,
    #[error("The file was made by a newer version of aidb. Update aidb to import it.")]
    NewerVersion,
    #[error("The file holds more than {MAX_ENTRIES} connections.")]
    TooManyEntries,
}

impl From<FileError> for AppError {
    fn from(error: FileError) -> Self {
        Self::invalid_input(error.to_string())
    }
}

/// One entry read from a file: a profile ready to validate, or why it could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// The entry's name if it has one, otherwise its position (`Entry 3`).
    pub label: String,
    pub content: Result<FileConnection, String>,
}

#[derive(Deserialize)]
struct RawFile {
    format: String,
    version: u32,
    #[serde(default)]
    connections: Vec<serde_json::Value>,
}

/// Reads an export file. Entries are parsed one by one so a bad entry does not hide the rest.
///
/// # Errors
///
/// A [`FileError`] when the file as a whole cannot be used.
pub fn parse(bytes: &[u8]) -> Result<Vec<FileEntry>, FileError> {
    if u64::try_from(bytes.len()).map_or(true, |len| len > MAX_FILE_BYTES) {
        return Err(FileError::TooLarge);
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| FileError::NotJson)?;
    let file: RawFile = serde_json::from_value(value).map_err(|_| FileError::WrongFormat)?;
    if file.format != FORMAT || file.version == 0 {
        return Err(FileError::WrongFormat);
    }
    if file.version > VERSION {
        return Err(FileError::NewerVersion);
    }
    if file.connections.len() > MAX_ENTRIES {
        return Err(FileError::TooManyEntries);
    }

    Ok(file
        .connections
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let label = value
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map_or_else(|| format!("Entry {}", index + 1), str::to_owned);
            let content = serde_json::from_value(value).map_err(|error| describe(&error));
            FileEntry { label, content }
        })
        .collect())
}

/// Names the missing field when that is the problem; never repeats a value from the file.
fn describe(error: &serde_json::Error) -> String {
    let text = error.to_string();
    text.strip_prefix("missing field `")
        .and_then(|rest| rest.split_once('`'))
        .map_or_else(
            || "A field is missing or has an invalid value.".to_owned(),
            |(field, _)| format!("The {field} field is missing."),
        )
}

impl FileConnection {
    /// The profile as it would be saved on import: always read-only until the user enables
    /// writes on this machine.
    #[must_use]
    pub fn into_import_draft(self) -> ConnectionDraft {
        ConnectionDraft {
            name: self.name,
            host: self.host,
            port: self.port,
            database: self.database,
            username: self.username,
            ssl_mode: self.ssl_mode,
            ca_cert_path: self.ca_cert_path,
            connect_timeout_secs: self.connect_timeout_secs,
            environment: self.environment,
            read_only: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(name: &str) -> ConnectionDraft {
        let mut draft = ConnectionDraft::new(name, "db.internal", "orders", "reader");
        draft.environment = Environment::Production;
        draft.read_only = false;
        draft
    }

    #[test]
    fn exports_round_trip_through_parse() {
        let profiles = [draft("Orders"), draft("Billing")];

        let bytes = render(&profiles, "2026-10-26T10:00:00Z");
        let entries = parse(&bytes).expect("parses");

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].label, "Orders");
        let first = entries[0].content.clone().expect("valid entry");
        assert_eq!(first.environment, Environment::Production);
        assert!(!first.read_only, "the file records the original setting");
        assert!(
            first.into_import_draft().read_only,
            "imports start read-only"
        );
    }

    #[test]
    fn exports_carry_format_version_and_no_local_details() {
        let bytes = render(&[draft("Orders")], "2026-10-26T10:00:00Z");
        let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json");

        assert_eq!(json["format"], FORMAT);
        assert_eq!(json["version"], VERSION);
        assert_eq!(json["exportedAt"], "2026-10-26T10:00:00Z");
        let entry = json["connections"][0].as_object().expect("object");
        for absent in ["id", "secretRef", "password", "createdAt", "updatedAt"] {
            assert!(!entry.contains_key(absent), "{absent} must not be exported");
        }
        assert!(bytes.ends_with(b"\n"));
    }

    #[test]
    fn missing_optional_fields_take_safe_defaults() {
        let file = br#"{"format":"aidb.connections","version":1,"connections":[
            {"name":"Minimal","host":"db","database":"app","username":"reader"}
        ]}"#;

        let entry = parse(file)
            .expect("parses")
            .remove(0)
            .content
            .expect("valid");

        assert_eq!(entry.port, 5432);
        assert_eq!(entry.ssl_mode, SslMode::VerifyFull);
        assert_eq!(entry.connect_timeout_secs, 10);
        assert!(entry.read_only);
    }

    #[test]
    fn passwords_in_hand_edited_files_are_ignored() {
        let file = br#"{"format":"aidb.connections","version":1,"connections":[
            {"name":"Edited","host":"db","database":"app","username":"reader","password":"hunter2"}
        ]}"#;

        let entry = parse(file)
            .expect("parses")
            .remove(0)
            .content
            .expect("valid");

        assert!(!format!("{entry:?}").contains("hunter2"));
    }

    #[test]
    fn bad_entries_are_reported_without_their_values() {
        let file = br#"{"format":"aidb.connections","version":1,"connections":[
            {"host":"db","database":"app","username":"reader"},
            {"name":"Odd port","host":"db","port":"hunter2","database":"app","username":"r"},
            {"name":"Fine","host":"db","database":"app","username":"reader"}
        ]}"#;

        let entries = parse(file).expect("parses");

        assert_eq!(entries[0].label, "Entry 1");
        assert_eq!(
            entries[0].content,
            Err("The name field is missing.".to_owned())
        );
        let problem = entries[1].content.clone().expect_err("invalid port");
        assert!(!problem.contains("hunter2"));
        assert!(entries[2].content.is_ok());
    }

    #[test]
    fn whole_files_are_refused_for_the_right_reasons() {
        let cases: [(&[u8], FileError); 4] = [
            (b"not json", FileError::NotJson),
            (br#"{"format":"other","version":1}"#, FileError::WrongFormat),
            (br#"[1, 2, 3]"#, FileError::WrongFormat),
            (
                br#"{"format":"aidb.connections","version":2,"connections":[]}"#,
                FileError::NewerVersion,
            ),
        ];
        for (bytes, expected) in cases {
            assert_eq!(parse(bytes), Err(expected));
        }

        let huge = vec![b' '; usize::try_from(MAX_FILE_BYTES).expect("fits") + 1];
        assert_eq!(parse(&huge), Err(FileError::TooLarge));
    }

    #[test]
    fn files_with_too_many_entries_are_refused() {
        let entry = r#"{"name":"x","host":"db","database":"app","username":"r"}"#;
        let entries = vec![entry; MAX_ENTRIES + 1].join(",");
        let file =
            format!(r#"{{"format":"aidb.connections","version":1,"connections":[{entries}]}}"#);

        assert_eq!(parse(file.as_bytes()), Err(FileError::TooManyEntries));
    }
}
