//! Open database sessions, at most one per connection profile.
//!
//! Connecting reads the profile, its password from the credential store and the limits from
//! Safety settings. A session keeps those limits until it is closed; changing the profile closes
//! it, so a new read-only setting can never be bypassed by an older session.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use secrecy::SecretString;
use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::{connection_secrets, settings};
use crate::adapters::credentials::Credentials;
use crate::adapters::database::{ConnectTarget, DatabaseAdapter, DatabaseSession, DbError};
use crate::adapters::local_store::LocalStore;
use crate::domain::connections::{ConnectionDraft, ConnectionId};
use crate::domain::database::{Capabilities, ServerInfo, SessionLimits};
use crate::errors::{AppError, AppResult};

/// An open session as the UI sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ActiveSession {
    pub connection_id: ConnectionId,
    pub capabilities: Capabilities,
    pub server: ServerInfo,
    pub connected_at: String,
}

/// The result of a connection test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ConnectionTestReport {
    pub server: ServerInfo,
    /// Time for one query round trip after connecting.
    pub round_trip_ms: u32,
}

struct Entry<S> {
    session: Arc<S>,
    active: ActiveSession,
}

/// The sessions currently open, keyed by profile.
pub struct Sessions<A: DatabaseAdapter> {
    adapter: A,
    open: Mutex<HashMap<ConnectionId, Entry<A::Session>>>,
}

impl<A: DatabaseAdapter> std::fmt::Debug for Sessions<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sessions")
            .field("open", &self.lock().len())
            .finish_non_exhaustive()
    }
}

impl<A: DatabaseAdapter> Sessions<A> {
    pub fn new(adapter: A) -> Self {
        Self {
            adapter,
            open: Mutex::default(),
        }
    }

    /// Opens a session for a profile, replacing (and closing) any session it already had.
    ///
    /// # Errors
    ///
    /// The adapter's [`DbError`] if the connection fails; an existing session stays open then.
    pub async fn open(
        &self,
        id: ConnectionId,
        target: ConnectTarget,
    ) -> Result<ActiveSession, DbError> {
        let session = self.adapter.connect(target).await?;
        let active = ActiveSession {
            connection_id: id,
            capabilities: self.adapter.capabilities(),
            server: session.server_info().clone(),
            connected_at: now(),
        };
        let replaced = self.lock().insert(
            id,
            Entry {
                session: Arc::new(session),
                active: active.clone(),
            },
        );
        if let Some(old) = replaced {
            old.session.close().await;
        }
        Ok(active)
    }

    /// Closes a profile's session. Returns whether one was open.
    pub async fn close(&self, id: ConnectionId) -> bool {
        let removed = self.lock().remove(&id);
        match removed {
            Some(entry) => {
                entry.session.close().await;
                true
            }
            None => false,
        }
    }

    /// Closes every session, for shutdown.
    pub async fn close_all(&self) {
        let entries: Vec<_> = self.lock().drain().map(|(_, entry)| entry).collect();
        for entry in entries {
            entry.session.close().await;
        }
    }

    #[must_use]
    pub fn get(&self, id: ConnectionId) -> Option<Arc<A::Session>> {
        self.lock().get(&id).map(|entry| Arc::clone(&entry.session))
    }

    /// Open sessions, oldest first.
    #[must_use]
    pub fn list(&self) -> Vec<ActiveSession> {
        let mut sessions: Vec<_> = self.lock().values().map(|e| e.active.clone()).collect();
        sessions.sort_by(|a, b| a.connected_at.cmp(&b.connected_at));
        sessions
    }

