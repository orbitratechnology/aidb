//! A fake database engine for service tests: it connects instantly, records what it was asked,
//! and serves a scriptable catalog.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use super::sessions::Sessions;
use crate::adapters::database::{
    ConnectTarget, DatabaseAdapter, DatabaseSession, DbError, DbErrorKind,
};
use crate::domain::catalog::{
    CatalogOverview, ObjectDetails, ObjectKind, ObjectName, ObjectSummary, SchemaSummary,
};
use crate::domain::connections::{ConnectionDraft, Engine};
use crate::domain::database::{Capabilities, OperationId, ServerInfo, SessionLimits};
use crate::domain::settings::SafetySettings;

/// The fake database's structure, shared by every session the adapter opens.
#[derive(Debug, Default)]
pub struct FakeCatalog {
    /// Change it to simulate DDL.
    pub fingerprint: Mutex<String>,
    pub fingerprint_checks: AtomicUsize,
    pub overview_loads: AtomicUsize,
    pub describe_loads: AtomicUsize,
}

impl FakeCatalog {
    pub fn set_fingerprint(&self, fingerprint: &str) {
        *self
            .fingerprint
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = fingerprint.to_owned();
    }

    fn fingerprint(&self) -> String {
        self.fingerprint
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// Records what it was asked to connect with; fails for the host `unreachable`.
#[derive(Debug, Default)]
pub struct FakeAdapter {
    pub connects: AtomicUsize,
    pub last_target: Mutex<Option<ConnectTarget>>,
    pub catalog: Arc<FakeCatalog>,
}

#[derive(Debug)]
pub struct FakeSession {
    info: ServerInfo,
    pub closed: Arc<AtomicBool>,
    catalog: Arc<FakeCatalog>,
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
        *self
            .last_target
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(target);
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
            catalog: Arc::clone(&self.catalog),
        })
    }
}

fn summary(name: &str) -> ObjectSummary {
    ObjectSummary {
        schema: "public".into(),
        name: name.into(),
        kind: ObjectKind::Table,
        row_estimate: Some(10),
        comment: None,
        readable: true,
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

    async fn catalog_fingerprint(&self) -> Result<String, DbError> {
        self.catalog
            .fingerprint_checks
            .fetch_add(1, Ordering::SeqCst);
        Ok(self.catalog.fingerprint())
    }

    async fn catalog_overview(&self) -> Result<CatalogOverview, DbError> {
        self.catalog.overview_loads.fetch_add(1, Ordering::SeqCst);
        Ok(CatalogOverview {
            database: "orders".into(),
            fingerprint: self.catalog.fingerprint(),
            schemas: vec![SchemaSummary {
                name: "public".into(),
                comment: None,
                accessible: true,
            }],
            objects: vec![summary("customers"), summary("orders")],
            truncated: false,
        })
    }

    async fn describe_object(&self, object: &ObjectName) -> Result<ObjectDetails, DbError> {
        self.catalog.describe_loads.fetch_add(1, Ordering::SeqCst);
        if object.schema != "public" || !["customers", "orders"].contains(&object.name.as_str()) {
            return Err(DbError::new(DbErrorKind::ObjectNotFound, "No such table."));
        }
        Ok(ObjectDetails {
            summary: summary(&object.name),
            columns: Vec::new(),
            primary_key: None,
            unique_constraints: Vec::new(),
            foreign_keys: Vec::new(),
            referenced_by: Vec::new(),
            indexes: Vec::new(),
            definition: None,
        })
    }
}

pub fn sessions() -> (Arc<FakeAdapter>, Sessions<Arc<FakeAdapter>>) {
    let adapter = Arc::new(FakeAdapter::default());
    (Arc::clone(&adapter), Sessions::new(adapter))
}

pub fn target(host: &str) -> ConnectTarget {
    let settings = ConnectionDraft::new("Orders", host, "orders", "reader");
    ConnectTarget {
        limits: SessionLimits::new(&settings, &SafetySettings::default()),
        settings,
        password: None,
    }
}
