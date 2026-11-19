//! Driver-neutral database types: what an adapter can do, the limits every session runs under,
//! and what a server reports about itself. No I/O happens here.

use std::fmt;
use std::ops::RangeInclusive;
use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::connections::{ConnectionDraft, Engine};
use super::settings::SafetySettings;

/// What an adapter supports. Features check these before offering an action, so a future
/// engine without, say, cancellation gets a disabled control instead of a broken one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub engine: Engine,
    /// A running statement can be stopped from outside its session.
    pub cancel_statements: bool,
    /// The server enforces a time limit on each statement.
    pub statement_timeout: bool,
    /// The server gives up waiting for a lock after a time limit.
    pub lock_timeout: bool,
    /// The server itself rejects writes in a read-only session.
    pub read_only_sessions: bool,
    /// Schema changes run inside transactions and can be rolled back.
    pub transactional_ddl: bool,
    pub tls: bool,
}

/// Limits applied to every session of a connection, from the profile and Safety settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionLimits {
    pub connect_timeout: Duration,
    pub statement_timeout: Duration,
    pub lock_timeout: Duration,
    /// Every transaction is read-only unless the profile allows writes.
    pub read_only: bool,
}

impl SessionLimits {
    #[must_use]
    pub fn new(profile: &ConnectionDraft, safety: &SafetySettings) -> Self {
        Self {
            connect_timeout: Duration::from_secs(u64::from(profile.connect_timeout_secs)),
            statement_timeout: Duration::from_secs(u64::from(safety.statement_timeout_secs)),
            lock_timeout: Duration::from_secs(u64::from(safety.lock_timeout_secs)),
            read_only: profile.read_only,
        }
    }
}

/// PostgreSQL major versions aidb is built and tested against (as `server_version_num`).
pub const SUPPORTED_POSTGRES_VERSIONS: RangeInclusive<u32> = 150_000..=189_999;

/// Facts a server reports after a connection opens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub engine: Engine,
    /// Version as the server prints it, such as `16.4`.
    pub version: String,
    /// Numeric version, such as `160004`.
    pub version_number: u32,
    /// Within the versions aidb is tested against.
    pub supported_version: bool,
    pub user: String,
    pub database: String,
    /// The connection is encrypted with TLS.
    pub encrypted: bool,
    pub tls_version: Option<String>,
    /// The session rejects writes (aidb's read-only setting).
    pub read_only: bool,
    /// The server is a standby, so it rejects writes whatever the session allows.
    pub in_recovery: bool,
}

/// Identifies one running database operation so it can be cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, type = "string"))]
pub struct OperationId(Uuid);

impl OperationId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for OperationId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for OperationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.hyphenated().fmt(f)
    }
}

impl FromStr for OperationId {
    type Err = uuid::Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(text).map(Self)
    }
}

impl Serialize for OperationId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for OperationId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_combine_the_profile_and_safety_settings() {
        let mut profile = ConnectionDraft::new("Orders", "db.internal", "orders", "reader");
        profile.connect_timeout_secs = 7;
        let safety = SafetySettings {
            statement_timeout_secs: 45,
            lock_timeout_secs: 3,
            ..SafetySettings::default()
        };

        let limits = SessionLimits::new(&profile, &safety);

        assert_eq!(limits.connect_timeout, Duration::from_secs(7));
        assert_eq!(limits.statement_timeout, Duration::from_secs(45));
        assert_eq!(limits.lock_timeout, Duration::from_secs(3));
        assert!(limits.read_only, "profiles are read-only by default");
    }

    #[test]
    fn supported_versions_cover_postgresql_15_to_18() {
        for supported in [150_000, 160_004, 170_002, 180_000] {
            assert!(SUPPORTED_POSTGRES_VERSIONS.contains(&supported));
        }
        for unsupported in [140_011, 190_000] {
            assert!(!SUPPORTED_POSTGRES_VERSIONS.contains(&unsupported));
        }
    }

    #[test]
    fn operation_ids_round_trip_as_text() {
        let id = OperationId::new();
        let json = serde_json::to_value(id).expect("serializes");

        assert_eq!(serde_json::from_value::<OperationId>(json).ok(), Some(id));
    }
}
