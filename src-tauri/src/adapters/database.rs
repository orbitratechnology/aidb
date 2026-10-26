//! The boundary every database engine implements. Services and, later, the policy engine use
//! these traits and [`DbError`] kinds, never a driver directly. PostgreSQL is the only engine
//! today; the integration tests in `tests/postgres.rs` hold the contract a new engine must pass.

use std::borrow::Cow;
use std::future::Future;
use std::time::Duration;

use secrecy::SecretString;
use thiserror::Error;

use crate::domain::connections::ConnectionDraft;
use crate::domain::database::{Capabilities, OperationId, ServerInfo, SessionLimits};
use crate::errors::{AppError, ErrorCode};

/// Everything needed to open a session. The password stays wrapped until the driver needs it.
#[derive(Debug, Clone)]
pub struct ConnectTarget {
    pub settings: ConnectionDraft,
    pub password: Option<SecretString>,
    pub limits: SessionLimits,
}

pub trait DatabaseAdapter: Send + Sync + 'static {
    type Session: DatabaseSession;

    fn capabilities(&self) -> Capabilities;

    /// Opens a session and reads the server's details. Fails within the connect timeout.
    fn connect(
        &self,
        target: ConnectTarget,
    ) -> impl Future<Output = Result<Self::Session, DbError>> + Send;
}

/// `Debug` output must never include credentials.
pub trait DatabaseSession: std::fmt::Debug + Send + Sync + 'static {
    fn server_info(&self) -> &ServerInfo;

    /// One round trip to the server. Returns how long it took.
    fn ping(&self) -> impl Future<Output = Result<Duration, DbError>> + Send;

    /// Asks the server to stop a running operation. Returns `false` if it is not running.
    fn cancel(&self, operation: OperationId) -> impl Future<Output = Result<bool, DbError>> + Send;

    /// Closes every connection of the session.
    fn close(&self) -> impl Future<Output = ()> + Send;
}

/// What went wrong, independent of the driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbErrorKind {
    /// The host cannot be found or nothing listens on the port.
    Unreachable,
    /// The server did not answer within the connect timeout.
    ConnectTimeout,
    /// TLS could not be set up, or the server's certificate is not trusted.
    Tls,
    /// The user name or password was rejected.
    Authentication,
    /// The server's access rules refuse this client.
    Rejected,
    DatabaseNotFound,
    PermissionDenied,
    /// A write was attempted in a read-only session.
    ReadOnly,
    /// The user cancelled the statement.
    Cancelled,
    StatementTimeout,
    LockTimeout,
    /// The server is starting, stopping or out of connection slots.
    ServerUnavailable,
    /// An open connection dropped.
    ConnectionLost,
    Other,
}

/// A database failure with a message that is safe to show: it never contains the password or
/// the connection URI. The driver's error stays as the cause, for local logs only.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct DbError {
    kind: DbErrorKind,
    message: Cow<'static, str>,
    sqlstate: Option<String>,
    #[source]
    cause: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl DbError {
    pub fn new(kind: DbErrorKind, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            kind,
            message: message.into(),
            sqlstate: None,
            cause: None,
        }
    }

    #[must_use]
    pub fn with_sqlstate(mut self, sqlstate: impl Into<String>) -> Self {
        self.sqlstate = Some(sqlstate.into());
        self
    }

    #[must_use]
    pub fn with_cause(
        mut self,
        cause: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        self.cause = Some(cause.into());
        self
    }

    #[must_use]
    pub fn kind(&self) -> DbErrorKind {
        self.kind
    }

    /// The server's SQLSTATE code, when it reported one.
    #[must_use]
    pub fn sqlstate(&self) -> Option<&str> {
        self.sqlstate.as_deref()
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<DbError> for AppError {
    fn from(error: DbError) -> Self {
        use DbErrorKind as Kind;
        let (code, retryable) = match error.kind {
            Kind::Unreachable | Kind::ServerUnavailable | Kind::ConnectionLost => {
                (ErrorCode::Unavailable, true)
            }
            Kind::ConnectTimeout => (ErrorCode::Timeout, true),
            // Retrying would not fix a certificate, and would rerun a slow statement.
            Kind::Tls => (ErrorCode::Unavailable, false),
            Kind::StatementTimeout | Kind::LockTimeout => (ErrorCode::Timeout, false),
            Kind::Authentication | Kind::Rejected | Kind::PermissionDenied | Kind::ReadOnly => {
                (ErrorCode::PermissionDenied, false)
            }
            Kind::DatabaseNotFound => (ErrorCode::NotFound, false),
            Kind::Cancelled => (ErrorCode::Cancelled, false),
            Kind::Other => (ErrorCode::Internal, false),
        };
        let message = error.message.clone();
        Self::new(code, message)
            .with_retryable(retryable)
            .with_cause(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_map_to_codes_and_retry_hints() {
        let cases = [
            (DbErrorKind::Unreachable, ErrorCode::Unavailable, true),
            (DbErrorKind::ConnectTimeout, ErrorCode::Timeout, true),
            (DbErrorKind::Tls, ErrorCode::Unavailable, false),
            (
                DbErrorKind::Authentication,
                ErrorCode::PermissionDenied,
                false,
            ),
            (DbErrorKind::ReadOnly, ErrorCode::PermissionDenied, false),
            (DbErrorKind::DatabaseNotFound, ErrorCode::NotFound, false),
            (DbErrorKind::Cancelled, ErrorCode::Cancelled, false),
            (DbErrorKind::StatementTimeout, ErrorCode::Timeout, false),
            (DbErrorKind::ConnectionLost, ErrorCode::Unavailable, true),
        ];
        for (kind, code, retryable) in cases {
            let error = AppError::from(DbError::new(kind, "message"));
            assert_eq!(
                (error.code(), error.is_retryable()),
                (code, retryable),
                "{kind:?}"
            );
            assert_eq!(error.message(), "message");
        }
    }
}
