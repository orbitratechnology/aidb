//! Schema details for the open connection, cached in memory.
//!
//! A cached catalog belongs to one session: reconnecting starts afresh. Before serving it, the
//! cache asks the database for its structural fingerprint once it is older than
//! [`RECHECK_AFTER`], and reloads after any DDL. Only the fingerprint and object count are
//! recorded on disk; the details stay in memory and never include row data.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::sessions::Sessions;
use crate::adapters::database::{DatabaseAdapter, DatabaseSession, DbErrorKind};
use crate::adapters::local_store::{LocalStore, StoreError};
use crate::domain::catalog::{CatalogOverview, ObjectDetails, ObjectName};
use crate::domain::connections::ConnectionId;
use crate::errors::{AppError, AppResult, ErrorCode};

/// How long a cached catalog is served before its fingerprint is checked again.
pub const RECHECK_AFTER: Duration = Duration::from_secs(15);

/// The overview as served, with when it was read from the database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct CatalogSnapshot {
    pub connection_id: ConnectionId,
    pub loaded_at: String,
    #[serde(flatten)]
    pub overview: CatalogOverview,
}

struct Entry {
    session_serial: u64,
    snapshot: Arc<CatalogSnapshot>,
    details: HashMap<ObjectName, Arc<ObjectDetails>>,
    /// When the fingerprint was last confirmed. `None` checks it again before the next use.
    checked_at: Option<Instant>,
}

/// Cached catalogs, one per open connection.
#[derive(Default)]
pub struct CatalogCache {
    entries: Mutex<HashMap<ConnectionId, Entry>>,
}

impl std::fmt::Debug for CatalogCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatalogCache")
            .field("connections", &self.lock().len())
            .finish_non_exhaustive()
    }
}

impl CatalogCache {
    /// Drops every cached catalog, as "Clear cached schema details" asks.
    pub fn clear(&self) {
        self.lock().clear();
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<ConnectionId, Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The cached entry for this session, if it is still the open one.
    fn current(
        &self,
        id: ConnectionId,
        serial: u64,
    ) -> Option<(Arc<CatalogSnapshot>, Option<Instant>)> {
        self.lock()
            .get(&id)
            .filter(|entry| entry.session_serial == serial)
            .map(|entry| (Arc::clone(&entry.snapshot), entry.checked_at))
    }

    /// Makes the next use of this snapshot check the fingerprint, unless it was replaced already.
    fn recheck_soon(&self, id: ConnectionId, snapshot: &Arc<CatalogSnapshot>) {
        if let Some(entry) = self.lock().get_mut(&id)
            && Arc::ptr_eq(&entry.snapshot, snapshot)
        {
            entry.checked_at = None;
        }
    }
}

/// The open connection's schemas, tables and views. `refresh` reloads them unconditionally.
///
/// # Errors
///
/// `unavailable` when the connection is not open, or the database's error.
pub async fn overview<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    cache: &CatalogCache,
    id: ConnectionId,
    refresh: bool,
) -> AppResult<Arc<CatalogSnapshot>> {
    prune(sessions, cache);
    let (session, serial) = sessions.get_with_serial(id).ok_or_else(not_connected)?;

    if !refresh && let Some((snapshot, checked_at)) = cache.current(id, serial) {
        if checked_at.is_some_and(|at| at.elapsed() < RECHECK_AFTER) {
            return Ok(snapshot);
        }
        let fingerprint = session.catalog_fingerprint().await?;
        if fingerprint == snapshot.overview.fingerprint {
            if let Some(entry) = cache.lock().get_mut(&id) {
                entry.checked_at = Some(Instant::now());
            }
            return Ok(snapshot);
        }
        tracing::info!(connection = %id, "schema changed; reloading its details");
    }

    let overview = session.catalog_overview().await?;
    let object_count = u32::try_from(overview.objects.len()).unwrap_or(u32::MAX);
    match store
        .schema_cache()
        .record(id, &overview.fingerprint, object_count)
        .await
    {
        Ok(()) | Err(StoreError::NotFound) => {}
        Err(error) => {
            tracing::warn!(error = ?AppError::from(error), "schema freshness not recorded")
        }
    }
    let snapshot = Arc::new(CatalogSnapshot {
        connection_id: id,
        loaded_at: now(),
        overview,
    });
    cache.lock().insert(
        id,
        Entry {
            session_serial: serial,
            snapshot: Arc::clone(&snapshot),
            details: HashMap::new(),
            checked_at: Some(Instant::now()),
        },
    );
    tracing::info!(connection = %id, objects = object_count, truncated = snapshot.overview.truncated, "schema details loaded");
    Ok(snapshot)
}

