//! The boundary every database engine implements. Services and, later, the policy engine use
//! these traits and [`DbError`] kinds, never a driver directly. PostgreSQL is the only engine
//! today; the integration tests in `tests/postgres.rs` hold the contract a new engine must pass.

use std::borrow::Cow;
use std::future::Future;
use std::time::Duration;

use secrecy::SecretString;
use thiserror::Error;

use crate::domain::catalog::{CatalogOverview, ObjectDetails, ObjectName};
use crate::domain::connections::ConnectionDraft;
use crate::domain::data_change::{DataChange, DataChangePlan};
use crate::domain::database::{Capabilities, OperationId, ServerInfo, SessionLimits};
use crate::domain::export::{ExportSummary, RowSink};
use crate::domain::policy::facts::TargetTable;
use crate::domain::query::QueryRows;
use crate::domain::sql::ReadShape;
use crate::domain::table_data::{PageLimits, PageRequest, TablePage};
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

    /// A digest of the database's structure. It changes when schemas, tables, views, columns,
    /// constraints, indexes, defaults or comments change, and not when statistics are refreshed.
    fn catalog_fingerprint(&self) -> impl Future<Output = Result<String, DbError>> + Send;

    /// Schemas and the tables and views in them, without their columns.
    fn catalog_overview(&self) -> impl Future<Output = Result<CatalogOverview, DbError>> + Send;

    /// Columns, keys, references and indexes of one table or view.
    fn describe_object(
        &self,
        object: &ObjectName,
    ) -> impl Future<Output = Result<ObjectDetails, DbError>> + Send;

    /// One page of a table or view, read in a read-only transaction. Column names in the
    /// request are matched against the live table; values are bound, never formatted into SQL.
    /// The read runs as `request.operation_id` when given, so it can be cancelled.
    fn read_page(
        &self,
        request: &PageRequest,
        limits: PageLimits,
    ) -> impl Future<Output = Result<TablePage, DbError>> + Send;

    /// Streams every row matching `request` (its offset and limit are ignored), up to `max_rows`,
    /// to `sink`, with complete values, in a read-only transaction. Runs as
    /// `request.operation_id` when given, so it can be cancelled.
    fn export_rows(
        &self,
        request: &PageRequest,
        max_rows: u32,
        sink: &mut (dyn RowSink + Send),
    ) -> impl Future<Output = Result<ExportSummary, ExportError>> + Send;

    /// Runs one statement already classified as a read, in a read-only transaction that is
    /// rolled back, returning at most `limits.max_rows` rows from `offset`. Errors carry the
    /// server's message and, when it gave one, the position within `sql`.
    fn run_read(
        &self,
        sql: &str,
        shape: ReadShape,
        limits: PageLimits,
        offset: u64,
        operation: Option<OperationId>,
    ) -> impl Future<Output = Result<QueryRows, DbError>> + Send;

    /// The planner's estimate of the rows one INSERT, UPDATE, DELETE or MERGE would write.
    /// Plans it with `EXPLAIN` in a read-only transaction that is rolled back: nothing runs.
    fn estimate_written_rows(&self, sql: &str)
    -> impl Future<Output = Result<u64, DbError>> + Send;

    /// For tables a statement writes (as written, resolved with the session's search path):
    /// whether they exist, what the role may do to them, and which tables delete with them.
    fn write_targets(
        &self,
        targets: &[String],
    ) -> impl Future<Output = Result<Vec<TargetTable>, DbError>> + Send;

    /// Makes `change` ready to run against the live table, in a read-only transaction: writes
    /// its statement, counts the rows an UPDATE or DELETE matches (refusing more than
    /// [`MAX_CHANGED_ROWS`](crate::domain::data_change::MAX_CHANGED_ROWS)), and has the
    /// database convert every value to its column's type. With `preview`, also reads some of
    /// the matching rows as they are now. Nothing is written.
    fn plan_data_change(
        &self,
        change: &DataChange,
        preview: Option<PageLimits>,
    ) -> impl Future<Output = Result<DataChangePlan, DbError>> + Send;

    /// Applies `change` in one transaction, only if it still plans to exactly `approved` (same
    /// statement, parameters, matching rows and stored values): locks the table against
    /// structure changes and the matching rows against other writers, runs the statement,
    /// checks the rows written and their stored values, and commits. Any difference rolls it
    /// back. Returns the rows written.
    fn apply_data_change(
        &self,
        change: &DataChange,
        approved: &DataChangePlan,
    ) -> impl Future<Output = Result<u64, ApplyError>> + Send;
}

/// Why an approved change was not applied.
#[derive(Debug, Error)]
pub enum ApplyError {
    /// It failed or a check refused it, and was rolled back: nothing was written.
    #[error(transparent)]
    RolledBack(DbError),
    /// The connection was lost while committing: it may or may not have been written.
    #[error(transparent)]
    Uncertain(DbError),
}

impl From<ExportError> for AppError {
    fn from(error: ExportError) -> Self {
        match error {
            ExportError::Database(error) => error.into(),
            ExportError::Write(error) => crate::adapters::files::FileError::Io(error).into(),
        }
    }
}

/// Why an export stopped: the database (or the request), or writing the output.
#[derive(Debug, Error)]
pub enum ExportError {
    #[error(transparent)]
    Database(DbError),
    #[error("writing the export failed")]
    Write(#[source] std::io::Error),
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
    /// A table or view that no longer exists, or never did.
    ObjectNotFound,
    /// The request does not fit the data: an unknown column, a value the column's type cannot
    /// hold, or a comparison the type does not support.
    InvalidRequest,
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
    /// Where in the statement the server found the problem: a character position, from 1.
    position: Option<u32>,
    #[source]
    cause: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl DbError {
    pub fn new(kind: DbErrorKind, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            kind,
            message: message.into(),
            sqlstate: None,
            position: None,
            cause: None,
        }
    }

    /// Replaces the message, for callers that show the server's own wording (the SQL editor).
    #[must_use]
    pub fn with_message(mut self, message: impl Into<Cow<'static, str>>) -> Self {
        self.message = message.into();
        self
    }

    #[must_use]
    pub const fn with_position(mut self, position: Option<u32>) -> Self {
        self.position = position;
        self
    }

    /// Character position in the statement, from 1, when the server reported one.
    #[must_use]
    pub const fn position(&self) -> Option<u32> {
        self.position
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
            Kind::DatabaseNotFound | Kind::ObjectNotFound => (ErrorCode::NotFound, false),
            Kind::InvalidRequest => (ErrorCode::InvalidInput, false),
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
            (DbErrorKind::InvalidRequest, ErrorCode::InvalidInput, false),
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
