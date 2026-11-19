//! Saving, listing, deleting, exporting and importing connection profiles.
//!
//! Every profile is validated here before it is stored, whatever its source (form, URI or file).
//! Imported profiles always start read-only, and a profile that would send the password over the
//! network unencrypted is saved only with the user's explicit consent.

use std::collections::HashSet;

use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::{audit, connection_secrets};
use crate::adapters::credentials::{Credentials, SecretRef};
use crate::adapters::local_store::{LocalStore, StoreError};
use crate::domain::audit::{Actor, AuditEvent, AuditStatus};
use crate::domain::connection_transfer::{self, FileEntry, MAX_ENTRIES};
use crate::domain::connections::{
    ConnectionDraft, ConnectionId, ConnectionProfile, Engine, Environment,
};
use crate::errors::{AppError, AppResult, ErrorCode};

/// Where a connection's password is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum PasswordStorage {
    /// In the OS credential store; survives restarts.
    OsStore,
    /// In memory until aidb quits.
    SessionOnly,
    NotSaved,
}

/// A saved profile as the UI sees it. Never includes the password or its store reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ConnectionDetails {
    pub id: ConnectionId,
    pub engine: Engine,
    #[serde(flatten)]
    pub settings: ConnectionDraft,
    pub password: PasswordStorage,
    /// TLS is off and the host is another machine.
    pub unencrypted_remote: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl ConnectionDetails {
    fn new(profile: ConnectionProfile, credentials: &Credentials) -> Self {
        let password = if profile.secret_ref.is_some() {
            PasswordStorage::OsStore
        } else if credentials.holds_in_session(&SecretRef::connection(profile.id)) {
            PasswordStorage::SessionOnly
        } else {
            PasswordStorage::NotSaved
        };
        Self {
            id: profile.id,
            engine: profile.engine,
            unencrypted_remote: profile.settings.is_unencrypted_remote(),
            settings: profile.settings,
            password,
            created_at: profile.created_at,
            updated_at: profile.updated_at,
        }
    }
}

/// Every saved profile, ordered by name.
///
/// # Errors
///
/// `storage` if the local database cannot be read.
pub async fn list(
    store: &LocalStore,
    credentials: &Credentials,
) -> AppResult<Vec<ConnectionDetails>> {
    Ok(store
        .connections()
        .list()
        .await?
        .into_iter()
        .map(|profile| ConnectionDetails::new(profile, credentials))
        .collect())
}

/// Validates and saves a new profile.
///
/// # Errors
///
/// `invalid_input` for a bad field or an unconfirmed unencrypted remote connection, `conflict`
/// if the name is taken, `storage` if saving fails.
pub async fn create(
    store: &LocalStore,
    credentials: &Credentials,
    draft: ConnectionDraft,
    unencrypted_allowed: bool,
) -> AppResult<ConnectionDetails> {
    let draft = draft.validated()?;
    draft.check_transport(unencrypted_allowed)?;
    let profile = store
        .connections()
        .insert(&draft)
        .await
        .map_err(|error| profile_error(error, &draft.name))?;
    tracing::info!(connection = %profile.id, environment = draft.environment.as_str(), "connection created");
    record(
        store,
        "connection_created",
        profile.id,
        draft.environment,
        access(draft.read_only),
    )
    .await;
    Ok(ConnectionDetails::new(profile, credentials))
}

fn access(read_only: bool) -> String {
    if read_only {
        "Read-only.".to_owned()
    } else {
        "Writes allowed.".to_owned()
    }
}

/// Records a profile change: its environment and access mode only, never its address, user or
/// other settings.
async fn record(
    store: &LocalStore,
    operation: &'static str,
    id: ConnectionId,
    environment: Environment,
    detail: String,
) {
    audit::record(
        store,
        &AuditEvent {
            connection_id: Some(id),
            environment: Some(environment),
            actor: Actor::User,
            operation,
            risk: None,
            fingerprint: None,
            approval_state: None,
            status: AuditStatus::Succeeded,
            rows_affected: None,
            detail: Some(detail),
        },
    )
    .await;
}

