//! A fake database engine for service tests: it connects instantly, records what it was asked,
//! and serves a scriptable catalog.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use super::sessions::Sessions;
use crate::adapters::database::{
    ApplyError, ConnectTarget, DatabaseAdapter, DatabaseSession, DbError, DbErrorKind, ExportError,
};
use crate::domain::catalog::{
    CatalogOverview, ObjectDetails, ObjectKind, ObjectName, ObjectSummary, SchemaSummary,
};
use crate::domain::connections::{ConnectionDraft, Engine};
use crate::domain::data_change::{DataChange, DataChangePlan};
use crate::domain::database::{Capabilities, OperationId, ServerInfo, SessionLimits};
use crate::domain::export::{ExportSummary, RowSink};
use crate::domain::policy::facts::TargetTable;
use crate::domain::query::QueryRows;
use crate::domain::settings::SafetySettings;
use crate::domain::sql::ReadShape;
use crate::domain::table_data::{PageColumn, PageLimits, PageRequest, TablePage, ValueKind};

/// The fake database's structure, shared by every session the adapter opens.
#[derive(Debug, Default)]
pub struct FakeCatalog {
    /// Change it to simulate DDL.
    pub fingerprint: Mutex<String>,
    pub fingerprint_checks: AtomicUsize,
    pub overview_loads: AtomicUsize,
    pub describe_loads: AtomicUsize,
    /// The last page read and the limits it was read with.
    pub last_page: Mutex<Option<(PageRequest, PageLimits)>>,
    /// The planner's estimate for any write; `None` fails the estimate.
    pub write_estimate: Mutex<Option<u64>>,
    /// What `write_targets` reports for a target; others exist with every privilege.
    pub targets: Mutex<Vec<TargetTable>>,
    /// Rows an UPDATE or DELETE matches; change it to simulate other writers.
    pub matched_rows: Mutex<u64>,
    /// Changes applied, in order.
    pub applied: Mutex<Vec<DataChange>>,
    /// Makes the next apply fail: rolled back, or uncertain when `true`.
    pub apply_failure: Mutex<Option<bool>>,
}

impl FakeCatalog {
    pub fn set_matched_rows(&self, rows: u64) {
        *self
            .matched_rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = rows;
    }

