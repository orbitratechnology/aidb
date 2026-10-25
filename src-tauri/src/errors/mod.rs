//! The error contract between the native core and the webview.
//!
//! Domain modules define their own typed errors and convert them into [`AppError`] at the
//! command layer. An `AppError` carries a stable [`ErrorCode`], a message that is safe to show,
//! a retryable flag and, optionally, the internal cause. Only the first three cross the IPC
//! boundary. The cause exists for local logs, and is redacted there as well.

mod redact;

use std::borrow::Cow;
use std::error::Error as StdError;
use std::fmt;

use serde::{Serialize, Serializer};

pub use redact::{REDACTED, redact};

/// Result type returned by commands and services.
pub type AppResult<T> = Result<T, AppError>;

type BoxedCause = Box<dyn StdError + Send + Sync + 'static>;

/// Stable, machine-readable error category. The webview branches on this, never on messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorCode {
    /// Input failed validation at the boundary.
    InvalidInput,
    /// The referenced item does not exist.
    NotFound,
    /// The request conflicts with current state, such as a stale plan or approval.
    Conflict,
    /// Application policy or the database role refused the operation.
    PermissionDenied,
    /// The operation needs an approval it does not have.
    ApprovalRequired,
    /// The operation exceeded its time limit.
    Timeout,
    /// The user or the system cancelled the operation.
    Cancelled,
    /// A remote service (database, AI provider) could not be reached.
    Unavailable,
    /// The local metadata store failed.
    Storage,
    /// The OS credential store failed or is unavailable.
    CredentialStore,
    /// A bug or an unexpected condition in the core.
    Internal,
}

impl ErrorCode {
    /// Every code, for exhaustive tests and documentation.
    pub const ALL: [Self; 11] = [
        Self::InvalidInput,
        Self::NotFound,
        Self::Conflict,
        Self::PermissionDenied,
        Self::ApprovalRequired,
        Self::Timeout,
        Self::Cancelled,
        Self::Unavailable,
        Self::Storage,
        Self::CredentialStore,
        Self::Internal,
    ];

    /// The wire name, identical to the serialized form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_input",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::PermissionDenied => "permission_denied",
            Self::ApprovalRequired => "approval_required",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::Unavailable => "unavailable",
            Self::Storage => "storage",
            Self::CredentialStore => "credential_store",
            Self::Internal => "internal",
        }
    }

    /// Whether repeating the same request may succeed. Individual errors can override this.
    #[must_use]
    pub const fn is_retryable_by_default(self) -> bool {
        matches!(self, Self::Timeout | Self::Unavailable)
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An error that can be returned to the webview.
///
/// `Debug` and `Display` redact secrets. The cause is deliberately not exposed through
/// [`std::error::Error::source`], so generic error reporters cannot print it unredacted.
pub struct AppError {
    code: ErrorCode,
    message: Cow<'static, str>,
    retryable: bool,
    cause: Option<BoxedCause>,
}

impl AppError {
    /// Message shown for internal errors, which never reveal their cause.
    pub const INTERNAL_MESSAGE: &'static str =
        "Something went wrong in the native core. Details are in the local log.";

    /// Creates an error with a message that is safe to show to the user.
    pub fn new(code: ErrorCode, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: code.is_retryable_by_default(),
            cause: None,
        }
    }

    pub fn invalid_input(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorCode::InvalidInput, message)
    }

    pub fn not_found(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorCode::NotFound, message)
    }

    pub fn conflict(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorCode::Conflict, message)
    }

    pub fn permission_denied(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorCode::PermissionDenied, message)
    }

    pub fn timeout(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorCode::Timeout, message)
    }

    pub fn cancelled(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorCode::Cancelled, message)
    }

    pub fn unavailable(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(ErrorCode::Unavailable, message)
    }

    /// Wraps an unexpected failure. The user sees a generic message; the cause goes to the log.
    pub fn internal(cause: impl Into<BoxedCause>) -> Self {
        Self::new(ErrorCode::Internal, Self::INTERNAL_MESSAGE).with_cause(cause)
    }

    /// Attaches the underlying cause for local diagnostics.
    #[must_use]
    pub fn with_cause(mut self, cause: impl Into<BoxedCause>) -> Self {
        self.cause = Some(cause.into());
        self
    }

    /// Overrides the code's default retry hint.
    #[must_use]
    pub fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    #[must_use]
    pub fn code(&self) -> ErrorCode {
        self.code
    }

    /// The user-facing message, redacted.
    #[must_use]
    pub fn message(&self) -> Cow<'_, str> {
        redact(&self.message)
    }

    #[must_use]
    pub fn is_retryable(&self) -> bool {
        self.retryable
    }

    /// The redacted cause chain, outermost first, for local logs.
    fn cause_chain(&self) -> Vec<String> {
        let mut chain = Vec::new();
        let mut next: Option<&(dyn StdError + 'static)> = self
            .cause
            .as_deref()
            .map(|cause| cause as &(dyn StdError + 'static));
        while let Some(error) = next {
            chain.push(redact(&error.to_string()).into_owned());
            next = error.source();
        }
        chain
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message())
    }
}

impl fmt::Debug for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AppError")
            .field("code", &self.code)
            .field("message", &self.message())
            .field("retryable", &self.retryable)
            .field("cause", &self.cause_chain())
            .finish()
    }
}

