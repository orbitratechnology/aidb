//! A question as the UI sends it: the text, the tables the user picked, files they attached and
//! the conversation so far. Everything is checked here before any of it reaches a model.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::context::{Attachment, AttachmentKind, ContextItem, MAX_QUESTION_CHARS};
use super::task::{Exchange, MAX_ANSWER_CHARS, MAX_HISTORY};
use crate::domain::catalog::ObjectName;

/// Files per question.
pub const MAX_ATTACHMENTS: usize = 5;
/// Size of one attached file, in bytes of text.
pub const MAX_ATTACHMENT_BYTES: usize = 512 * 1024;
/// Length of an attached file's name, in characters.
pub const MAX_ATTACHMENT_NAME_CHARS: usize = 255;
/// Tables the user may pick to describe in full.
pub const MAX_SELECTED_TABLES: usize = 10;

/// A file the user attached, as the UI read it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttachmentInput {
    pub name: String,
    pub content: String,
}

/// A question for the assistant.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AskRequest {
    pub question: String,
    /// Tables described in full, in addition to the schema overview.
    #[serde(default)]
    pub selected: Vec<ObjectName>,
    #[serde(default)]
    pub attachments: Vec<AttachmentInput>,
    /// Earlier questions and answers of this conversation, oldest first.
    #[serde(default)]
    pub history: Vec<Exchange>,
    /// The user allowed sending data values (query results, attached files) to a hosted
    /// provider for this conversation. Local providers need no permission.
    #[serde(default)]
    pub share_data: bool,
}

/// An [`AskRequest`] that passed every check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedAsk {
    pub question: String,
    pub selected: Vec<ObjectName>,
    pub attachments: Vec<Attachment>,
    pub history: Vec<Exchange>,
    pub share_data: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AskError {
    #[error("Write a question first.")]
    EmptyQuestion,
    #[error("The question is too long ({MAX_QUESTION_CHARS} characters at most).")]
    QuestionTooLong,
    #[error("Attach at most {MAX_ATTACHMENTS} files.")]
    TooManyAttachments,
    #[error("“{0}” cannot be attached: use a .sql, .csv, .json, .md or .txt file.")]
    UnsupportedFile(String),
    #[error("“{0}” is not a text file.")]
    NotText(String),
    #[error("“{0}” is too large to attach ({kb} KB at most).", kb = MAX_ATTACHMENT_BYTES / 1024)]
    FileTooLarge(String),
    #[error("Give every attached file a name of at most {MAX_ATTACHMENT_NAME_CHARS} characters.")]
    BadFileName,
    #[error("Pick at most {MAX_SELECTED_TABLES} tables.")]
    TooManyTables,
    #[error("The conversation is too long to send; start a new one.")]
    HistoryTooLong,
}

impl AskRequest {
    /// Checks every limit; duplicate tables are dropped.
    ///
    /// # Errors
    ///
    /// The first limit the request breaks.
    pub fn checked(self) -> Result<CheckedAsk, AskError> {
        let question = self.question.trim().to_owned();
        if question.is_empty() {
            return Err(AskError::EmptyQuestion);
        }
        if question.chars().count() > MAX_QUESTION_CHARS {
            return Err(AskError::QuestionTooLong);
        }
        if self.attachments.len() > MAX_ATTACHMENTS {
            return Err(AskError::TooManyAttachments);
        }
        let attachments = self
            .attachments
            .into_iter()
            .map(check_attachment)
            .collect::<Result<Vec<_>, _>>()?;
        let mut selected: Vec<ObjectName> = Vec::with_capacity(self.selected.len());
        for object in self.selected {
            if !selected.contains(&object) {
                selected.push(object);
            }
        }
        if selected.len() > MAX_SELECTED_TABLES {
            return Err(AskError::TooManyTables);
        }
        if self.history.len() > MAX_HISTORY
            || self.history.iter().any(|exchange| {
                exchange.question.chars().count() > MAX_QUESTION_CHARS
                    || exchange.answer.chars().count() > MAX_ANSWER_CHARS
            })
        {
            return Err(AskError::HistoryTooLong);
        }
        // An exchange without an answer (a cancelled question) adds nothing to build on.
        let history = self
            .history
            .into_iter()
            .filter(|exchange| !exchange.answer.trim().is_empty())
            .collect();
        Ok(CheckedAsk {
            question,
            selected,
            attachments,
            history,
            share_data: self.share_data,
        })
    }
}

fn check_attachment(input: AttachmentInput) -> Result<Attachment, AskError> {
    // Only the file's own name is kept, never a path, and nothing that could break a line.
    let name: String = input
        .name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    let name = name.trim().to_owned();
    if name.is_empty() || name.chars().count() > MAX_ATTACHMENT_NAME_CHARS {
        return Err(AskError::BadFileName);
    }
    let kind = AttachmentKind::from_file_name(&name)
        .ok_or_else(|| AskError::UnsupportedFile(name.clone()))?;
    if input.content.len() > MAX_ATTACHMENT_BYTES {
        return Err(AskError::FileTooLarge(name));
    }
    if input.content.contains('\0') {
        return Err(AskError::NotText(name));
    }
    Ok(Attachment {
        name,
        kind,
        content: input.content,
    })
}

