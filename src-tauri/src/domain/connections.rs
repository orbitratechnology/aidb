//! Connection profiles: what aidb knows about a database it can connect to.
//!
//! A profile never holds a password. `secret_ref` names an entry in the OS credential store.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::errors::AppError;

/// Stable identifier of a saved connection profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConnectionId(Uuid);

impl ConnectionId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ConnectionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.hyphenated().fmt(f)
    }
}

impl FromStr for ConnectionId {
    type Err = uuid::Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(text).map(Self)
    }
}

/// Database engines aidb can connect to. PostgreSQL only for the MVP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    #[default]
    Postgres,
}

/// Deployment environment of a connection. Shown everywhere the connection is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    Local,
    #[default]
    Development,
    Staging,
    Production,
    Other,
}

/// How the PostgreSQL connection uses TLS. Defaults to full certificate verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SslMode {
    Disable,
    Require,
    VerifyCa,
    #[default]
    VerifyFull,
}

/// Generates `as_str` and `FromStr` from one list of wire names, so the two can never disagree.
macro_rules! wire_names {
    ($kind:literal, $ty:ty { $($variant:ident => $name:literal),+ $(,)? }) => {
        impl $ty {
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $name),+
                }
            }
        }

        impl FromStr for $ty {
            type Err = UnknownValue;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                match text {
                    $($name => Ok(Self::$variant),)+
                    _ => Err(UnknownValue { kind: $kind, value: text.to_owned() }),
                }
            }
        }
    };
}

wire_names!("engine", Engine { Postgres => "postgres" });
wire_names!("environment", Environment {
    Local => "local",
    Development => "development",
    Staging => "staging",
    Production => "production",
    Other => "other",
});
wire_names!("ssl mode", SslMode {
    Disable => "disable",
    Require => "require",
    VerifyCa => "verify-ca",
    VerifyFull => "verify-full",
});

/// A stored value that does not match any known variant.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unknown {kind} `{value}`")]
pub struct UnknownValue {
    kind: &'static str,
    value: String,
}

/// Limits shared with the local store's CHECK constraints.
pub mod limits {
    pub const NAME_MAX: usize = 100;
    pub const HOST_MAX: usize = 255;
    /// PostgreSQL truncates identifiers longer than 63 bytes.
    pub const IDENTIFIER_MAX: usize = 63;
    pub const CA_CERT_PATH_MAX: usize = 4096;
    pub const CONNECT_TIMEOUT_SECS: std::ops::RangeInclusive<u32> = 1..=300;
}

/// Editable fields of a connection profile, validated before they are stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionDraft {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub ssl_mode: SslMode,
    pub ca_cert_path: Option<String>,
    pub connect_timeout_secs: u32,
    pub environment: Environment,
    pub read_only: bool,
}

impl ConnectionDraft {
    /// A draft with safe defaults: TLS with full verification, read-only, 10 second timeout.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        host: impl Into<String>,
        database: impl Into<String>,
        username: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            host: host.into(),
            port: 5432,
            database: database.into(),
            username: username.into(),
            ssl_mode: SslMode::default(),
            ca_cert_path: None,
            connect_timeout_secs: 10,
            environment: Environment::default(),
            read_only: true,
        }
    }

    /// Trims text fields and checks every limit.
    ///
    /// # Errors
    ///
    /// Returns the first [`DraftError`] found.
    pub fn validated(mut self) -> Result<Self, DraftError> {
        self.name = self.name.trim().to_owned();
        self.host = self.host.trim().to_owned();
        self.ca_cert_path = self
            .ca_cert_path
            .map(|path| path.trim().to_owned())
            .filter(|path| !path.is_empty());

        check_text(&self.name, Field::Name, limits::NAME_MAX)?;
        check_text(&self.host, Field::Host, limits::HOST_MAX)?;
        if self.host.chars().any(char::is_whitespace) {
            return Err(DraftError::Invalid(Field::Host, "must not contain spaces"));
        }
        if self.port == 0 {
            return Err(DraftError::Invalid(
                Field::Port,
                "must be between 1 and 65535",
            ));
        }
        check_identifier(&self.database, Field::Database)?;
        check_identifier(&self.username, Field::Username)?;
        if let Some(path) = &self.ca_cert_path
            && path.len() > limits::CA_CERT_PATH_MAX
        {
            return Err(DraftError::TooLong(
                Field::CaCertPath,
                limits::CA_CERT_PATH_MAX,
            ));
        }
        if !limits::CONNECT_TIMEOUT_SECS.contains(&self.connect_timeout_secs) {
            return Err(DraftError::Invalid(
                Field::ConnectTimeout,
                "must be between 1 and 300 seconds",
            ));
        }
        Ok(self)
    }
}