    fn matched_rows(&self) -> u64 {
        *self
            .matched_rows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
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

/// The plan the fake writes for a change: plain SQL for the policy to read, the values as
/// given, and [`FakeCatalog::matched_rows`] rows for an UPDATE or DELETE.
fn fake_plan(change: &DataChange, matched: u64) -> DataChangePlan {
    let table = change.table();
    let name = format!("{}.{}", table.schema, table.name);
    let predicate = |offset: usize| {
        change
            .filters()
            .iter()
            .enumerate()
            .map(|(index, filter)| format!("{} = ${}", filter.column, offset + index + 1))
            .collect::<Vec<_>>()
            .join(" AND ")
    };
    let filter_values = change.filters().iter().map(|f| f.value.clone());
    match change {
        DataChange::Insert { columns, rows, .. } => DataChangePlan {
            statement: format!(
                "INSERT INTO {name} ({}) VALUES {}",
                columns.join(", "),
                (0..rows.len())
                    .map(|row| format!(
                        "({})",
                        (0..columns.len())
                            .map(|c| format!("${}", row * columns.len() + c + 1))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            parameters: rows.iter().flatten().cloned().collect(),
            rows: rows.len() as u64,
            value_columns: columns.clone(),
            values: rows.clone(),
            before: None,
        },
        DataChange::Update { set, .. } => DataChangePlan {
            statement: format!(
                "UPDATE {name} SET {} WHERE {}",
                set.iter()
                    .enumerate()
                    .map(|(index, value)| format!("{} = ${}", value.column, index + 1))
                    .collect::<Vec<_>>()
                    .join(", "),
                predicate(set.len())
            ),
            parameters: set
                .iter()
                .map(|value| value.value.clone())
                .chain(filter_values)
                .collect(),
            rows: matched,
            value_columns: set.iter().map(|value| value.column.clone()).collect(),
            values: vec![set.iter().map(|value| value.value.clone()).collect()],
            before: None,
        },
        DataChange::Delete { .. } => DataChangePlan {
            statement: format!("DELETE FROM {name} WHERE {}", predicate(0)),
            parameters: filter_values.collect(),
            rows: matched,
            value_columns: Vec::new(),
            values: Vec::new(),
            before: None,
        },
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

    async fn read_page(
        &self,
        request: &PageRequest,
        limits: PageLimits,
    ) -> Result<TablePage, DbError> {
        *self
            .catalog
            .last_page
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some((request.clone(), limits));
        if request.object.name == "ghost" {
            return Err(DbError::new(DbErrorKind::ObjectNotFound, "No such table."));
        }
        Ok(TablePage {
            object: request.object.clone(),
            columns: Vec::new(),
            omitted_columns: Vec::new(),
            rows: Vec::new(),
            truncated_cells: Vec::new(),
            offset: request.offset,
            has_more: false,
            byte_limit_reached: false,
            order: request.sort.clone(),
            stable_order: true,
            cell_char_limit: crate::domain::table_data::CELL_CHAR_LIMIT,
        })
    }

    /// Writes up to three rows of `id, name`, stopping at `max_rows`; fails for `ghost`.
    async fn export_rows(
        &self,
        request: &PageRequest,
        max_rows: u32,
        sink: &mut (dyn RowSink + Send),
    ) -> Result<ExportSummary, ExportError> {
        if request.object.name == "ghost" {
            return Err(ExportError::Database(DbError::new(
                DbErrorKind::ObjectNotFound,
                "No such table.",
            )));
        }
        let column = |name: &str, kind| PageColumn {
            name: name.into(),
            data_type: String::new(),
            kind,
        };
        sink.columns(&[
            column("id", ValueKind::Number),
            column("name", ValueKind::Text),
        ])
        .map_err(ExportError::Write)?;
        let available = 3_u32;
        let rows = available.min(max_rows);
        for id in 1..=rows {
            sink.row(&[Some(id.to_string()), Some(format!("row {id}"))])
                .map_err(ExportError::Write)?;
        }
        Ok(ExportSummary {
            rows: u64::from(rows),
            truncated: available > max_rows,
        })
    }

    /// One row with the SQL's length; SQL containing `boom` fails at character 8.
    async fn run_read(
        &self,
        sql: &str,
        _shape: ReadShape,
        _limits: PageLimits,
        _offset: u64,
        _operation: Option<OperationId>,
    ) -> Result<QueryRows, DbError> {
        if sql.contains("boom") {
            return Err(DbError::new(
                DbErrorKind::InvalidRequest,
                "column \"boom\" does not exist.",
            )
            .with_sqlstate("42703")
            .with_position(Some(8)));
        }
        Ok(QueryRows {
            columns: vec![PageColumn {
                name: "length".into(),
                data_type: "integer".into(),
                kind: ValueKind::Number,
            }],
            rows: vec![vec![Some(sql.len().to_string())]],
            truncated_cells: Vec::new(),
            has_more: false,
            byte_limit_reached: false,
        })
    }

    async fn estimate_written_rows(&self, _sql: &str) -> Result<u64, DbError> {
        let estimate = *self
            .catalog
            .write_estimate
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        estimate.ok_or_else(|| DbError::new(DbErrorKind::Other, "no estimate"))
    }

    async fn write_targets(&self, targets: &[String]) -> Result<Vec<TargetTable>, DbError> {
        let known = self
            .catalog
            .targets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Ok(targets
            .iter()
            .map(|target| {
                known
                    .iter()
                    .find(|table| &table.target == target)
                    .cloned()
                    .unwrap_or_else(|| TargetTable {
                        target: target.clone(),
                        resolved: Some(format!("public.{target}")),
                        can_insert: true,
                        can_update: true,
                        can_delete: true,
                        can_truncate: true,
                        cascades_to: Vec::new(),
                    })
            })
            .collect())
    }

    /// Refuses the table `ghost`.
    async fn plan_data_change(
        &self,
        change: &DataChange,
        preview: Option<PageLimits>,
    ) -> Result<DataChangePlan, DbError> {
        if change.table().name == "ghost" {
            return Err(DbError::new(DbErrorKind::ObjectNotFound, "No such table."));
        }
        let mut plan = fake_plan(change, self.catalog.matched_rows());
        if preview.is_some() && !matches!(change, DataChange::Insert { .. }) {
            plan.before = Some(Self::before_page(change));
        }
        Ok(plan)
    }

    /// Applies only when the plan still matches, like the real engine.
    async fn apply_data_change(
        &self,
        change: &DataChange,
        approved: &DataChangePlan,
    ) -> Result<u64, ApplyError> {
        let failure = self
            .catalog
            .apply_failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        match failure {
            Some(true) => {
                return Err(ApplyError::Uncertain(DbError::new(
                    DbErrorKind::ConnectionLost,
                    "The connection to the database was lost.",
                )));
            }
            Some(false) => {
                return Err(ApplyError::RolledBack(DbError::new(
                    DbErrorKind::InvalidRequest,
                    "duplicate key value violates unique constraint \"orders_pkey\".",
                )));
            }
            None => {}
        }
        let unchanged = DataChangePlan {
            before: None,
            ..approved.clone()
        };
        if fake_plan(change, self.catalog.matched_rows()) != unchanged {
            return Err(ApplyError::RolledBack(DbError::new(
                DbErrorKind::InvalidRequest,
                "The rows changed since the change was approved.",
            )));
        }
        self.catalog
            .applied
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(change.clone());
        Ok(approved.rows)
    }
}

impl FakeSession {
    /// A page of the matching rows, as the preview reads them.
    fn before_page(change: &DataChange) -> TablePage {
        TablePage {
            object: change.table().clone(),
            columns: Vec::new(),
            omitted_columns: Vec::new(),
            rows: Vec::new(),
            truncated_cells: Vec::new(),
            offset: 0,
            has_more: false,
            byte_limit_reached: false,
            order: Vec::new(),
            stable_order: true,
            cell_char_limit: crate::domain::table_data::CELL_CHAR_LIMIT,
        }
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
