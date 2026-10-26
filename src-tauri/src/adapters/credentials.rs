//! Secrets (database passwords, API keys) in the OS credential store.
//!
//! macOS Keychain, Windows Credential Manager and the Secret Service on Linux are reached through
//! the `keyring` crate's native stores. When none is available, secrets are kept in memory for the
//! current session only, and the user is told so. There is never a plaintext fallback. Secrets
//! stay wrapped in [`SecretString`], which redacts itself in `Debug` and zeroes memory on drop.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use thiserror::Error;
use zeroize::Zeroize;

use crate::domain::connections::ConnectionId;
use crate::errors::{AppError, ErrorCode};

/// Service name under which every aidb secret is filed in the OS store.
pub const SERVICE: &str = "com.orbitratech.aidb";

/// Entry used only to check that the OS store answers; it is never written.
const PROBE_ENTRY: &str = "aidb:availability-probe";

/// Name of a secret's entry in the credential store. This, never the secret, is what other
/// components keep.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SecretRef(String);

impl SecretRef {
    /// The password of a saved connection profile.
    #[must_use]
    pub fn connection(id: ConnectionId) -> Self {
        Self(format!("connection:{id}"))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where secrets are kept on this device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum CredentialBackend {
    /// The operating system's credential store; secrets survive restarts.
    OsStore,
    /// Memory only; secrets are forgotten when the app quits.
    SessionOnly,
}

#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("the OS credential store is unavailable: {0}")]
    Unavailable(String),
    #[error("the OS credential store denied access: {0}")]
    AccessDenied(String),
    #[error("the stored secret could not be decoded")]
    Unreadable,
    #[error("the secret is longer than the OS credential store allows")]
    TooLong,
    #[error("the OS credential store failed: {0}")]
    Platform(String),
}

impl From<keyring::Error> for CredentialError {
    fn from(error: keyring::Error) -> Self {
        use keyring::Error as K;
        match error {
            // These variants carry the raw secret bytes. Zero them and keep no detail.
            K::BadEncoding(mut bytes) => {
                bytes.zeroize();
                Self::Unreadable
            }
            K::BadDataFormat(mut bytes, _) => {
                bytes.zeroize();
                Self::Unreadable
            }
            K::NoStorageAccess(cause) => Self::AccessDenied(cause.to_string()),
            K::NoDefaultStore => Self::Unavailable("no default store".to_owned()),
            K::TooLong(..) => Self::TooLong,
            other => Self::Platform(other.to_string()),
        }
    }
}

impl From<CredentialError> for AppError {
    fn from(error: CredentialError) -> Self {
        let message = match &error {
            CredentialError::AccessDenied(_) => {
                "The system keychain is locked or denied access. Unlock it and try again."
            }
            CredentialError::TooLong => "The secret is too long to store in the system keychain.",
            CredentialError::Unreadable => {
                "The saved secret could not be read. Enter it again to replace it."
            }
            CredentialError::Unavailable(_) | CredentialError::Platform(_) => {
                "The system keychain could not be used. Details are in the local log."
            }
        };
        Self::new(ErrorCode::CredentialStore, message).with_cause(error)
    }
}

#[derive(Debug)]
enum Backend {
    Os,
    Memory(Mutex<HashMap<SecretRef, SecretString>>),
}

/// Handle to the credential store chosen at startup. Cheap to clone.
#[derive(Clone)]
pub struct Credentials {
    inner: Arc<Inner>,
}

struct Inner {
    backend: Backend,
    kind: CredentialBackend,
    /// Why the OS store is not used, when it is not.
    fallback_reason: Option<String>,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never print stored secrets, even redacted, and never their names in bulk.
        f.debug_struct("Credentials")
            .field("backend", &self.inner.kind)
            .field("fallback_reason", &self.inner.fallback_reason)
            .finish_non_exhaustive()
    }
}

impl Credentials {
    /// Uses the OS credential store if it answers, otherwise session-only memory.
    ///
    /// Blocking: talks to the OS store. Call once at startup.
    #[must_use]
    pub fn detect() -> Self {
        match probe_os_store() {
            Ok(()) => Self::new(Backend::Os, CredentialBackend::OsStore, None),
            Err(error) => {
                tracing::warn!(error = %error, "OS credential store unavailable; secrets will last for this session only");
                Self::session_only(error.to_string())
            }
        }
    }

    /// Keeps secrets in memory only, for when no OS store is available.
    #[must_use]
    pub fn session_only(reason: impl Into<String>) -> Self {
        Self::new(
            Backend::Memory(Mutex::default()),
            CredentialBackend::SessionOnly,
            Some(reason.into()),
        )
    }

    /// In-memory store that reports itself as durable, to test code paths for the OS store.
    #[cfg(test)]
    pub(crate) fn in_memory_as_os_store() -> Self {
        Self::new(
            Backend::Memory(Mutex::default()),
            CredentialBackend::OsStore,
            None,
        )
    }

    fn new(backend: Backend, kind: CredentialBackend, fallback_reason: Option<String>) -> Self {
        Self {
            inner: Arc::new(Inner {
                backend,
                kind,
                fallback_reason,
            }),
        }
    }

    #[must_use]
    pub fn backend(&self) -> CredentialBackend {
        self.inner.kind
    }

    /// Why secrets are session-only, if they are.
    #[must_use]
    pub fn fallback_reason(&self) -> Option<&str> {
        self.inner.fallback_reason.as_deref()
    }

    /// Stores or replaces a secret. Blocking when the OS store is used.
    ///
    /// # Errors
    ///
    /// Fails if the OS store rejects the write.
    pub fn store(&self, key: &SecretRef, secret: &SecretString) -> Result<(), CredentialError> {
        match &self.inner.backend {
            Backend::Os => {
                os_entry(key)?.set_password(secret.expose_secret())?;
                Ok(())
            }
            Backend::Memory(map) => {
                lock(map).insert(key.clone(), secret.clone());
                Ok(())
            }
        }
    }