    /// Connects, makes one round trip and disconnects. Nothing is kept.
    ///
    /// # Errors
    ///
    /// The adapter's [`DbError`] if connecting or the round trip fails.
    pub async fn test(&self, target: ConnectTarget) -> Result<ConnectionTestReport, DbError> {
        let session = self.adapter.connect(target).await?;
        let round_trip = session.ping().await;
        session.close().await;
        Ok(ConnectionTestReport {
            server: session.server_info().clone(),
            round_trip_ms: u32::try_from(round_trip?.as_millis()).unwrap_or(u32::MAX),
        })
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<ConnectionId, Entry<A::Session>>> {
        // Inserts and removals only; a panic cannot leave the map half-updated.
        self.open.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Opens a session for a saved profile. Uses `password` if given, otherwise the saved one.
///
/// # Errors
///
/// `not_found` for an unknown profile, the credential store's or database's error otherwise.
pub async fn connect<A: DatabaseAdapter>(
    store: &LocalStore,
    credentials: &Credentials,
    sessions: &Sessions<A>,
    id: ConnectionId,
    password: Option<SecretString>,
) -> AppResult<ActiveSession> {
    let profile = store
        .connections()
        .get(id)
        .await?
        .ok_or_else(|| AppError::not_found("The connection no longer exists."))?;
    let password = match password {
        Some(password) => Some(password),
        None => connection_secrets::load_password(credentials, id).await?,
    };
    let target = target(store, profile.settings, password).await?;
    let active = sessions.open(id, target).await?;
    tracing::info!(
        connection = %id,
        server_version = %active.server.version,
        encrypted = active.server.encrypted,
        read_only = active.server.read_only,
        "connected"
    );
    Ok(active)
}

/// Tests settings before (or without) saving them. With no `password`, uses the one saved for
/// `saved_id`, if any.
///
/// # Errors
///
/// `invalid_input` for bad settings or unconfirmed unencrypted access to a remote host; the
/// database's error if the test fails.
pub async fn test<A: DatabaseAdapter>(
    store: &LocalStore,
    credentials: &Credentials,
    sessions: &Sessions<A>,
    settings: ConnectionDraft,
    password: Option<SecretString>,
    saved_id: Option<ConnectionId>,
    unencrypted_allowed: bool,
) -> AppResult<ConnectionTestReport> {
    let settings = settings.validated()?;
    settings.check_transport(unencrypted_allowed)?;
    let password = match (password, saved_id) {
        (Some(password), _) => Some(password),
        (None, Some(id)) => connection_secrets::load_password(credentials, id).await?,
        (None, None) => None,
    };
    let target = target(store, settings, password).await?;
    Ok(sessions.test(target).await?)
}

async fn target(
    store: &LocalStore,
    settings: ConnectionDraft,
    password: Option<SecretString>,
) -> AppResult<ConnectTarget> {
    let safety = settings::load(store).await?.safety;
    Ok(ConnectTarget {
        limits: SessionLimits::new(&settings, &safety),
        settings,
        password,
    })
}

fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    use secrecy::ExposeSecret;

    use super::*;
    use crate::adapters::database::DbErrorKind;
    use crate::adapters::local_store::test_support::temp_store;
    use crate::domain::connections::{Engine, SslMode};
    use crate::domain::database::OperationId;
    use crate::domain::settings::{SafetySettings, SettingsPatch};
    use crate::errors::ErrorCode;

    /// Records what it was asked to connect with; fails for the host `unreachable`.
    #[derive(Default)]
    struct FakeAdapter {
        connects: AtomicUsize,
        last_target: Mutex<Option<ConnectTarget>>,
    }

    #[derive(Debug)]
    struct FakeSession {
        info: ServerInfo,
        closed: Arc<AtomicBool>,
    }

    impl DatabaseAdapter for Arc<FakeAdapter> {
        type Session = FakeSession;

        fn capabilities(&self) -> Capabilities {
            Capabilities {
                engine: Engine::Postgres,
                cancel_statements: false,
                statement_timeout: true,
                lock_timeout: true,
                read_only_sessions: true,
                transactional_ddl: true,
                tls: true,
            }
        }

        async fn connect(&self, target: ConnectTarget) -> Result<FakeSession, DbError> {
            self.connects.fetch_add(1, Ordering::SeqCst);
            let unreachable = target.settings.host == "unreachable";
            let read_only = target.limits.read_only;
            *self.last_target.lock().expect("lock") = Some(target);
            if unreachable {
                return Err(DbError::new(DbErrorKind::Unreachable, "Nothing there."));
            }
            Ok(FakeSession {
                info: ServerInfo {
                    engine: Engine::Postgres,
                    version: "17.2".into(),
                    version_number: 170_002,
                    supported_version: true,
                    user: "reader".into(),
                    database: "orders".into(),
                    encrypted: true,
                    tls_version: Some("TLSv1.3".into()),
                    read_only,
                    in_recovery: false,
                },
                closed: Arc::default(),
            })
        }
    }

    impl DatabaseSession for FakeSession {
        fn server_info(&self) -> &ServerInfo {
            &self.info
        }

        async fn ping(&self) -> Result<Duration, DbError> {
            Ok(Duration::from_millis(3))
        }

        async fn cancel(&self, _operation: OperationId) -> Result<bool, DbError> {
            Ok(false)
        }

        async fn close(&self) {
            self.closed.store(true, Ordering::SeqCst);
        }
    }

    fn sessions() -> (Arc<FakeAdapter>, Sessions<Arc<FakeAdapter>>) {
        let adapter = Arc::new(FakeAdapter::default());
        (Arc::clone(&adapter), Sessions::new(adapter))
    }

    fn target(host: &str) -> ConnectTarget {
        let settings = ConnectionDraft::new("Orders", host, "orders", "reader");
        ConnectTarget {
            limits: SessionLimits::new(&settings, &SafetySettings::default()),
            settings,
            password: None,
        }
    }

    #[tokio::test]
    async fn reconnecting_replaces_and_closes_the_old_session() {
        let (_, sessions) = sessions();
        let id = ConnectionId::new();
        sessions.open(id, target("db")).await.expect("first");
        let first = sessions.get(id).expect("open");

        sessions.open(id, target("db")).await.expect("second");

        assert!(first.closed.load(Ordering::SeqCst));
        assert_eq!(sessions.list().len(), 1);
    }

    #[tokio::test]
    async fn a_failed_reconnect_keeps_the_open_session() {
        let (_, sessions) = sessions();
        let id = ConnectionId::new();
        sessions.open(id, target("db")).await.expect("first");

        let error = sessions
            .open(id, target("unreachable"))
            .await
            .expect_err("fails");

        assert_eq!(error.kind(), DbErrorKind::Unreachable);
        assert!(
            !sessions
                .get(id)
                .expect("still open")
                .closed
                .load(Ordering::SeqCst)
        );
    }

    #[tokio::test]
    async fn closing_reports_whether_a_session_was_open() {
        let (_, sessions) = sessions();
        let id = ConnectionId::new();
        sessions.open(id, target("db")).await.expect("open");
        let session = sessions.get(id).expect("open");

        assert!(sessions.close(id).await);
        assert!(!sessions.close(id).await);
        assert!(session.closed.load(Ordering::SeqCst));
        assert!(sessions.list().is_empty());
    }

    #[tokio::test]
    async fn tests_keep_nothing_open() {
        let (adapter, sessions) = sessions();

        let report = sessions.test(target("db")).await.expect("test");

        assert_eq!(report.round_trip_ms, 3);
        assert_eq!(adapter.connects.load(Ordering::SeqCst), 1);
        assert!(sessions.list().is_empty());
    }

    #[tokio::test]
    async fn connecting_uses_the_saved_password_and_current_safety_limits() {
        let (_dir, store) = temp_store().await;
        let credentials = Credentials::in_memory_as_os_store();
        let (adapter, sessions) = sessions();
        let profile = store
            .connections()
            .insert(
                &ConnectionDraft::new("Orders", "db.internal", "orders", "reader")
                    .validated()
                    .expect("valid"),
            )
            .await
            .expect("insert");
        connection_secrets::save_password(
            &store,
            &credentials,
            profile.id,
            SecretString::from("saved-pw".to_owned()),
        )
        .await
        .expect("save password");
        settings::update(
            &store,
            SettingsPatch {
                safety: Some(SafetySettings {
                    statement_timeout_secs: 90,
                    ..SafetySettings::default()
                }),
                ..SettingsPatch::default()
            },
        )
        .await
        .expect("settings");

        let active = connect(&store, &credentials, &sessions, profile.id, None)
            .await
            .expect("connect");

        let used = adapter
            .last_target
            .lock()
            .expect("lock")
            .clone()
            .expect("connected");
        assert_eq!(
            used.password.as_ref().map(|p| p.expose_secret()),
            Some("saved-pw")
        );
        assert_eq!(used.limits.statement_timeout, Duration::from_secs(90));
        assert!(used.limits.read_only);
        assert_eq!(active.connection_id, profile.id);
        assert!(active.server.read_only);
    }

    #[tokio::test]
    async fn testing_unsaved_settings_checks_them_first() {
        let (_dir, store) = temp_store().await;
        let credentials = Credentials::in_memory_as_os_store();
        let (adapter, sessions) = sessions();
        let mut plaintext = ConnectionDraft::new("Plain", "db.internal", "orders", "reader");
        plaintext.ssl_mode = SslMode::Disable;

        let refused = test(
            &store,
            &credentials,
            &sessions,
            plaintext.clone(),
            None,
            None,
            false,
        )
        .await
        .expect_err("needs consent");
        let tested = test(&store, &credentials, &sessions, plaintext, None, None, true).await;

        assert_eq!(refused.code(), ErrorCode::InvalidInput);
        assert!(tested.is_ok());
        assert_eq!(
            adapter.connects.load(Ordering::SeqCst),
            1,
            "refused before connecting"
        );
    }

    #[tokio::test]
    async fn unknown_profiles_cannot_connect() {
        let (_dir, store) = temp_store().await;
        let (_, sessions) = sessions();

        let error = connect(
            &store,
            &Credentials::in_memory_as_os_store(),
            &sessions,
            ConnectionId::new(),
            None,
        )
        .await
        .expect_err("unknown");

        assert_eq!(error.code(), ErrorCode::NotFound);
    }
}
