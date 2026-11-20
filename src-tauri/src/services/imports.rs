//! CSV imports: a file is read in the core and held here while the user maps its columns; the
//! import is then checked and planned as batches of inserts through the data change engine, and
//! held for approval like any other change. Applying it is `data_changes::apply`: one
//! transaction, all rows or none.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::approvals::Approvals;
use super::data_changes::{self, PreviewOptions};
use super::sessions::{Sessions, not_connected};
use crate::adapters::database::{DatabaseAdapter, DatabaseSession};
use crate::adapters::local_store::LocalStore;
use crate::domain::audit::Actor;
use crate::domain::catalog::{ObjectDetails, ObjectName};
use crate::domain::connections::ConnectionId;
use crate::domain::csv_import::{
    CsvImportDraft, CsvImportPreview, CsvImportReady, CsvImportRequest, CsvOptions, ParsedCsv,
    detect, import_changes, import_problems, parse, sample, suggest_mapping,
};
use crate::domain::data_change::{DataChange, IMPORT_LIMITS};
use crate::domain::table_data::truncate_cell;
use crate::errors::{AppError, AppResult, ErrorCode};

/// Files held at a time; the oldest goes first.
const MAX_HELD: usize = 4;

/// A file read for import into one table.
#[derive(Debug)]
struct Held {
    connection_id: ConnectionId,
    table: ObjectName,
    file_name: String,
    bytes: Arc<Vec<u8>>,
    /// The table's column names, for detection and suggestions.
    table_columns: Vec<String>,
    parsed: Arc<ParsedCsv>,
    /// Order of arrival, to drop the oldest.
    serial: u64,
}

/// Files being imported, in memory only. They never outlive the app.
#[derive(Debug, Default)]
pub struct Imports {
    held: Mutex<HashMap<uuid::Uuid, Held>>,
    serial: std::sync::atomic::AtomicU64,
}

impl Imports {
    fn lock(&self) -> MutexGuard<'_, HashMap<uuid::Uuid, Held>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Drops every file read for a connection, when it closes or changes.
    pub fn forget_connection(&self, connection_id: ConnectionId) {
        self.lock()
            .retain(|_, held| held.connection_id != connection_id);
    }
}

fn no_such_import() -> AppError {
    AppError::not_found("That file is no longer open for import. Choose it again.")
}

fn draft(id: uuid::Uuid, held: &Held) -> CsvImportDraft {
    CsvImportDraft {
        import_id: id,
        file_name: held.file_name.clone(),
        options: held.parsed.options.clone(),
        columns: held.parsed.columns.clone(),
        suggested: suggest_mapping(&held.parsed, &held.table_columns),
        sample: sample(&held.parsed),
        total_rows: held.parsed.rows.len(),
        warnings: held.parsed.warnings.clone(),
    }
}

async fn table_details<A: DatabaseAdapter>(
    sessions: &Sessions<A>,
    connection_id: ConnectionId,
    table: &ObjectName,
) -> AppResult<ObjectDetails> {
    let session = sessions.get(connection_id).ok_or_else(not_connected)?;
    session.describe_object(table).await.map_err(AppError::from)
}

/// Reads a file for import into `table`, detecting how it is written.
///
/// # Errors
///
/// `invalid_input` when the file is empty, malformed or over the limits; `not_found` for an
/// unknown table; `unavailable` when not connected.
pub async fn open<A: DatabaseAdapter>(
    sessions: &Sessions<A>,
    imports: &Imports,
    connection_id: ConnectionId,
    table: ObjectName,
    file_name: String,
    bytes: Vec<u8>,
) -> AppResult<CsvImportDraft> {
    let details = table_details(sessions, connection_id, &table).await?;
    let table_columns: Vec<String> = details.columns.iter().map(|c| c.name.clone()).collect();
    let options = detect(&bytes, &table_columns)?;
    let parsed = parse(&bytes, options)?;
    let id = uuid::Uuid::new_v4();
    let held = Held {
        connection_id,
        table,
        file_name,
        bytes: Arc::new(bytes),
        table_columns,
        parsed: Arc::new(parsed),
        serial: imports
            .serial
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    };
    let result = draft(id, &held);
    let mut files = imports.lock();
    while files.len() >= MAX_HELD {
        let oldest = files
            .iter()
            .min_by_key(|(_, held)| held.serial)
            .map(|(id, _)| *id);
        match oldest {
            Some(oldest) => {
                files.remove(&oldest);
            }
            None => break,
        }
    }
    files.insert(id, held);
    Ok(result)
}

/// Reads a held file again with other options (encoding, delimiter, header).
///
/// # Errors
///
/// `not_found` when the file is no longer held; `invalid_input` when it cannot be read so.
pub fn reread(imports: &Imports, id: uuid::Uuid, options: CsvOptions) -> AppResult<CsvImportDraft> {
    let bytes = imports
        .lock()
        .get(&id)
        .map(|held| Arc::clone(&held.bytes))
        .ok_or_else(no_such_import)?;
    let parsed = parse(&bytes, options)?;
    let mut files = imports.lock();
    let held = files.get_mut(&id).ok_or_else(no_such_import)?;
    held.parsed = Arc::new(parsed);
    Ok(draft(id, held))
}