    /// Reads a secret, or `None` if none is stored. Blocking when the OS store is used.
    ///
    /// # Errors
    ///
    /// Fails if the OS store cannot be read.
    pub fn load(&self, key: &SecretRef) -> Result<Option<SecretString>, CredentialError> {
        match &self.inner.backend {
            Backend::Os => match os_entry(key)?.get_password() {
                Ok(password) => Ok(Some(SecretString::from(password))),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(error) => Err(error.into()),
            },
            Backend::Memory(map) => Ok(lock(map).get(key).cloned()),
        }
    }

    /// Deletes a secret. Returns whether one existed. Blocking when the OS store is used.
    ///
    /// # Errors
    ///
    /// Fails if the OS store rejects the deletion.
    pub fn forget(&self, key: &SecretRef) -> Result<bool, CredentialError> {
        match &self.inner.backend {
            Backend::Os => match os_entry(key)?.delete_credential() {
                Ok(()) => Ok(true),
                Err(keyring::Error::NoEntry) => Ok(false),
                Err(error) => Err(error.into()),
            },
            Backend::Memory(map) => Ok(lock(map).remove(key).is_some()),
        }
    }
}

fn lock(
    map: &Mutex<HashMap<SecretRef, SecretString>>,
) -> std::sync::MutexGuard<'_, HashMap<SecretRef, SecretString>> {
    // A panic while holding the lock cannot leave the map half-updated; keep using it.
    map.lock().unwrap_or_else(PoisonError::into_inner)
}

fn os_entry(key: &SecretRef) -> Result<keyring::Entry, CredentialError> {
    Ok(keyring::Entry::new(SERVICE, key.as_str())?)
}

/// Checks that the OS store initialized and answers a read.
fn probe_os_store() -> Result<(), CredentialError> {
    if let Err(error) = keyring::Entry::store_status() {
        return Err(CredentialError::Unavailable(error.to_string()));
    }
    match keyring::Entry::new(SERVICE, PROBE_ENTRY)?.get_password() {
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(text: &str) -> SecretString {
        SecretString::from(text.to_owned())
    }

    /// Exercises the real OS store. Needs an unlocked keychain, so it runs only on request:
    /// `cargo test os_store -- --ignored` (CI runs it on Linux against gnome-keyring).
    #[test]
    #[ignore = "needs an unlocked OS credential store"]
    fn os_store_round_trips_and_forgets() {
        let credentials = Credentials::detect();
        assert_eq!(
            credentials.backend(),
            CredentialBackend::OsStore,
            "OS store unavailable: {:?}",
            credentials.fallback_reason()
        );
        let key = SecretRef::connection(ConnectionId::new());

        credentials.store(&key, &secret("hunter2")).expect("store");
        credentials
            .store(&key, &secret("hunter3"))
            .expect("replace");
        let loaded = credentials.load(&key).expect("load");
        let forgotten = credentials.forget(&key).expect("forget");
        let after = credentials.load(&key).expect("load after forget");

        assert_eq!(
            loaded.as_ref().map(ExposeSecret::expose_secret),
            Some("hunter3")
        );
        assert!(forgotten);
        assert!(after.is_none());
    }

    #[test]
    fn session_store_round_trips_and_forgets() {
        let credentials = Credentials::session_only("no keychain in tests");
        let key = SecretRef::connection(ConnectionId::new());

        credentials.store(&key, &secret("hunter2")).expect("store");
        let loaded = credentials.load(&key).expect("load");
        let forgotten = credentials.forget(&key).expect("forget");
        let after = credentials.load(&key).expect("load again");

        assert_eq!(
            loaded.as_ref().map(ExposeSecret::expose_secret),
            Some("hunter2")
        );
        assert!(forgotten);
        assert!(after.is_none());
        assert!(!credentials.forget(&key).expect("forget again"));
    }

    #[test]
    fn session_only_mode_is_reported_with_its_reason() {
        let credentials = Credentials::session_only("Secret Service not running");

        assert_eq!(credentials.backend(), CredentialBackend::SessionOnly);
        assert_eq!(
            credentials.fallback_reason(),
            Some("Secret Service not running")
        );
    }

    #[test]
    fn debug_output_never_contains_secrets_or_entry_names() {
        let credentials = Credentials::session_only("test");
        let key = SecretRef::connection(ConnectionId::new());
        credentials.store(&key, &secret("hunter2")).expect("store");

        let debug = format!("{credentials:?}");

        assert!(!debug.contains("hunter2"));
        assert!(!debug.contains(key.as_str()));
    }

    #[test]
    fn keyring_errors_with_secret_bytes_are_dropped_without_detail() {
        let raw = b"hunter2".to_vec();

        let error = CredentialError::from(keyring::Error::BadEncoding(raw));
        let app_error = AppError::from(error);

        assert!(!format!("{app_error:?}").contains("hunter2"));
        assert!(!format!("{app_error:?}").contains("104, 117"));
        assert_eq!(app_error.code(), ErrorCode::CredentialStore);
    }

    #[test]
    fn access_errors_ask_the_user_to_unlock_the_keychain() {
        let error = AppError::from(CredentialError::AccessDenied("locked".into()));

        assert!(error.message().contains("locked or denied access"));
        assert!(!error.is_retryable());
    }

    #[test]
    fn connection_refs_are_stable_and_namespaced() {
        let id = ConnectionId::new();

        assert_eq!(SecretRef::connection(id), SecretRef::connection(id));
        assert_eq!(
            SecretRef::connection(id).as_str(),
            format!("connection:{id}")
        );
    }
}
