//! Saving, reading and forgetting a connection's password.
//!
//! The password goes to the credential store. The local database records the secret's reference
//! only when the secret is durable (OS store), so session-only secrets leave nothing on disk.

use secrecy::{ExposeSecret, SecretString};

use super::run_blocking;
use crate::adapters::credentials::{CredentialBackend, Credentials, SecretRef};
use crate::adapters::local_store::{LocalStore, StoreError};
use crate::domain::connections::ConnectionId;
use crate::errors::{AppError, AppResult};

/// Longest password accepted, in bytes. Within every OS store's limit.
pub const MAX_PASSWORD_BYTES: usize = 2048;

/// Stores a connection's password and records where it lives. Returns where it was stored.
///
/// # Errors
///
/// `invalid_input` for an empty or oversized password, `not_found` if the profile does not
/// exist, `credential_store` or `storage` if either store fails. On failure nothing is left
/// behind.
pub async fn save_password(
    store: &LocalStore,
    credentials: &Credentials,
    id: ConnectionId,
    password: SecretString,
) -> AppResult<CredentialBackend> {
    let length = password.expose_secret().len();
    if length == 0 {
        return Err(AppError::invalid_input("The password is empty."));
    }
    if length > MAX_PASSWORD_BYTES {
        return Err(AppError::invalid_input(format!(
            "The password must be at most {MAX_PASSWORD_BYTES} bytes."
        )));
    }
    if store.connections().get(id).await?.is_none() {
        return Err(AppError::not_found("The connection no longer exists."));
    }

    let key = SecretRef::connection(id);
    {
        let (credentials, key) = (credentials.clone(), key.clone());
        run_blocking(move || credentials.store(&key, &password)).await??;
    }

    let durable = credentials.backend() == CredentialBackend::OsStore;
    let reference = durable.then(|| key.as_str());
    if let Err(error) = store.connections().set_secret_ref(id, reference).await {
        // Do not leave a secret behind that no profile points to.
        let (credentials, key) = (credentials.clone(), key.clone());
        if let Err(cleanup) = run_blocking(move || credentials.forget(&key)).await? {
            tracing::warn!(connection = %id, error = %cleanup, "could not remove an orphaned password");
        }
        return Err(error.into());
    }

    tracing::info!(connection = %id, backend = ?credentials.backend(), "connection password saved");
    Ok(credentials.backend())
}

/// Reads a connection's password, or `None` if none is stored (or it was session-only and the
/// app has restarted since).
///
/// # Errors
///
/// `credential_store` if the store cannot be read.
pub async fn load_password(
    credentials: &Credentials,
    id: ConnectionId,
) -> AppResult<Option<SecretString>> {
    let (credentials, key) = (credentials.clone(), SecretRef::connection(id));
    Ok(run_blocking(move || credentials.load(&key)).await??)
}