impl StdError for AppError {}

/// The wire shape of an error, as the webview receives it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorPayload<'a> {
    code: ErrorCode,
    message: Cow<'a, str>,
    retryable: bool,
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ErrorPayload {
            code: self.code,
            message: self.message(),
            retryable: self.retryable,
        }
        .serialize(serializer)
    }
}

impl From<tauri::Error> for AppError {
    fn from(error: tauri::Error) -> Self {
        Self::internal(error)
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use serde_json::json;

    use super::*;

    #[test]
    fn serializes_only_code_message_and_retryable() {
        let cause = io::Error::other("connect to postgres://bob:hunter2@db failed");
        let error = AppError::unavailable("The database did not respond.").with_cause(cause);

        let value = serde_json::to_value(&error).expect("error serializes");

        assert_eq!(
            value,
            json!({
                "code": "unavailable",
                "message": "The database did not respond.",
                "retryable": true,
            })
        );
    }

    #[test]
    fn internal_errors_hide_their_cause_from_the_webview() {
        let error = AppError::internal(io::Error::other("token=abc123 rejected"));

        let wire = serde_json::to_string(&error).expect("error serializes");

        assert!(wire.contains(r#""code":"internal""#));
        assert!(wire.contains(AppError::INTERNAL_MESSAGE));
        assert!(!wire.contains("abc123"));
    }

    #[test]
    fn debug_output_redacts_the_whole_cause_chain() {
        let inner = io::Error::other("password=hunter2");
        let outer = io::Error::new(io::ErrorKind::ConnectionRefused, inner);
        let error = AppError::internal(outer);

        let debug = format!("{error:?}");

        assert!(debug.contains("password=[redacted]"));
        assert!(!debug.contains("hunter2"));
    }

    #[test]
    fn messages_are_redacted_even_when_a_caller_leaks_a_secret() {
        let error = AppError::invalid_input("Could not parse postgres://amy:pw@host/db");

        assert_eq!(
            error.message(),
            "Could not parse postgres://amy:[redacted]@host/db"
        );
        assert!(!error.to_string().contains(":pw@"));
    }

    #[test]
    fn source_is_not_exposed_to_generic_error_reporters() {
        let error = AppError::internal(io::Error::other("secret detail"));

        assert!(error.source().is_none());
    }

    #[test]
    fn retry_hint_follows_the_code_unless_overridden() {
        assert!(AppError::timeout("Timed out.").is_retryable());
        assert!(!AppError::invalid_input("Bad port.").is_retryable());
        assert!(
            !AppError::unavailable("Offline.")
                .with_retryable(false)
                .is_retryable()
        );
    }

    #[test]
    fn wire_names_match_serialized_codes() {
        for code in ErrorCode::ALL {
            assert_eq!(
                serde_json::to_value(code).expect("code serializes"),
                json!(code.as_str())
            );
        }
    }
}