/// Forgets a held file.
pub fn close(imports: &Imports, id: uuid::Uuid) {
    imports.lock().remove(&id);
}

/// The longest statement shown in a preview, in characters.
const SHOWN_STATEMENT_CHARS: usize = 2_000;

/// Checks the mapping, then plans the import against the live table and holds it for approval.
/// A value the database refuses is traced to its row.
///
/// # Errors
///
/// `not_found` when the file is no longer held or belongs to another connection;
/// `permission_denied` when the policy refuses the import here; `unavailable` when not
/// connected.
pub async fn preview<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    approvals: &Approvals,
    imports: &Imports,
    connection_id: ConnectionId,
    id: uuid::Uuid,
    request: CsvImportRequest,
) -> AppResult<CsvImportPreview> {
    let (table, parsed) = {
        let files = imports.lock();
        let held = files
            .get(&id)
            .filter(|held| held.connection_id == connection_id)
            .ok_or_else(no_such_import)?;
        (held.table.clone(), Arc::clone(&held.parsed))
    };
    let details = table_details(sessions, connection_id, &table).await?;
    let problems = import_problems(&parsed, &details, &request);
    if !problems.is_empty() {
        return Ok(CsvImportPreview {
            problems,
            ready: None,
        });
    }
    let (changes, starts) = import_changes(&parsed, &table, &request);
    let batches = changes.len();
    let rows = parsed.rows.len();
    let options = PreviewOptions {
        limits: IMPORT_LIMITS,
        listed: false,
    };
    let planned = data_changes::preview_with(
        store,
        sessions,
        approvals,
        connection_id,
        changes.clone(),
        Actor::User,
        options,
    )
    .await;
    let preview = match planned {
        Ok(preview) => preview,
        Err(error) if error.code() == ErrorCode::InvalidInput => {
            let problem = locate(sessions, connection_id, &changes, &starts, &error).await;
            return Ok(CsvImportPreview {
                problems: vec![problem],
                ready: None,
            });
        }
        Err(error) => return Err(error),
    };
    let first = preview.changes.first();
    let mut statement = first.map(|c| c.statement.clone()).unwrap_or_default();
    let cut = truncate_cell(&mut statement) || statement.chars().count() > SHOWN_STATEMENT_CHARS;
    let mut statement: String = statement.chars().take(SHOWN_STATEMENT_CHARS).collect();
    if cut {
        statement.push('…');
    }
    Ok(CsvImportPreview {
        problems: Vec::new(),
        ready: Some(CsvImportReady {
            approval_id: preview.approval_id,
            rows: rows as u64,
            batches,
            after: first
                .and_then(|c| c.after.clone())
                .unwrap_or_else(|| crate::domain::data_change::ValueSample::of(&[], &[])),
            statement,
            risk: preview.risk,
            approval: preview.approval,
            reasons: preview.reasons,
            confirmation: preview.confirmation,
            expires_at: preview.expires_at,
        }),
    })
}

/// The row a planning error comes from: the batch it names, narrowed by planning halves of it
/// until one row fails. Planning only reads, so this writes nothing.
async fn locate<A: DatabaseAdapter>(
    sessions: &Sessions<A>,
    connection_id: ConnectionId,
    changes: &[DataChange],
    starts: &[usize],
    error: &AppError,
) -> String {
    let message = error.message().into_owned();
    let (index, reason) = match message.strip_prefix("Change ") {
        Some(rest) => {
            let index = rest
                .split_once(' ')
                .and_then(|(number, _)| number.parse::<usize>().ok())
                .map_or(0, |number| number.saturating_sub(1));
            let reason = rest
                .split_once(": ")
                .map_or(message.as_str(), |(_, reason)| reason)
                .to_owned();
            (index, reason)
        }
        None => (0, message.clone()),
    };
    let (
        Some(DataChange::Insert {
            table,
            columns,
            rows,
            upsert_key,
        }),
        Some(start),
    ) = (changes.get(index), starts.get(index))
    else {
        return reason;
    };
    let Some(session) = sessions.get(connection_id) else {
        return reason;
    };
    let fails = |part: &[Vec<Option<String>>]| {
        let change = DataChange::Insert {
            table: table.clone(),
            columns: columns.clone(),
            rows: part.to_vec(),
            upsert_key: upsert_key.clone(),
        };
        let session = Arc::clone(&session);
        async move {
            session
                .plan_data_changes(&[change], None, IMPORT_LIMITS.max_rows)
                .await
                .err()
                .map(|error| error.message().to_owned())
        }
    };
    // [low, high) holds a failing row; keep the half that fails, the first half first.
    let (mut low, mut high) = (0, rows.len());
    while high - low > 1 {
        let middle = low + (high - low) / 2;
        if fails(&rows[low..middle]).await.is_some() {
            high = middle;
        } else {
            low = middle;
        }
    }
    let reason = fails(&rows[low..high]).await.unwrap_or(reason);
    format!("Row {}: {reason}", start + low + 1)
}

#[cfg(test)]
mod tests;