/// Removes a connection's password from the credential store and clears its reference.
/// Returns whether a password existed. Works after the profile itself was deleted.
///
/// # Errors
///
/// `credential_store` or `storage` if either store fails.
pub async fn forget_password(
    store: &LocalStore,
    credentials: &Credentials,
    id: ConnectionId,
) -> AppResult<bool> {
    let existed = {
        let (credentials, key) = (credentials.clone(), SecretRef::connection(id));
        run_blocking(move || credentials.forget(&key)).await??
    };
    match store.connections().set_secret_ref(id, None).await {
        Ok(()) | Err(StoreError::NotFound) => {}
        Err(error) => return Err(error.into()),
    }
    tracing::info!(connection = %id, existed, "connection password forgotten");
    Ok(existed)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::adapters::local_store::test_support::temp_store;
    use crate::domain::connections::{ConnectionDraft, ConnectionProfile};
    use crate::errors::ErrorCode;

    /// Distinctive enough that finding it anywhere means it leaked.
    const PASSWORD: &str = "c0rrect-H0rse-b4ttery-st4ple-7f3a9";

    fn password() -> SecretString {
        SecretString::from(PASSWORD.to_owned())
    }

    async fn profile(store: &LocalStore) -> ConnectionProfile {
        let draft = ConnectionDraft::new("Orders", "db.internal", "orders", "reader")
            .validated()
            .expect("valid draft");
        store.connections().insert(&draft).await.expect("insert")
    }

    /// Asserts that no file in `dir` (database, WAL, shared memory) contains the password.
    fn assert_absent_from_files(dir: &Path) {
        for entry in std::fs::read_dir(dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_file() {
                let bytes = std::fs::read(&path).expect("read file");
                assert!(
                    !bytes
                        .windows(PASSWORD.len())
                        .any(|window| window == PASSWORD.as_bytes()),
                    "password found in {}",
                    path.display()
                );
            }
        }
    }

    #[tokio::test]
    async fn durable_secrets_leave_only_a_reference_in_the_database() {
        let (dir, store) = temp_store().await;
        let credentials = Credentials::in_memory_as_os_store();
        let saved = profile(&store).await;

        let backend = save_password(&store, &credentials, saved.id, password())
            .await
            .expect("save");

        let reloaded = store
            .connections()
            .get(saved.id)
            .await
            .expect("get")
            .expect("exists");
        assert_eq!(backend, CredentialBackend::OsStore);
        assert_eq!(
            reloaded.secret_ref.as_deref(),
            Some(SecretRef::connection(saved.id).as_str())
        );
        let loaded = load_password(&credentials, saved.id).await.expect("load");
        assert_eq!(
            loaded.as_ref().map(ExposeSecret::expose_secret),
            Some(PASSWORD)
        );
        assert_absent_from_files(dir.path());
    }

    #[tokio::test]
    async fn session_only_secrets_leave_nothing_on_disk() {
        let (dir, store) = temp_store().await;
        let credentials = Credentials::session_only("no keychain");
        let saved = profile(&store).await;

        let backend = save_password(&store, &credentials, saved.id, password())
            .await
            .expect("save");

        let reloaded = store.connections().get(saved.id).await.expect("get");
        assert_eq!(backend, CredentialBackend::SessionOnly);
        assert_eq!(reloaded.and_then(|p| p.secret_ref), None);
        assert!(
            load_password(&credentials, saved.id)
                .await
                .expect("load")
                .is_some()
        );
        assert_absent_from_files(dir.path());
    }

    #[tokio::test]
    async fn forgetting_removes_the_secret_and_its_reference() {
        let (_dir, store) = temp_store().await;
        let credentials = Credentials::in_memory_as_os_store();
        let saved = profile(&store).await;
        save_password(&store, &credentials, saved.id, password())
            .await
            .expect("save");

        let existed = forget_password(&store, &credentials, saved.id)
            .await
            .expect("forget");

        assert!(existed);
        assert!(
            load_password(&credentials, saved.id)
                .await
                .expect("load")
                .is_none()
        );
        let reloaded = store.connections().get(saved.id).await.expect("get");
        assert_eq!(reloaded.and_then(|p| p.secret_ref), None);
    }

    #[tokio::test]
    async fn forgetting_still_works_after_the_profile_is_deleted() {
        let (_dir, store) = temp_store().await;
        let credentials = Credentials::in_memory_as_os_store();
        let saved = profile(&store).await;
        save_password(&store, &credentials, saved.id, password())
            .await
            .expect("save");
        store.connections().delete(saved.id).await.expect("delete");

        assert!(
            forget_password(&store, &credentials, saved.id)
                .await
                .expect("forget")
        );
    }

    #[tokio::test]
    async fn invalid_requests_store_nothing() {
        let (_dir, store) = temp_store().await;
        let credentials = Credentials::session_only("test");
        let saved = profile(&store).await;
        let missing = ConnectionId::new();

        let empty = save_password(
            &store,
            &credentials,
            saved.id,
            SecretString::from(String::new()),
        )
        .await;
        let oversized = save_password(
            &store,
            &credentials,
            saved.id,
            SecretString::from("x".repeat(MAX_PASSWORD_BYTES + 1)),
        )
        .await;
        let unknown = save_password(&store, &credentials, missing, password()).await;

        assert_eq!(empty.expect_err("empty").code(), ErrorCode::InvalidInput);
        assert_eq!(
            oversized.expect_err("too long").code(),
            ErrorCode::InvalidInput
        );
        assert_eq!(unknown.expect_err("unknown").code(), ErrorCode::NotFound);
        assert!(
            load_password(&credentials, saved.id)
                .await
                .expect("load")
                .is_none()
        );
        assert!(
            load_password(&credentials, missing)
                .await
                .expect("load")
                .is_none()
        );
    }

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .expect("capture lock")
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn logs_never_contain_the_password() {
        let capture = Capture::default();
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || writer.clone())
            .finish();
        // While only one dispatcher exists, tracing-core asks the registering thread's default
        // instead of every dispatcher, so a callsite first hit by a parallel test would be cached
        // as disabled. A second, idle dispatcher makes it consult the capturing one too.
        let _idle = tracing::Dispatch::new(tracing_subscriber::registry());
        let _guard = tracing::subscriber::set_default(subscriber);

        let (_dir, store) = temp_store().await;
        let credentials = Credentials::in_memory_as_os_store();
        let saved = profile(&store).await;
        save_password(&store, &credentials, saved.id, password())
            .await
            .expect("save");
        load_password(&credentials, saved.id).await.expect("load");
        let failure = save_password(&store, &credentials, ConnectionId::new(), password()).await;
        tracing::error!(error = ?failure.expect_err("missing profile"), "logged failure");
        forget_password(&store, &credentials, saved.id)
            .await
            .expect("forget");

        let logs = String::from_utf8(capture.0.lock().expect("lock").clone()).expect("utf-8 logs");
        assert!(logs.contains("connection password saved"), "{logs}");
        assert!(!logs.contains(PASSWORD), "{logs}");
    }
}
