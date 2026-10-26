//! Driver options for a profile.
//!
//! Every field comes from the profile. `~/.pgpass` is never read, and a profile without a
//! password sends an empty one, so `PGPASSWORD` cannot supply credentials behind the user's back.
//! aidb's session settings are appended last, so they override anything in `PGOPTIONS`.

use std::time::Duration;

use secrecy::ExposeSecret;
use sqlx::postgres::{PgConnectOptions, PgSslMode};

use crate::adapters::database::ConnectTarget;
use crate::domain::connections::SslMode;
use crate::domain::database::SessionLimits;

/// Shown in `pg_stat_activity`, so administrators can see aidb's sessions.
pub const APPLICATION_NAME: &str = "aidb";

/// A transaction left open this long is ended by the server.
const IDLE_IN_TRANSACTION_TIMEOUT: Duration = Duration::from_secs(300);

/// Builds the driver options. Their `Debug` output includes the password, so they must never be
/// printed or logged; `PgSession` keeps them out of its own `Debug`.
pub(super) fn build(target: &ConnectTarget) -> PgConnectOptions {
    let settings = &target.settings;
    let password = target
        .password
        .as_ref()
        .map_or("", |password| password.expose_secret());
    let mut options = PgConnectOptions::new_without_pgpass()
        .host(&settings.host)
        .port(settings.port)
        .username(&settings.username)
        .database(&settings.database)
        .password(password)
        .ssl_mode(ssl_mode(settings.ssl_mode))
        .application_name(APPLICATION_NAME);
    if let Some(ca) = &settings.ca_cert_path {
        options = options.ssl_root_cert(ca);
    }
    options.options(session_settings(&target.limits))
}

pub(super) const fn ssl_mode(mode: SslMode) -> PgSslMode {
    match mode {
        SslMode::Disable => PgSslMode::Disable,
        SslMode::Require => PgSslMode::Require,
        SslMode::VerifyCa => PgSslMode::VerifyCa,
        SslMode::VerifyFull => PgSslMode::VerifyFull,
    }
}

/// Server settings applied when each connection starts. The server, not aidb, enforces them.
pub(super) fn session_settings(limits: &SessionLimits) -> [(&'static str, String); 4] {
    [
        ("statement_timeout", milliseconds(limits.statement_timeout)),
        ("lock_timeout", milliseconds(limits.lock_timeout)),
        (
            "idle_in_transaction_session_timeout",
            milliseconds(IDLE_IN_TRANSACTION_TIMEOUT),
        ),
        (
            "default_transaction_read_only",
            if limits.read_only { "on" } else { "off" }.to_owned(),
        ),
    ]
}

fn milliseconds(duration: Duration) -> String {
    format!("{}ms", duration.as_millis())
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use super::*;
    use crate::domain::connections::ConnectionDraft;

    fn target(password: Option<&str>) -> ConnectTarget {
        let mut settings = ConnectionDraft::new("Orders", "db.internal", "orders", "reader");
        settings.port = 6432;
        ConnectTarget {
            settings,
            password: password.map(|p| SecretString::from(p.to_owned())),
            limits: SessionLimits {
                connect_timeout: Duration::from_secs(10),
                statement_timeout: Duration::from_secs(30),
                lock_timeout: Duration::from_secs(5),
                read_only: true,
            },
        }
    }

    #[test]
    fn options_come_from_the_profile() {
        let options = build(&target(Some("pw")));

        assert_eq!(options.get_host(), "db.internal");
        assert_eq!(options.get_port(), 6432);
        assert_eq!(options.get_username(), "reader");
        assert_eq!(options.get_database(), Some("orders"));
        assert!(matches!(options.get_ssl_mode(), PgSslMode::VerifyFull));
        assert_eq!(options.get_application_name(), Some(APPLICATION_NAME));
    }

    #[test]
    fn session_limits_are_sent_as_server_settings_last() {
        let options = build(&target(None));
        let sent = options.get_options().expect("options are set");

        let ours = "-c statement_timeout=30000ms -c lock_timeout=5000ms \
                    -c idle_in_transaction_session_timeout=300000ms \
                    -c default_transaction_read_only=on";
        assert!(sent.ends_with(ours), "{sent}");
    }

    #[test]
    fn writable_profiles_turn_read_only_off_explicitly() {
        let mut writable = target(None);
        writable.limits.read_only = false;

        let settings = session_settings(&writable.limits);

        assert_eq!(
            settings[3],
            ("default_transaction_read_only", "off".to_owned())
        );
    }
}
