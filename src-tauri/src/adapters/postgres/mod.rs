//! PostgreSQL through SQLx.
//!
//! A session is a small pool of connections that all start with the same server settings
//! (statement and lock timeouts, read-only transactions). Cancelling an operation sends
//! `pg_cancel_backend` from a separate connection to the backend running it; a connection that
//! was sent a cancel is closed instead of returned to the pool, so a late signal can never hit
//! the next operation.

mod errors;
mod options;

use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgConnectOptions, PgConnection, PgPool, PgPoolOptions};
use sqlx::{Connection, Postgres};

use self::errors::{Endpoint, Phase, classify, connect_timeout};
pub use self::options::APPLICATION_NAME;
use super::database::{ConnectTarget, DatabaseAdapter, DatabaseSession, DbError, DbErrorKind};
use crate::domain::connections::Engine;
use crate::domain::database::{Capabilities, OperationId, SUPPORTED_POSTGRES_VERSIONS, ServerInfo};

/// Connections per session: enough for a query, a page load and a cancel at the same time.
const MAX_CONNECTIONS: u32 = 4;
/// Idle connections are closed after this long.
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);

const CAPABILITIES: Capabilities = Capabilities {
    engine: Engine::Postgres,
    cancel_statements: true,
    statement_timeout: true,
    lock_timeout: true,
    read_only_sessions: true,
    transactional_ddl: true,
    tls: true,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct PostgresAdapter;

impl DatabaseAdapter for PostgresAdapter {
    type Session = PgSession;

    fn capabilities(&self) -> Capabilities {
        CAPABILITIES
    }

    async fn connect(&self, target: ConnectTarget) -> Result<PgSession, DbError> {
        PgSession::open(target).await
    }
}

type Running = Arc<Mutex<HashMap<OperationId, RunningOperation>>>;

#[derive(Debug, Clone, Copy)]
struct RunningOperation {
    backend_pid: i32,
    cancel_sent: bool,
}

/// An open PostgreSQL session.
pub struct PgSession {
    pool: PgPool,
    options: PgConnectOptions,
    connect_timeout: Duration,
    endpoint: Endpoint,
    info: ServerInfo,
    running: Running,
}

impl std::fmt::Debug for PgSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The driver options hold the password; never print them.
        f.debug_struct("PgSession")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl PgSession {
    async fn open(target: ConnectTarget) -> Result<Self, DbError> {
        let options = options::build(&target);
        let endpoint = Endpoint {
            host: target.settings.host.clone(),
            port: target.settings.port,
            database: target.settings.database.clone(),
            username: target.settings.username.clone(),
            has_ca_file: target.settings.ca_cert_path.is_some(),
        };
        let connect_timeout = target.limits.connect_timeout;

        // The first connection is opened directly so a failure reports its real cause; the pool
        // would retry until its timeout and report only that.
        let mut first = connect(&options, connect_timeout, &endpoint).await?;
        let info = read_server_info(&mut first)
            .await
            .map_err(|error| classify(error, &endpoint, Phase::Connect))?;
        let _ = first.close().await;

        let pool = PgPoolOptions::new()
            .max_connections(MAX_CONNECTIONS)
            .min_connections(0)
            .acquire_timeout(connect_timeout)
            .idle_timeout(IDLE_TIMEOUT)
            .test_before_acquire(true)
            .connect_lazy_with(options.clone());

        Ok(Self {
            pool,
            options,
            connect_timeout,
            endpoint,
            info,
            running: Arc::default(),
        })
    }

    /// Takes a connection for one cancellable operation. The operation runs on the returned
    /// connection; dropping it ends the operation.
    ///
    /// # Errors
    ///
    /// If no connection can be acquired, or `id` is already running.
    pub async fn begin_operation(&self, id: OperationId) -> Result<OperationConnection, DbError> {
        let mut connection = self
            .pool
            .acquire()
            .await
            .map_err(|e| self.session_error(e))?;
        let backend_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *connection)
            .await
            .map_err(|e| self.session_error(e))?;
        let mut running = lock(&self.running);
        if running.contains_key(&id) {
            return Err(DbError::new(
                DbErrorKind::Other,
                "That operation is already running.",
            ));
        }
        running.insert(
            id,
            RunningOperation {
                backend_pid,
                cancel_sent: false,
            },
        );
        Ok(OperationConnection {
            id,
            connection: Some(connection),
            running: Arc::clone(&self.running),
        })
    }

    /// Classifies an error from a statement run on this session.
    #[must_use]
    pub fn session_error(&self, error: sqlx::Error) -> DbError {
        classify(error, &self.endpoint, Phase::Session)
    }
}

impl DatabaseSession for PgSession {
    fn server_info(&self) -> &ServerInfo {
        &self.info
    }