/// Validates and saves changes to an existing profile.
///
/// # Errors
///
/// As [`create`], plus `not_found` if the profile no longer exists.
pub async fn update(
    store: &LocalStore,
    credentials: &Credentials,
    id: ConnectionId,
    draft: ConnectionDraft,
    unencrypted_allowed: bool,
) -> AppResult<ConnectionDetails> {
    let draft = draft.validated()?;
    draft.check_transport(unencrypted_allowed)?;
    let before = store.connections().get(id).await?;
    let profile = store
        .connections()
        .update(id, &draft)
        .await
        .map_err(|error| profile_error(error, &draft.name))?;
    tracing::info!(connection = %id, read_only = draft.read_only, "connection updated");
    let mut changes = Vec::new();
    if let Some(before) = &before {
        if before.settings.environment != draft.environment {
            changes.push(format!(
                "Environment changed from {} to {}.",
                before.settings.environment.as_str(),
                draft.environment.as_str()
            ));
        }
        if before.settings.read_only != draft.read_only {
            changes.push(if draft.read_only {
                "Writes turned off.".to_owned()
            } else {
                "Writes turned on.".to_owned()
            });
        }
    }
    if changes.is_empty() {
        changes.push("Settings changed.".to_owned());
    }
    record(
        store,
        "connection_updated",
        id,
        draft.environment,
        changes.join(" "),
    )
    .await;
    Ok(ConnectionDetails::new(profile, credentials))
}

/// Deletes a profile, its cached schema details and, if asked, its saved password. Saved queries
/// and history stay, detached from the connection.
///
/// The password is removed first, so a failure leaves the profile in place to retry.
///
/// # Errors
///
/// `not_found` if the profile does not exist, `credential_store` or `storage` on failure.
pub async fn delete(
    store: &LocalStore,
    credentials: &Credentials,
    id: ConnectionId,
    forget_password: bool,
) -> AppResult<()> {
    let Some(profile) = store.connections().get(id).await? else {
        return Err(missing());
    };
    if forget_password {
        connection_secrets::forget_password(store, credentials, id).await?;
    }
    if !store.connections().delete(id).await? {
        return Err(missing());
    }
    tracing::info!(connection = %id, forget_password, "connection deleted");
    record(
        store,
        "connection_deleted",
        id,
        profile.settings.environment,
        if forget_password {
            "Removed with its saved password.".to_owned()
        } else {
            "Removed.".to_owned()
        },
    )
    .await;
    Ok(())
}

/// Every profile as an export file, and how many it holds. Passwords are never included.
///
/// # Errors
///
/// `storage` if the local database cannot be read.
pub async fn export(store: &LocalStore) -> AppResult<(Vec<u8>, u32)> {
    let profiles = store.connections().list().await?;
    let exported_at = OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .ok()
        .and_then(|now| now.format(&Rfc3339).ok())
        .unwrap_or_default();
    let bytes = connection_transfer::render(profiles.iter().map(|p| &p.settings), &exported_at);
    Ok((bytes, count(profiles.len())))
}

/// Default name for an export file, such as `aidb-connections-2026-10-26.json`.
#[must_use]
pub fn export_file_name() -> String {
    let today = OffsetDateTime::now_utc().date();
    format!("aidb-connections-{today}.json")
}

/// What would happen to one entry of an import file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ImportStatus {
    Ready,
    Duplicate,
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ImportCandidate {
    pub label: String,
    pub status: ImportStatus,
    pub problem: Option<String>,
    /// The profile as it would be saved (always read-only); `None` if the entry is unreadable.
    pub connection: Option<ConnectionDraft>,
    /// The file had writes enabled; the import turns them off.
    pub writes_disabled: bool,
    pub unencrypted_remote: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub file_name: String,
    pub candidates: Vec<ImportCandidate>,
}