/// Columns, keys, references and indexes of one table or view of the open connection.
///
/// # Errors
///
/// `invalid_input` for an impossible name, `not_found` if the object does not exist, otherwise as
/// [`overview`].
pub async fn describe<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    cache: &CatalogCache,
    id: ConnectionId,
    object: ObjectName,
) -> AppResult<Arc<ObjectDetails>> {
    check_name(&object)?;
    // Revalidates the cache first, so details are never older than the overview.
    let snapshot = overview(store, sessions, cache, id, false).await?;
    if let Some(details) = cache
        .lock()
        .get(&id)
        .and_then(|entry| entry.details.get(&object))
    {
        return Ok(Arc::clone(details));
    }

    let (session, _) = sessions.get_with_serial(id).ok_or_else(not_connected)?;
    let details = match session.describe_object(&object).await {
        Ok(details) => Arc::new(details),
        Err(error) => {
            // A listed object that is gone means the schema changed: notice it on the next read
            // instead of up to `RECHECK_AFTER` later.
            if error.kind() == DbErrorKind::ObjectNotFound {
                cache.recheck_soon(id, &snapshot);
            }
            return Err(error.into());
        }
    };
    if let Some(entry) = cache.lock().get_mut(&id)
        && Arc::ptr_eq(&entry.snapshot, &snapshot)
    {
        entry.details.insert(object, Arc::clone(&details));
    }
    Ok(details)
}

/// PostgreSQL names are at most 63 bytes; anything else cannot exist.
fn check_name(object: &ObjectName) -> AppResult<()> {
    let valid = |part: &str| !part.is_empty() && part.len() <= 63 && !part.contains('\0');
    if valid(&object.schema) && valid(&object.name) {
        Ok(())
    } else {
        Err(AppError::invalid_input(
            "That is not a valid table or view name.",
        ))
    }
}

/// Forgets catalogs of connections that are no longer open.
fn prune<A: DatabaseAdapter>(sessions: &Sessions<A>, cache: &CatalogCache) {
    let open = sessions.open_ids();
    cache.lock().retain(|id, _| open.contains(id));
}

fn not_connected() -> AppError {
    AppError::new(
        ErrorCode::Unavailable,
        "This connection is not open. Connect to it first.",
    )
    .with_retryable(false)
}

fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;
    use crate::adapters::local_store::test_support::temp_store;
    use crate::domain::connections::ConnectionDraft;
    use crate::services::fakes::{sessions, target};

    async fn setup() -> (
        tempfile::TempDir,
        LocalStore,
        Arc<crate::services::fakes::FakeAdapter>,
        Sessions<Arc<crate::services::fakes::FakeAdapter>>,
        ConnectionId,
    ) {
        let (dir, store) = temp_store().await;
        let profile = store
            .connections()
            .insert(
                &ConnectionDraft::new("Orders", "db", "orders", "reader")
                    .validated()
                    .expect("valid"),
            )
            .await
            .expect("insert");
        let (adapter, sessions) = sessions();
        adapter.catalog.set_fingerprint("v1");
        sessions.open(profile.id, target("db")).await.expect("open");
        (dir, store, adapter, sessions, profile.id)
    }

    fn age(cache: &CatalogCache, id: ConnectionId) {
        if let Some(entry) = cache.lock().get_mut(&id) {
            entry.checked_at = None;
        }
    }

    #[tokio::test]
    async fn overviews_are_cached_and_recorded() {
        let (_dir, store, adapter, sessions, id) = setup().await;
        let cache = CatalogCache::default();

        let first = overview(&store, &sessions, &cache, id, false)
            .await
            .expect("load");
        let second = overview(&store, &sessions, &cache, id, false)
            .await
            .expect("cached");

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(adapter.catalog.overview_loads.load(Ordering::SeqCst), 1);
        assert_eq!(adapter.catalog.fingerprint_checks.load(Ordering::SeqCst), 0);
        let record = store
            .schema_cache()
            .get(id)
            .await
            .expect("get")
            .expect("recorded");
        assert_eq!(
            (record.fingerprint.as_str(), record.object_count),
            ("v1", 2)
        );
    }

    #[tokio::test]
    async fn stale_caches_are_kept_while_the_structure_is_unchanged() {
        let (_dir, store, adapter, sessions, id) = setup().await;
        let cache = CatalogCache::default();
        overview(&store, &sessions, &cache, id, false)
            .await
            .expect("load");
        age(&cache, id);

        overview(&store, &sessions, &cache, id, false)
            .await
            .expect("revalidated");

        assert_eq!(adapter.catalog.fingerprint_checks.load(Ordering::SeqCst), 1);
        assert_eq!(adapter.catalog.overview_loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn ddl_changes_reload_and_drop_cached_details() {
        let (_dir, store, adapter, sessions, id) = setup().await;
        let cache = CatalogCache::default();
        let orders = ObjectName {
            schema: "public".into(),
            name: "orders".into(),
        };
        describe(&store, &sessions, &cache, id, orders.clone())
            .await
            .expect("describe");
        describe(&store, &sessions, &cache, id, orders.clone())
            .await
            .expect("cached");
        assert_eq!(adapter.catalog.describe_loads.load(Ordering::SeqCst), 1);

        adapter.catalog.set_fingerprint("v2");
        age(&cache, id);
        let reloaded = overview(&store, &sessions, &cache, id, false)
            .await
            .expect("reload");
        describe(&store, &sessions, &cache, id, orders)
            .await
            .expect("describe again");

        assert_eq!(reloaded.overview.fingerprint, "v2");
        assert_eq!(adapter.catalog.overview_loads.load(Ordering::SeqCst), 2);
        assert_eq!(adapter.catalog.describe_loads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_missing_object_rechecks_the_schema_on_the_next_read() {
        let (_dir, store, adapter, sessions, id) = setup().await;
        let cache = CatalogCache::default();
        let dropped = ObjectName {
            schema: "public".into(),
            name: "dropped".into(),
        };
        describe(&store, &sessions, &cache, id, dropped)
            .await
            .expect_err("gone");

        adapter.catalog.set_fingerprint("v2");
        let reloaded = overview(&store, &sessions, &cache, id, false)
            .await
            .expect("reload");

        assert_eq!(adapter.catalog.fingerprint_checks.load(Ordering::SeqCst), 1);
        assert_eq!(reloaded.overview.fingerprint, "v2");
    }

    #[tokio::test]
    async fn refresh_reloads_and_reconnecting_starts_afresh() {
        let (_dir, store, adapter, sessions, id) = setup().await;
        let cache = CatalogCache::default();
        overview(&store, &sessions, &cache, id, false)
            .await
            .expect("load");

        overview(&store, &sessions, &cache, id, true)
            .await
            .expect("refresh");
        sessions.open(id, target("db")).await.expect("reconnect");
        overview(&store, &sessions, &cache, id, false)
            .await
            .expect("new session");

        assert_eq!(adapter.catalog.overview_loads.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn closed_connections_and_bad_names_are_refused() {
        let (_dir, store, _adapter, sessions, id) = setup().await;
        let cache = CatalogCache::default();
        overview(&store, &sessions, &cache, id, false)
            .await
            .expect("load");
        let too_long = ObjectName {
            schema: "public".into(),
            name: "x".repeat(64),
        };
        let missing = ObjectName {
            schema: "public".into(),
            name: "ghost".into(),
        };

        let invalid = describe(&store, &sessions, &cache, id, too_long)
            .await
            .expect_err("bad");
        let not_found = describe(&store, &sessions, &cache, id, missing)
            .await
            .expect_err("gone");
        sessions.close(id).await;
        let closed = overview(&store, &sessions, &cache, id, false)
            .await
            .expect_err("closed");

        assert_eq!(invalid.code(), ErrorCode::InvalidInput);
        assert_eq!(not_found.code(), ErrorCode::NotFound);
        assert_eq!(closed.code(), ErrorCode::Unavailable);
        assert!(!closed.is_retryable());
        assert!(
            cache.lock().is_empty(),
            "a closed connection's catalog is dropped"
        );
    }
}