/// The provider a question goes to, as the user sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct AssistantProvider {
    pub name: String,
    pub model: String,
    /// Runs on another machine: data values need the user's permission.
    pub hosted: bool,
}

/// What would be sent for a question, before it is sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ContextPreview {
    pub provider: AssistantProvider,
    pub items: Vec<ContextItem>,
    pub estimated_tokens: u32,
    pub budget_tokens: u32,
    /// Query results may go to the model (a local provider, or the user allowed it).
    pub data_allowed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(question: &str) -> AskRequest {
        AskRequest {
            question: question.into(),
            selected: Vec::new(),
            attachments: Vec::new(),
            history: Vec::new(),
            share_data: false,
        }
    }

    fn file(name: &str, content: &str) -> AttachmentInput {
        AttachmentInput {
            name: name.into(),
            content: content.into(),
        }
    }

    #[test]
    fn questions_are_trimmed_and_bounded() {
        assert_eq!(
            request("  How many?  ").checked().expect("ok").question,
            "How many?"
        );
        assert_eq!(request("   ").checked(), Err(AskError::EmptyQuestion));
        let long = "x".repeat(MAX_QUESTION_CHARS + 1);
        assert_eq!(request(&long).checked(), Err(AskError::QuestionTooLong));
    }

    #[test]
    fn attachments_must_be_small_named_text_files() {
        let mut ask = request("q");
        ask.attachments = vec![file("C:\\work\\report\n.csv", "a,b\n1,2")];
        let checked = ask.checked().expect("ok");
        assert_eq!(checked.attachments[0].name, "report.csv");
        assert_eq!(checked.attachments[0].kind, AttachmentKind::Csv);

        let mut ask = request("q");
        ask.attachments = vec![file("photo.png", "x")];
        assert_eq!(
            ask.checked(),
            Err(AskError::UnsupportedFile("photo.png".into()))
        );
        let mut ask = request("q");
        ask.attachments = vec![file("dump.sql", "a\0b")];
        assert_eq!(ask.checked(), Err(AskError::NotText("dump.sql".into())));
        let mut ask = request("q");
        ask.attachments = vec![file("big.txt", &"x".repeat(MAX_ATTACHMENT_BYTES + 1))];
        assert_eq!(ask.checked(), Err(AskError::FileTooLarge("big.txt".into())));
        let mut ask = request("q");
        ask.attachments = vec![file("/tmp/", "x")];
        assert_eq!(ask.checked(), Err(AskError::BadFileName));
        let mut ask = request("q");
        ask.attachments = (0..=MAX_ATTACHMENTS)
            .map(|i| file(&format!("{i}.md"), "x"))
            .collect();
        assert_eq!(ask.checked(), Err(AskError::TooManyAttachments));
    }

    #[test]
    fn tables_are_deduplicated_and_bounded() {
        let table = |name: &str| ObjectName {
            schema: "public".into(),
            name: name.into(),
        };
        let mut ask = request("q");
        ask.selected = vec![table("a"), table("a"), table("b")];
        assert_eq!(ask.checked().expect("ok").selected.len(), 2);
        let mut ask = request("q");
        ask.selected = (0..=MAX_SELECTED_TABLES)
            .map(|i| table(&i.to_string()))
            .collect();
        assert_eq!(ask.checked(), Err(AskError::TooManyTables));
    }

    #[test]
    fn history_is_bounded() {
        let exchange = Exchange {
            question: "q".into(),
            answer: "a".into(),
        };
        let mut ask = request("q");
        ask.history = vec![exchange.clone(); MAX_HISTORY];
        assert!(ask.clone().checked().is_ok());
        ask.history.push(exchange);
        assert_eq!(ask.checked(), Err(AskError::HistoryTooLong));
    }

    #[test]
    fn requests_reject_unknown_fields() {
        let json = serde_json::json!({"question": "q", "tools": []});
        assert!(serde_json::from_value::<AskRequest>(json).is_err());
    }

    /// The composer checks files with the same limits; CI fails if this copy is stale.
    #[test]
    fn export_assistant_limits() {
        let ts = format!(
            "// This file was generated by `cargo test` from src-tauri/src/domain/agent/ask.rs. \
             Do not edit this file manually.\n\n\
             /**\n * Limits of a question to the assistant, as checked by the native core.\n */\n\
             export const ASSISTANT_LIMITS = {{\n  \
             questionMaxChars: {question},\n  maxAttachments: {files},\n  \
             attachmentMaxBytes: {bytes},\n  attachmentNameMaxChars: {name},\n  \
             maxSelectedTables: {tables},\n  maxHistory: {history},\n  \
             answerMaxChars: {answer},\n\
             }} as const;\n",
            question = MAX_QUESTION_CHARS,
            files = MAX_ATTACHMENTS,
            bytes = MAX_ATTACHMENT_BYTES,
            name = MAX_ATTACHMENT_NAME_CHARS,
            tables = MAX_SELECTED_TABLES,
            history = MAX_HISTORY,
            answer = MAX_ANSWER_CHARS,
        );
        let dir = std::env::var_os("TS_RS_EXPORT_DIR").expect("set in .cargo/config.toml");
        std::fs::write(std::path::Path::new(&dir).join("AssistantLimits.ts"), ts).expect("writes");
    }
}