    async fn ping(&self) -> Result<Duration, DbError> {
        let started = Instant::now();
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map_err(|e| self.session_error(e))?;
        Ok(started.elapsed())
    }

    async fn cancel(&self, operation: OperationId) -> Result<bool, DbError> {
        let backend_pid = {
            let mut running = lock(&self.running);
            let Some(entry) = running.get_mut(&operation) else {
                return Ok(false);
            };
            entry.cancel_sent = true;
            entry.backend_pid
        };
        // A separate connection, so a busy pool cannot delay the cancel.
        let mut connection = connect(&self.options, self.connect_timeout, &self.endpoint).await?;
        let signalled: bool = sqlx::query_scalar("SELECT pg_cancel_backend($1)")
            .bind(backend_pid)
            .fetch_one(&mut connection)
            .await
            .map_err(|e| self.session_error(e))?;
        let _ = connection.close().await;
        tracing::info!(operation = %operation, signalled, "cancel sent to the database");
        Ok(signalled)
    }

    async fn close(&self) {
        self.pool.close().await;
    }
}

/// A pooled connection reserved for one operation, which can be cancelled while it runs.
pub struct OperationConnection {
    id: OperationId,
    connection: Option<PoolConnection<Postgres>>,
    running: Running,
}

impl std::fmt::Debug for OperationConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationConnection")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Deref for OperationConnection {
    type Target = PgConnection;

    fn deref(&self) -> &PgConnection {
        self.connection
            .as_deref()
            .unwrap_or_else(|| unreachable_connection())
    }
}

impl DerefMut for OperationConnection {
    fn deref_mut(&mut self) -> &mut PgConnection {
        match self.connection.as_deref_mut() {
            Some(connection) => connection,
            None => unreachable_connection(),
        }
    }
}

impl Drop for OperationConnection {
    fn drop(&mut self) {
        let cancel_sent = lock(&self.running)
            .remove(&self.id)
            .is_some_and(|entry| entry.cancel_sent);
        if cancel_sent && let Some(connection) = self.connection.take() {
            // Close the connection rather than reuse a backend a late cancel could still reach.
            drop(connection.detach());
        }
    }
}

#[expect(
    clippy::panic,
    reason = "the connection is only taken out in Drop, after which no deref can happen"
)]
fn unreachable_connection() -> ! {
    panic!("operation connection used after release")
}

async fn connect(
    options: &PgConnectOptions,
    timeout: Duration,
    endpoint: &Endpoint,
) -> Result<PgConnection, DbError> {
    match tokio::time::timeout(timeout, PgConnection::connect_with(options)).await {
        Ok(Ok(connection)) => Ok(connection),
        Ok(Err(error)) => Err(classify(error, endpoint, Phase::Connect)),
        Err(_) => Err(connect_timeout(endpoint)),
    }
}

#[derive(sqlx::FromRow)]
struct ServerRow {
    version: String,
    version_number: i32,
    current_user: String,
    database: String,
    in_recovery: bool,
    encrypted: bool,
    tls_version: Option<String>,
    read_only: bool,
}

async fn read_server_info(connection: &mut PgConnection) -> Result<ServerInfo, sqlx::Error> {
    let row: ServerRow = sqlx::query_as(
        "SELECT current_setting('server_version') AS version, \
                current_setting('server_version_num')::int4 AS version_number, \
                current_user::text AS current_user, \
                current_database()::text AS database, \
                pg_is_in_recovery() AS in_recovery, \
                COALESCE(ssl.ssl, false) AS encrypted, \
                ssl.version AS tls_version, \
                current_setting('default_transaction_read_only') = 'on' AS read_only \
         FROM (SELECT 1) AS one \
         LEFT JOIN pg_stat_ssl AS ssl ON ssl.pid = pg_backend_pid()",
    )
    .fetch_one(connection)
    .await?;
    let version_number = u32::try_from(row.version_number).unwrap_or_default();
    Ok(ServerInfo {
        engine: Engine::Postgres,
        // Packaged builds append details, such as "16.4 (Ubuntu 16.4-1)".
        version: row
            .version
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_owned(),
        version_number,
        supported_version: SUPPORTED_POSTGRES_VERSIONS.contains(&version_number),
        user: row.current_user,
        database: row.database,
        encrypted: row.encrypted,
        tls_version: row.tls_version,
        read_only: row.read_only,
        in_recovery: row.in_recovery,
    })
}

fn lock(running: &Running) -> MutexGuard<'_, HashMap<OperationId, RunningOperation>> {
    // The map is only inserted into and removed from; a panic cannot leave it half-updated.
    running.lock().unwrap_or_else(PoisonError::into_inner)
}