fn check_text(value: &str, field: Field, max: usize) -> Result<(), DraftError> {
    if value.is_empty() {
        Err(DraftError::Missing(field))
    } else if value.chars().count() > max {
        Err(DraftError::TooLong(field, max))
    } else {
        Ok(())
    }
}

fn check_identifier(value: &str, field: Field) -> Result<(), DraftError> {
    if value.is_empty() {
        Err(DraftError::Missing(field))
    } else if value.len() > limits::IDENTIFIER_MAX {
        Err(DraftError::TooLong(field, limits::IDENTIFIER_MAX))
    } else {
        Ok(())
    }
}

/// A saved connection profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionProfile {
    pub id: ConnectionId,
    pub engine: Engine,
    pub settings: ConnectionDraft,
    /// Reference to the password in the OS credential store, if one is saved.
    pub secret_ref: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Fields a user can get wrong when editing a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Host,
    Port,
    Database,
    Username,
    CaCertPath,
    ConnectTimeout,
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Name => "Name",
            Self::Host => "Host",
            Self::Port => "Port",
            Self::Database => "Database",
            Self::Username => "Username",
            Self::CaCertPath => "CA certificate path",
            Self::ConnectTimeout => "Connection timeout",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DraftError {
    #[error("{0} is required")]
    Missing(Field),
    #[error("{0} must be at most {1} characters")]
    TooLong(Field, usize),
    #[error("{0} {1}")]
    Invalid(Field, &'static str),
}

impl From<DraftError> for AppError {
    fn from(error: DraftError) -> Self {
        Self::invalid_input(format!("{error}."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> ConnectionDraft {
        ConnectionDraft::new("Orders (dev)", "db.internal", "orders", "app_reader")
    }

    #[test]
    fn new_drafts_default_to_safe_settings() {
        let draft = draft();

        assert!(draft.read_only);
        assert_eq!(draft.ssl_mode, SslMode::VerifyFull);
        assert_eq!(draft.environment, Environment::Development);
        assert_eq!(draft.port, 5432);
    }

    #[test]
    fn validation_trims_text_and_drops_blank_optional_paths() {
        let mut input = draft();
        input.name = "  Orders  ".into();
        input.host = " db.internal ".into();
        input.ca_cert_path = Some("   ".into());

        let valid = input.validated().expect("draft is valid");

        assert_eq!(valid.name, "Orders");
        assert_eq!(valid.host, "db.internal");
        assert_eq!(valid.ca_cert_path, None);
    }

    type Case = (fn(&mut ConnectionDraft), DraftError);

    #[test]
    fn validation_rejects_each_bad_field() {
        let cases: [Case; 6] = [
            (|d| d.name = "   ".into(), DraftError::Missing(Field::Name)),
            (
                |d| d.host = "db internal".into(),
                DraftError::Invalid(Field::Host, "must not contain spaces"),
            ),
            (
                |d| d.port = 0,
                DraftError::Invalid(Field::Port, "must be between 1 and 65535"),
            ),
            (
                |d| d.database = "x".repeat(64),
                DraftError::TooLong(Field::Database, 63),
            ),
            (
                |d| d.username = String::new(),
                DraftError::Missing(Field::Username),
            ),
            (
                |d| d.connect_timeout_secs = 0,
                DraftError::Invalid(Field::ConnectTimeout, "must be between 1 and 300 seconds"),
            ),
        ];

        for (mutate, expected) in cases {
            let mut input = draft();
            mutate(&mut input);
            assert_eq!(input.validated(), Err(expected));
        }
    }

    #[test]
    fn validation_errors_become_invalid_input() {
        let error = AppError::from(DraftError::Missing(Field::Host));

        assert_eq!(error.code(), crate::errors::ErrorCode::InvalidInput);
        assert_eq!(error.message(), "Host is required.");
    }

    #[test]
    fn wire_names_round_trip() {
        for mode in [
            SslMode::Disable,
            SslMode::Require,
            SslMode::VerifyCa,
            SslMode::VerifyFull,
        ] {
            assert_eq!(mode.as_str().parse::<SslMode>(), Ok(mode));
            assert_eq!(
                serde_json::to_value(mode).expect("serializes"),
                mode.as_str()
            );
        }
        for environment in [
            Environment::Local,
            Environment::Development,
            Environment::Staging,
            Environment::Production,
            Environment::Other,
        ] {
            assert_eq!(environment.as_str().parse::<Environment>(), Ok(environment));
        }
        assert!("prefer".parse::<SslMode>().is_err());
    }

    #[test]
    fn connection_ids_round_trip_through_text() {
        let id = ConnectionId::new();
        assert_eq!(id.to_string().parse::<ConnectionId>().ok(), Some(id));
    }
}
