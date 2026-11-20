//! Schema comparison: reads the target schema on the open connection and the source (another
//! schema there, a schema on another saved connection, or a `.sql` file the core holds), and
//! keeps the resulting migration in the core under its id, for saving and for applying later.
//! Nothing runs on any database but catalog reads.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use secrecy::SecretString;

use super::sessions::{Sessions, not_connected, read_schema_elsewhere};
use crate::adapters::credentials::Credentials;
use crate::adapters::database::{DatabaseAdapter, DatabaseSession};
use crate::adapters::local_store::LocalStore;
use crate::domain::connections::ConnectionId;
use crate::domain::schema_diff::generate::plan;
use crate::domain::schema_diff::{
    CompareRequest, CompareSource, SchemaDiff, SchemaFile, SchemaModel, parse,
};
use crate::errors::{AppError, AppResult};

/// Files and comparisons held at a time; the oldest goes first.
const MAX_FILES: usize = 4;
const MAX_DIFFS: usize = 8;

/// PostgreSQL's limit on a name, in bytes.
const MAX_NAME_BYTES: usize = 63;

#[derive(Debug)]
struct HeldFile {
    file_name: String,
    text: Arc<String>,
    serial: u64,
}

/// A comparison kept for later: the migration and the connection it was made for.
#[derive(Debug, Clone)]
pub struct HeldDiff {
    pub connection_id: ConnectionId,
    pub diff: Arc<SchemaDiff>,
    serial: u64,
}

/// `.sql` files read for comparison and the comparisons made, in memory only.
#[derive(Debug, Default)]
pub struct SchemaDiffs {
    files: Mutex<HashMap<uuid::Uuid, HeldFile>>,
    diffs: Mutex<HashMap<uuid::Uuid, HeldDiff>>,
    serial: AtomicU64,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Removes the entry with the lowest serial while `map` holds `max` or more.
fn make_room<V>(map: &mut HashMap<uuid::Uuid, V>, max: usize, serial: impl Fn(&V) -> u64) {
    while map.len() >= max {
        let Some(oldest) = map
            .iter()
            .min_by_key(|(_, value)| serial(value))
            .map(|(id, _)| *id)
        else {
            break;
        };
        map.remove(&oldest);
    }
}

impl SchemaDiffs {
    fn next(&self) -> u64 {
        self.serial.fetch_add(1, Ordering::Relaxed)
    }

    /// Drops the comparisons made on a connection, when it closes or changes: they describe a
    /// schema that may no longer be what it was.
    pub fn forget_connection(&self, connection_id: ConnectionId) {
        lock(&self.diffs).retain(|_, held| held.connection_id != connection_id);
    }

