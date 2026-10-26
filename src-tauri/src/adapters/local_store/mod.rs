//! Local metadata store: one SQLite database in the app data directory.
//!
//! The schema is versioned by embedded migrations (`src-tauri/migrations`). Before an existing
//! database is upgraded, a consistent copy is written next to it, so a failed upgrade never loses
//! data. Secrets and query results are never stored here.

mod connections;
mod maintenance;
mod settings;

use std::path::{Path, PathBuf};
use std::time::Duration;

use sqlx::migrate::{MigrateError, Migrator};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};
use thiserror::Error;

use crate::errors::{AppError, ErrorCode};

pub use connections::ConnectionProfiles;
pub use maintenance::{ClearTarget, Maintenance, StorageSummary};
pub use settings::Settings;

/// File name of the local database inside the app data directory.
pub const DATABASE_FILE: &str = "aidb.sqlite3";

static MIGRATOR: Migrator = sqlx::migrate!();

const MAX_CONNECTIONS: u32 = 4;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("could not open the local database")]
    Open(#[source] sqlx::Error),
    #[error("could not back up the local database before upgrading it")]
    Backup(#[source] sqlx::Error),
    #[error("could not upgrade the local database schema")]
    Migrate(#[from] MigrateError),
    #[error("a record with this {0} already exists")]
    Duplicate(&'static str),
    #[error("the record does not exist")]
    NotFound,
    #[error("invalid setting key `{0}`")]
    InvalidKey(String),
    #[error("stored data is invalid: {0}")]
    Corrupt(String),
    #[error("local database query failed")]
    Query(#[source] sqlx::Error),
}

impl From<sqlx::Error> for StoreError {
    fn from(error: sqlx::Error) -> Self {
        Self::Query(error)
    }
}

impl From<StoreError> for AppError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::Duplicate(field) => {
                Self::conflict(format!("That {field} is already in use. Choose another."))
            }
            StoreError::NotFound => Self::not_found("The item no longer exists."),
            other => Self::new(
                ErrorCode::Storage,
                "The app's local database could not be read or written. Details are in the local log.",
            )
            .with_cause(other),
        }
    }
}

/// Handle to the local database. Cheap to clone; all clones share one connection pool.
#[derive(Debug, Clone)]
pub struct LocalStore {
    pool: SqlitePool,
    path: PathBuf,
}

impl LocalStore {
    /// Opens (or creates) the database in `data_dir` and applies pending migrations.
    ///
    /// # Errors
    ///
    /// Fails if the file cannot be opened, backed up before an upgrade, or migrated.
    pub async fn open(data_dir: &Path) -> Result<Self, StoreError> {
        Self::open_with(&data_dir.join(DATABASE_FILE), &MIGRATOR).await
    }

    async fn open_with(path: &Path, migrator: &Migrator) -> Result<Self, StoreError> {
        let existed = path.exists();
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(BUSY_TIMEOUT);
        let pool = SqlitePoolOptions::new()
            .max_connections(MAX_CONNECTIONS)
            .connect_with(options)
            .await
            .map_err(StoreError::Open)?;

        if existed {
            backup_before_upgrade(&pool, path, migrator).await?;
        }
        migrator.run(&pool).await?;

        Ok(Self {
            pool,
            path: path.to_owned(),
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The newest applied schema migration, or 0 for an empty database.
    ///
    /// # Errors
    ///
    /// Fails if the migrations table cannot be read.
    pub async fn schema_version(&self) -> Result<i64, StoreError> {
        applied_version(&self.pool).await
    }

    #[must_use]
    pub fn settings(&self) -> Settings<'_> {
        Settings::new(&self.pool)
    }

    #[must_use]
    pub fn connections(&self) -> ConnectionProfiles<'_> {
        ConnectionProfiles::new(&self.pool)
    }

    #[must_use]
    pub fn maintenance(&self) -> Maintenance<'_> {
        Maintenance::new(&self.pool)
    }

    /// Closes all connections, checkpointing the write-ahead log.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    #[cfg(test)]
    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

async fn applied_version(pool: &SqlitePool) -> Result<i64, StoreError> {
    let has_table: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations')",
    )
    .fetch_one(pool)
    .await?;
    if !has_table {
        return Ok(0);
    }
    let version: Option<i64> =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(pool)
            .await?;
    Ok(version.unwrap_or(0))
}

/// Writes a consistent copy of the database before applying migrations to existing data.
async fn backup_before_upgrade(
    pool: &SqlitePool,
    path: &Path,
    migrator: &Migrator,
) -> Result<(), StoreError> {
    let current = applied_version(pool).await?;
    let target = migrator.iter().map(|m| m.version).max().unwrap_or(0);
    if current == 0 || current >= target {
        return Ok(());
    }

    let backup = backup_path(path, current);
    // `VACUUM INTO` refuses to overwrite; an older copy of the same version is redundant.
    if backup.exists() {
        std::fs::remove_file(&backup)
            .map_err(|error| StoreError::Backup(sqlx::Error::Io(error)))?;
    }
    sqlx::query("VACUUM INTO ?")
        .bind(backup.to_string_lossy().into_owned())
        .execute(pool)
        .await
        .map_err(StoreError::Backup)?;

    tracing::info!(
        from_version = current,
        to_version = target,
        "backed up the local database before upgrading it"
    );
    Ok(())
}

fn backup_path(path: &Path, version: i64) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".v{version}.bak"));
    path.with_file_name(name)
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::LocalStore;

    /// A store in a fresh temporary directory. Keep the directory alive for the test's duration.
    pub(crate) async fn temp_store() -> (tempfile::TempDir, LocalStore) {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = LocalStore::open(dir.path()).await.expect("store opens");
        (dir, store)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::test_support::temp_store;
    use super::*;

    #[tokio::test]
    async fn a_new_database_is_migrated_to_the_latest_schema() {
        let (_dir, store) = temp_store().await;

        assert_eq!(store.schema_version().await.expect("version"), 1);
        assert!(store.path().ends_with(DATABASE_FILE));
    }

    #[tokio::test]
    async fn connections_use_wal_and_enforce_foreign_keys() {
        let (_dir, store) = temp_store().await;

        let journal: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(store.pool())
            .await
            .expect("pragma");
        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(store.pool())
            .await
            .expect("pragma");

        assert_eq!(journal, "wal");
        assert_eq!(foreign_keys, 1);
    }

    #[tokio::test]
    async fn reopening_keeps_data_and_does_not_reapply_migrations() {
        let dir = tempfile::tempdir().expect("temp dir");
        let store = LocalStore::open(dir.path()).await.expect("first open");
        store
            .settings()
            .set("ui.theme", &"dark")
            .await
            .expect("setting saved");
        store.close().await;

        let reopened = LocalStore::open(dir.path()).await.expect("second open");

        assert_eq!(reopened.schema_version().await.expect("version"), 1);
        let theme: Option<String> = reopened.settings().get("ui.theme").await.expect("read");
        assert_eq!(theme.as_deref(), Some("dark"));
        assert!(!backup_path(reopened.path(), 1).exists());
    }

    #[tokio::test]
    async fn upgrading_an_existing_database_writes_a_backup_first() {
        let dir = tempfile::tempdir().expect("temp dir");
        let migrations = dir.path().join("migrations");
        fs::create_dir(&migrations).expect("migrations dir");
        fs::write(
            migrations.join("0001_notes.sql"),
            "CREATE TABLE notes (body TEXT NOT NULL) STRICT;",
        )
        .expect("v1");
        let db_path = dir.path().join(DATABASE_FILE);

        let v1 = Migrator::new(migrations.as_path())
            .await
            .expect("v1 migrator");
        let store = LocalStore::open_with(&db_path, &v1)
            .await
            .expect("v1 opens");
        sqlx::query("INSERT INTO notes (body) VALUES ('kept')")
            .execute(store.pool())
            .await
            .expect("insert");
        store.close().await;

        fs::write(
            migrations.join("0002_tags.sql"),
            "CREATE TABLE tags (name TEXT NOT NULL) STRICT;",
        )
        .expect("v2");
        let v2 = Migrator::new(migrations.as_path())
            .await
            .expect("v2 migrator");
        let upgraded = LocalStore::open_with(&db_path, &v2)
            .await
            .expect("v2 opens");

        assert_eq!(upgraded.schema_version().await.expect("version"), 2);
        let backup = backup_path(&db_path, 1);
        assert!(backup.exists(), "backup of v1 must exist");
        let backup_store = LocalStore::open_with(&backup, &v1)
            .await
            .expect("backup opens");
        let body: String = sqlx::query_scalar("SELECT body FROM notes")
            .fetch_one(backup_store.pool())
            .await
            .expect("backup holds v1 data");
        assert_eq!(body, "kept");
    }

    #[tokio::test]
    async fn audit_rows_cannot_be_edited_but_can_be_cleared() {
        let (_dir, store) = temp_store().await;
        sqlx::query(
            "INSERT INTO operation_audit (actor, operation, status) VALUES ('user', 'connect', 'succeeded')",
        )
        .execute(store.pool())
        .await
        .expect("insert");

        let edit = sqlx::query("UPDATE operation_audit SET status = 'failed'")
            .execute(store.pool())
            .await;
        let clear = sqlx::query("DELETE FROM operation_audit")
            .execute(store.pool())
            .await;

        let message = edit.expect_err("update is rejected").to_string();
        assert!(message.contains("append-only"), "{message}");
        assert_eq!(clear.expect("delete allowed").rows_affected(), 1);
    }

    #[tokio::test]
    async fn check_constraints_reject_values_outside_the_contract() {
        let (_dir, store) = temp_store().await;

        let result = sqlx::query(
            "INSERT INTO connection_profiles (id, name, host, port, database_name, username, environment)
             VALUES ('x', 'n', 'h', 5432, 'd', 'u', 'prod')",
        )
        .execute(store.pool())
        .await;

        assert!(result.is_err());
    }

    #[test]
    fn storage_failures_map_to_storage_errors_without_details() {
        let error = AppError::from(StoreError::Corrupt("bad row".into()));

        assert_eq!(error.code(), ErrorCode::Storage);
        assert!(!error.message().contains("bad row"));
    }
}