/// Reads an export file and reports what importing it would do. Saves nothing.
///
/// # Errors
///
/// `invalid_input` if the file as a whole cannot be used, `storage` if existing names cannot be
/// read.
pub async fn preview_import(
    store: &LocalStore,
    file_name: String,
    bytes: &[u8],
) -> AppResult<ImportPreview> {
    let entries = connection_transfer::parse(bytes)?;
    let mut taken: HashSet<String> = store
        .connections()
        .list()
        .await?
        .into_iter()
        .map(|profile| name_key(&profile.settings.name))
        .collect();
    let candidates = entries
        .into_iter()
        .map(|entry| candidate(entry, &mut taken))
        .collect();
    Ok(ImportPreview {
        file_name,
        candidates,
    })
}

fn candidate(entry: FileEntry, taken: &mut HashSet<String>) -> ImportCandidate {
    let invalid = |label: String, problem: String| ImportCandidate {
        label,
        status: ImportStatus::Invalid,
        problem: Some(problem),
        connection: None,
        writes_disabled: false,
        unencrypted_remote: false,
    };
    let file_connection = match entry.content {
        Ok(connection) => connection,
        Err(problem) => return invalid(entry.label, problem),
    };
    let writes_disabled = !file_connection.read_only;
    let draft = match file_connection.into_import_draft().validated() {
        Ok(draft) => draft,
        Err(error) => return invalid(entry.label, format!("{error}.")),
    };
    // SQLite's NOCASE comparison, which enforces unique names, folds ASCII letters only.
    let (status, problem) = if taken.insert(name_key(&draft.name)) {
        (ImportStatus::Ready, None)
    } else {
        (
            ImportStatus::Duplicate,
            Some("A connection with this name already exists.".to_owned()),
        )
    };
    ImportCandidate {
        label: draft.name.clone(),
        status,
        problem,
        writes_disabled,
        unencrypted_remote: draft.is_unencrypted_remote(),
        connection: Some(draft),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SkippedImport {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ImportOutcome {
    pub imported: u32,
    pub skipped: Vec<SkippedImport>,
}

/// Saves the chosen profiles, read-only. Entries that fail validation, need unconfirmed
/// unencrypted access or clash with an existing name are skipped and reported.
///
/// # Errors
///
/// `invalid_input` for more than [`MAX_ENTRIES`] profiles, `storage` if saving fails.
pub async fn import(
    store: &LocalStore,
    drafts: Vec<ConnectionDraft>,
    unencrypted_allowed: bool,
) -> AppResult<ImportOutcome> {
    if drafts.len() > MAX_ENTRIES {
        return Err(AppError::invalid_input(format!(
            "Import at most {MAX_ENTRIES} connections at a time."
        )));
    }
    let mut outcome = ImportOutcome {
        imported: 0,
        skipped: Vec::new(),
    };
    for draft in drafts {
        let name = draft.name.trim().to_owned();
        let draft = ConnectionDraft {
            read_only: true,
            ..draft
        };
        let saved = match draft.validated() {
            Ok(draft) => match draft.check_transport(unencrypted_allowed) {
                Ok(()) => store
                    .connections()
                    .insert(&draft)
                    .await
                    .map_err(|error| profile_error(error, &draft.name)),
                Err(error) => Err(error.into()),
            },
            Err(error) => Err(error.into()),
        };
        match saved {
            Ok(profile) => {
                outcome.imported += 1;
                record(
                    store,
                    "connection_imported",
                    profile.id,
                    profile.settings.environment,
                    access(profile.settings.read_only),
                )
                .await;
            }
            Err(error) if error.code() == ErrorCode::Storage => return Err(error),
            Err(error) => outcome.skipped.push(SkippedImport {
                name,
                reason: error.message().into_owned(),
            }),
        }
    }
    tracing::info!(
        imported = outcome.imported,
        skipped = outcome.skipped.len(),
        "connections imported"
    );
    Ok(outcome)
}

fn name_key(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn count(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

fn missing() -> AppError {
    AppError::not_found("The connection no longer exists.")
}

fn profile_error(error: StoreError, name: &str) -> AppError {
    match error {
        StoreError::Duplicate(_) => {
            AppError::conflict(format!("A connection named “{name}” already exists."))
        }
        StoreError::NotFound => missing(),
        other => other.into(),
    }
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use super::*;
    use crate::adapters::local_store::test_support::temp_store;
    use crate::domain::connections::{Environment, SslMode};

    const PASSWORD: &str = "ex-p0rt-n3ver-c4rries-th1s-9d2e";

    fn draft(name: &str) -> ConnectionDraft {
        ConnectionDraft::new(name, "db.internal", "orders", "reader")
    }

    fn unencrypted(name: &str) -> ConnectionDraft {
        ConnectionDraft {
            ssl_mode: SslMode::Disable,
            ..draft(name)
        }
    }

    #[tokio::test]
    async fn created_profiles_are_listed_with_their_password_status() {
        let (_dir, store) = temp_store().await;
        let credentials = Credentials::in_memory_as_os_store();
        let saved = create(&store, &credentials, draft("Orders"), false)
            .await
            .expect("create");
        connection_secrets::save_password(
            &store,
            &credentials,
            saved.id,
            SecretString::from(PASSWORD.to_owned()),
        )
        .await
        .expect("save password");
        create(&store, &credentials, draft("Billing"), false)
            .await
            .expect("create");

        let listed = list(&store, &credentials).await.expect("list");

        let summary: Vec<(&str, PasswordStorage)> = listed
            .iter()
            .map(|c| (c.settings.name.as_str(), c.password))
            .collect();
        assert_eq!(
            summary,
            [
                ("Billing", PasswordStorage::NotSaved),
                ("Orders", PasswordStorage::OsStore)
            ]
        );
    }

    #[tokio::test]
    async fn session_only_passwords_are_reported_without_touching_a_store() {
        let (_dir, store) = temp_store().await;
        let credentials = Credentials::session_only("no keychain in tests");
        let saved = create(&store, &credentials, draft("Orders"), false)
            .await
            .expect("create");
        connection_secrets::save_password(
            &store,
            &credentials,
            saved.id,
            SecretString::from(PASSWORD.to_owned()),
        )
        .await
        .expect("save password");

        let listed = list(&store, &credentials).await.expect("list");

        assert_eq!(listed[0].password, PasswordStorage::SessionOnly);
    }

    #[tokio::test]
    async fn invalid_or_duplicate_profiles_are_refused() {
        let (_dir, store) = temp_store().await;
        let credentials = Credentials::in_memory_as_os_store();
        create(&store, &credentials, draft("Orders"), false)
            .await
            .expect("create");

        let duplicate = create(&store, &credentials, draft("ORDERS"), false)
            .await
            .expect_err("duplicate");
        let invalid = create(
            &store,
            &credentials,
            ConnectionDraft {
                port: 0,
                ..draft("Bad port")
            },
            false,
        )
        .await
        .expect_err("invalid");

        assert_eq!(duplicate.code(), ErrorCode::Conflict);
        assert_eq!(
            duplicate.message(),
            "A connection named “ORDERS” already exists."
        );
        assert_eq!(invalid.code(), ErrorCode::InvalidInput);
    }

    #[tokio::test]
    async fn unencrypted_remote_profiles_need_consent() {
        let (_dir, store) = temp_store().await;
        let credentials = Credentials::in_memory_as_os_store();

        let refused = create(&store, &credentials, unencrypted("Plain"), false)
            .await
            .expect_err("needs consent");
        let saved = create(&store, &credentials, unencrypted("Plain"), true)
            .await
            .expect("saved with consent");

        assert_eq!(refused.code(), ErrorCode::InvalidInput);
        assert!(saved.unencrypted_remote);
        let renamed = update(
            &store,
            &credentials,
            saved.id,
            unencrypted("Plain (renamed)"),
            false,
        )
        .await;
        assert!(renamed.is_err(), "every save needs consent again");
    }

    #[tokio::test]
    async fn updates_change_settings_and_missing_profiles_are_reported() {
        let (_dir, store) = temp_store().await;
        let credentials = Credentials::in_memory_as_os_store();
        let saved = create(&store, &credentials, draft("Orders"), false)
            .await
            .expect("create");

        let updated = update(
            &store,
            &credentials,
            saved.id,
            ConnectionDraft {
                environment: Environment::Production,
                ..draft("Orders")
            },
            false,
        )
        .await
        .expect("update");
        let missing = update(
            &store,
            &credentials,
            ConnectionId::new(),
            draft("Ghost"),
            false,
        )
        .await
        .expect_err("missing");

        assert_eq!(updated.settings.environment, Environment::Production);
        assert_eq!(missing.code(), ErrorCode::NotFound);
    }

    #[tokio::test]
    async fn deleting_can_keep_or_forget_the_password() {
        let (_dir, store) = temp_store().await;
        let credentials = Credentials::in_memory_as_os_store();
        let mut ids = Vec::new();
        for name in ["Keep", "Forget"] {
            let saved = create(&store, &credentials, draft(name), false)
                .await
                .expect("create");
            connection_secrets::save_password(
                &store,
                &credentials,
                saved.id,
                SecretString::from(PASSWORD.to_owned()),
            )
            .await
            .expect("save password");
            ids.push(saved.id);
        }

        delete(&store, &credentials, ids[0], false)
            .await
            .expect("delete, keep password");
        delete(&store, &credentials, ids[1], true)
            .await
            .expect("delete, forget password");

        assert!(list(&store, &credentials).await.expect("list").is_empty());
        let kept = connection_secrets::load_password(&credentials, ids[0])
            .await
            .expect("load");
        let forgotten = connection_secrets::load_password(&credentials, ids[1])
            .await
            .expect("load");
        assert!(kept.is_some());
        assert!(forgotten.is_none());
        let again = delete(&store, &credentials, ids[0], false)
            .await
            .expect_err("already gone");
        assert_eq!(again.code(), ErrorCode::NotFound);
    }

    #[tokio::test]
    async fn exports_never_contain_passwords_or_references() {
        let (_dir, store) = temp_store().await;
        let credentials = Credentials::in_memory_as_os_store();
        let saved = create(&store, &credentials, draft("Orders"), false)
            .await
            .expect("create");
        connection_secrets::save_password(
            &store,
            &credentials,
            saved.id,
            SecretString::from(PASSWORD.to_owned()),
        )
        .await
        .expect("save password");

        let (bytes, exported) = export(&store).await.expect("export");
        let text = String::from_utf8(bytes).expect("utf-8");

        assert_eq!(exported, 1);
        assert!(text.contains("\"Orders\""));
        assert!(!text.contains(PASSWORD));
        assert!(!text.contains(SecretRef::connection(saved.id).as_str()));
        assert!(!text.contains(&saved.id.to_string()));
    }

    #[tokio::test]
    async fn export_files_import_into_another_store_read_only() {
        let (_source_dir, source) = temp_store().await;
        let credentials = Credentials::in_memory_as_os_store();
        create(
            &source,
            &credentials,
            ConnectionDraft {
                read_only: false,
                environment: Environment::Staging,
                ..draft("Orders")
            },
            false,
        )
        .await
        .expect("create");
        let (bytes, _) = export(&source).await.expect("export");

        let (_target_dir, target) = temp_store().await;
        let preview = preview_import(&target, "orders.json".into(), &bytes)
            .await
            .expect("preview");
        let candidate = &preview.candidates[0];
        assert_eq!(candidate.status, ImportStatus::Ready);
        assert!(candidate.writes_disabled);
        let chosen = preview
            .candidates
            .iter()
            .filter_map(|c| c.connection.clone())
            .collect();
        let outcome = import(&target, chosen, false).await.expect("import");

        assert_eq!(outcome.imported, 1);
        let imported = list(&target, &credentials).await.expect("list");
        assert!(imported[0].settings.read_only);
        assert_eq!(imported[0].settings.environment, Environment::Staging);
        assert_eq!(imported[0].password, PasswordStorage::NotSaved);
    }

    #[tokio::test]
    async fn previews_flag_duplicates_invalid_entries_and_plaintext() {
        let (_dir, store) = temp_store().await;
        let credentials = Credentials::in_memory_as_os_store();
        create(&store, &credentials, draft("Orders"), false)
            .await
            .expect("create");
        let file = br#"{"format":"aidb.connections","version":1,"connections":[
            {"name":"orders","host":"db","database":"app","username":"r"},
            {"name":"Billing","host":"db.internal","database":"app","username":"r","sslMode":"disable"},
            {"name":"billing","host":"db","database":"app","username":"r"},
            {"name":"Broken","host":"bad host","database":"app","username":"r"}
        ]}"#;

        let preview = preview_import(&store, "f.json".into(), file)
            .await
            .expect("preview");

        let statuses: Vec<ImportStatus> = preview.candidates.iter().map(|c| c.status).collect();
        assert_eq!(
            statuses,
            [
                ImportStatus::Duplicate,
                ImportStatus::Ready,
                ImportStatus::Duplicate,
                ImportStatus::Invalid
            ]
        );
        assert!(preview.candidates[1].unencrypted_remote);
        assert_eq!(
            preview.candidates[3].problem.as_deref(),
            Some("Host must be a host name or an IP address.")
        );
    }

    #[tokio::test]
    async fn imports_skip_what_cannot_be_saved_and_force_read_only() {
        let (_dir, store) = temp_store().await;
        let credentials = Credentials::in_memory_as_os_store();
        create(&store, &credentials, draft("Orders"), false)
            .await
            .expect("create");

        let outcome = import(
            &store,
            vec![
                ConnectionDraft {
                    read_only: false,
                    ..draft("Billing")
                },
                draft("orders"),
                unencrypted("Plain"),
                ConnectionDraft {
                    host: "bad host".into(),
                    ..draft("Broken")
                },
            ],
            false,
        )
        .await
        .expect("import");

        assert_eq!(outcome.imported, 1);
        let skipped: Vec<&str> = outcome.skipped.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(skipped, ["orders", "Plain", "Broken"]);
        let billing = list(&store, &credentials)
            .await
            .expect("list")
            .into_iter()
            .find(|c| c.settings.name == "Billing")
            .expect("imported");
        assert!(billing.settings.read_only);
    }

    #[tokio::test]
    async fn safety_relevant_changes_are_audited_without_details() {
        use crate::domain::audit::AuditFilter;

        let (_dir, store) = temp_store().await;
        let credentials = Credentials::session_only("tests");
        let created = create(&store, &credentials, draft("Billing"), false)
            .await
            .expect("create");
        let mut writable = draft("Billing");
        writable.read_only = false;
        writable.environment = Environment::Production;
        update(&store, &credentials, created.id, writable, false)
            .await
            .expect("update");
        delete(&store, &credentials, created.id, false)
            .await
            .expect("delete");

        let events = store
            .audit()
            .list(&AuditFilter {
                connection_id: Some(created.id),
                before_id: None,
                limit: 10,
            })
            .await
            .expect("audit");
        let summary: Vec<(&str, Option<&str>)> = events
            .iter()
            .map(|event| (event.operation.as_str(), event.detail.as_deref()))
            .collect();
        assert_eq!(
            summary,
            [
                ("connection_deleted", Some("Removed.")),
                (
                    "connection_updated",
                    Some("Environment changed from development to production. Writes turned on.")
                ),
                ("connection_created", Some("Read-only.")),
            ]
        );
        assert_eq!(events[1].environment, Some(Environment::Production));
        for event in &events {
            let detail = event.detail.as_deref().unwrap_or_default();
            assert!(!detail.contains("db.internal") && !detail.contains("reader"));
        }
    }
}