    /// A comparison made earlier, if it is still held.
    #[must_use]
    pub fn get(&self, id: uuid::Uuid) -> Option<HeldDiff> {
        lock(&self.diffs).get(&id).cloned()
    }
}

/// Reads a `.sql` file for comparison and holds it.
///
/// # Errors
///
/// `invalid_input` when the file is not UTF-8 text or cannot be split into statements.
pub fn open_file(diffs: &SchemaDiffs, file_name: String, bytes: Vec<u8>) -> AppResult<SchemaFile> {
    let text = String::from_utf8(bytes).map_err(|_| {
        AppError::invalid_input("The file is not UTF-8 text. Save it as UTF-8 and choose it again.")
    })?;
    let text = text
        .strip_prefix('\u{feff}')
        .map(str::to_owned)
        .unwrap_or(text);
    let (schemas, statements) =
        parse::survey(&text).map_err(|error| AppError::invalid_input(error.to_string()))?;
    if statements == 0 {
        return Err(AppError::invalid_input("The file has no SQL statements."));
    }
    let file_id = uuid::Uuid::new_v4();
    let held = HeldFile {
        file_name: file_name.clone(),
        text: Arc::new(text),
        serial: diffs.next(),
    };
    let mut files = lock(&diffs.files);
    make_room(&mut files, MAX_FILES, |file| file.serial);
    files.insert(file_id, held);
    Ok(SchemaFile {
        file_id,
        file_name,
        schemas,
        statements,
    })
}

/// Forgets a held file.
pub fn close_file(diffs: &SchemaDiffs, id: uuid::Uuid) {
    lock(&diffs.files).remove(&id);
}

fn check_name(schema: &str) -> AppResult<()> {
    if schema.trim().is_empty() || schema.len() > MAX_NAME_BYTES || schema.contains('\0') {
        return Err(AppError::invalid_input("Name a schema of 1 to 63 bytes."));
    }
    Ok(())
}

/// Compares `request.target_schema` on the open connection with the request's source and holds
/// the migration that makes the target match. `password` is for a source on another connection
/// whose password is not saved; it is used once.
///
/// # Errors
///
/// `unavailable` when the connection is not open; `not_found` for an unknown schema, file or
/// connection; `invalid_input` for a bad request or file; the database's error otherwise.
pub async fn compare<A: DatabaseAdapter>(
    store: &LocalStore,
    credentials: &Credentials,
    sessions: &Sessions<A>,
    diffs: &SchemaDiffs,
    connection_id: ConnectionId,
    request: CompareRequest,
    password: Option<SecretString>,
) -> AppResult<SchemaDiff> {
    check_name(&request.target_schema)?;
    let session = sessions.get(connection_id).ok_or_else(not_connected)?;
    let profile = store
        .connections()
        .get(connection_id)
        .await?
        .ok_or_else(|| AppError::not_found("The connection no longer exists."))?;
    let here = profile.settings.name;
    let target = session.read_schema(&request.target_schema).await?;

    let (source, label): (SchemaModel, String) = match &request.source {
        CompareSource::Schema { schema } => {
            check_name(schema)?;
            same_schema(schema, &request.target_schema)?;
            (
                session.read_schema(schema).await?,
                format!("{schema} on {here}"),
            )
        }
        CompareSource::Connection {
            connection_id: other,
            schema,
        } => {
            check_name(schema)?;
            if *other == connection_id {
                same_schema(schema, &request.target_schema)?;
                (
                    session.read_schema(schema).await?,
                    format!("{schema} on {here}"),
                )
            } else {
                let (name, model) =
                    read_schema_elsewhere(store, credentials, sessions, *other, password, schema)
                        .await?;
                (model, format!("{schema} on {name}"))
            }
        }
        CompareSource::File { file_id, schema } => {
            check_name(schema)?;
            let (file_name, text) = {
                let files = lock(&diffs.files);
                let file = files.get(file_id).ok_or_else(|| {
                    AppError::not_found("That file is no longer open. Choose it again.")
                })?;
                (file.file_name.clone(), Arc::clone(&file.text))
            };
            let model = parse::read(&text, schema)
                .map_err(|error| AppError::invalid_input(error.to_string()))?;
            (model, format!("{file_name} ({schema})"))
        }
    };

    let planned = plan(
        &source,
        &target,
        request.options,
        profile.settings.environment,
    );
    let mut notes = planned.notes;
    if !source.not_compared.is_empty() {
        notes.push(format!(
            "Not compared in {label}: {}.",
            source.not_compared.join(", ")
        ));
    }
    if !target.not_compared.is_empty() {
        notes.push(format!(
            "Not compared in {} on {here}: {}.",
            request.target_schema,
            target.not_compared.join(", ")
        ));
    }
    if !source.stored_text && !source.views.is_empty() {
        notes.push(
            "PostgreSQL rewrites a view's query when it stores it, so views in a file are only \
             added or removed, never compared with the database's."
                .to_owned(),
        );
    }
    let diff = SchemaDiff {
        diff_id: uuid::Uuid::new_v4(),
        source: label,
        target: format!("{} on {here}", request.target_schema),
        target_schema: request.target_schema,
        summary: planned.summary,
        steps: planned.steps,
        script: planned.script,
        notes,
        risk: planned.risk,
    };
    let mut held = lock(&diffs.diffs);
    make_room(&mut held, MAX_DIFFS, |held| held.serial);
    held.insert(
        diff.diff_id,
        HeldDiff {
            connection_id,
            diff: Arc::new(diff.clone()),
            serial: diffs.next(),
        },
    );
    Ok(diff)
}

fn same_schema(source: &str, target: &str) -> AppResult<()> {
    if source == target {
        return Err(AppError::invalid_input(
            "Choose another schema to compare with: this is the schema being changed.",
        ));
    }
    Ok(())
}

/// A held comparison's script and a file name for it.
///
/// # Errors
///
/// `not_found` when the comparison is no longer held; `invalid_input` when it has nothing to do.
pub fn script(diffs: &SchemaDiffs, id: uuid::Uuid) -> AppResult<(String, String)> {
    let held = diffs
        .get(id)
        .ok_or_else(|| AppError::not_found("That comparison is no longer open. Compare again."))?;
    if held.diff.script.is_empty() {
        return Err(AppError::invalid_input(
            "The schemas match: there is nothing to save.",
        ));
    }
    let name: String = held
        .diff
        .target_schema
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    Ok((format!("migrate-{name}.sql"), held.diff.script.clone()))
}

#[cfg(test)]
mod tests;
