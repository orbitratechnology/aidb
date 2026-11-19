//! Query history and saved queries: SQL the user ran, and SQL they chose to keep.
//!
//! History is written by the core after each run, with credentials redacted from the text, and
//! kept to the most recent [`HISTORY_LIMIT`] entries. Saved queries are kept until deleted.

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::connections::ConnectionId;
use crate::errors::{AppError, ErrorCode, redact};

/// Most history entries kept; older ones are removed as new ones arrive.
pub const HISTORY_LIMIT: u32 = 1_000;
/// Longest SQL text kept in history, in characters.
pub const HISTORY_SQL_CHARS: usize = 100_000;
/// Longest saved query name, in characters.
pub const SAVED_NAME_CHARS: usize = 200;
/// Longest saved query text, in characters (the editor never runs more than 1 MB).
pub const SAVED_SQL_CHARS: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum HistoryStatus {
    Completed,
    Refused,
    Failed,
    Cancelled,
}

impl HistoryStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Refused => "refused",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// # Errors
    ///
    /// For any other text (a damaged row).
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "completed" => Ok(Self::Completed),
            "refused" => Ok(Self::Refused),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(format!("unknown history status `{other}`")),
        }
    }
}

/// One run of a statement from the editor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    #[cfg_attr(test, ts(type = "number"))]
    pub id: i64,
    pub executed_at: String,
    pub connection_id: Option<ConnectionId>,
    /// The statement as run, with credentials redacted.
    pub sql: String,
    pub status: HistoryStatus,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub row_count: Option<u64>,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub elapsed_ms: Option<u64>,
    /// The error category of a failed run.
    pub error_code: Option<ErrorCode>,
}

/// What to record about a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewHistoryEntry {
    pub connection_id: ConnectionId,
    pub sql: String,
    pub status: HistoryStatus,
    pub row_count: Option<u64>,
    pub elapsed_ms: Option<u64>,
    pub error_code: Option<ErrorCode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SavedQuery {
    pub id: String,
    pub name: String,
    pub sql: String,
    /// The connection it was saved from, if that profile still exists.
    pub connection_id: Option<ConnectionId>,
    pub created_at: String,
    pub updated_at: String,
}

/// A new saved query, or new contents for an existing one (`id`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavedQueryDraft {
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub id: Option<String>,
    pub name: String,
    pub sql: String,
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub connection_id: Option<ConnectionId>,
}

impl SavedQueryDraft {
    /// The draft with its name trimmed, if it fits the limits.
    ///
    /// # Errors
    ///
    /// `invalid_input` for an empty or overlong name, or empty or overlong SQL.
    pub fn validated(mut self) -> Result<Self, AppError> {
        self.name = self.name.trim().to_owned();
        if self.name.is_empty() {
            return Err(AppError::invalid_input("Give the query a name."));
        }
        if self.name.chars().count() > SAVED_NAME_CHARS {
            return Err(AppError::invalid_input(format!(
                "Names can be at most {SAVED_NAME_CHARS} characters."
            )));
        }
        if self.sql.trim().is_empty() {
            return Err(AppError::invalid_input("There is no SQL to save."));
        }
        if self.sql.chars().count() > SAVED_SQL_CHARS {
            return Err(AppError::invalid_input(
                "That SQL is too long to save (1 MB at most).",
            ));
        }
        if let Some(id) = &self.id
            && uuid::Uuid::parse_str(id).is_err()
        {
            return Err(AppError::invalid_input("That saved query does not exist."));
        }
        Ok(self)
    }
}

/// `PASSWORD '...'` in role statements, which the generic `key = value` rule does not cover.
static SQL_PASSWORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b(password)(\s+)('(?:[^']|'')*'|"(?:[^"]|"")*")"#)
        .expect("redaction pattern must compile")
});

/// The text to keep in history: credentials and keys redacted, cut to [`HISTORY_SQL_CHARS`].
#[must_use]
pub fn history_text(sql: &str) -> String {
    let redacted = redact(sql);
    let redacted: Cow<'_, str> = if SQL_PASSWORD.is_match(&redacted) {
        Cow::Owned(
            SQL_PASSWORD
                .replace_all(&redacted, "${1}${2}'[redacted]'")
                .into_owned(),
        )
    } else {
        redacted
    };
    match redacted.char_indices().nth(HISTORY_SQL_CHARS) {
        Some((end, _)) => format!("{} …", &redacted[..end]),
        None => redacted.into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_text_hides_credentials() {
        assert_eq!(
            history_text("ALTER ROLE app PASSWORD 'it''s secret'"),
            "ALTER ROLE app PASSWORD '[redacted]'"
        );
        assert_eq!(
            history_text("create user x with encrypted password \"pw\""),
            "create user x with encrypted password '[redacted]'"
        );
        let keyed = history_text("SELECT * FROM t WHERE api_key = 'sk-abcdefghijklmnopqrstu'");
        assert!(!keyed.contains("sk-abc"), "{keyed}");
        assert_eq!(history_text("SELECT 1"), "SELECT 1");
    }

    #[test]
    fn history_text_is_capped() {
        let long = "x".repeat(HISTORY_SQL_CHARS + 10);
        let kept = history_text(&long);
        assert_eq!(kept.chars().count(), HISTORY_SQL_CHARS + 2);
        assert!(kept.ends_with(" …"));
    }

    #[test]
    fn saved_query_drafts_are_checked() {
        let draft = |name: &str, sql: &str| SavedQueryDraft {
            id: None,
            name: name.into(),
            sql: sql.into(),
            connection_id: None,
        };
        assert_eq!(
            draft("  Top customers ", "SELECT 1")
                .validated()
                .ok()
                .map(|d| d.name),
            Some("Top customers".into())
        );
        for (name, sql) in [
            ("", "SELECT 1"),
            ("ok", "  "),
            (&"n".repeat(201), "SELECT 1"),
        ] {
            let error = draft(name, sql).validated().expect_err("invalid");
            assert_eq!(error.code(), ErrorCode::InvalidInput);
        }
        let mut bad_id = draft("ok", "SELECT 1");
        bad_id.id = Some("not-a-uuid".into());
        assert!(bad_id.validated().is_err());
    }

    #[test]
    fn statuses_round_trip() {
        for status in [
            HistoryStatus::Completed,
            HistoryStatus::Refused,
            HistoryStatus::Failed,
            HistoryStatus::Cancelled,
        ] {
            assert_eq!(HistoryStatus::parse(status.as_str()), Ok(status));
        }
    }
}
