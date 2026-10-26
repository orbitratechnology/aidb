//! Driver errors turned into [`DbError`]s with actionable messages.
//!
//! Messages name the host, port, database or user (never the password) and say what to check.
//! Server text is included only where it is about access rules or permissions.

use std::io;

use crate::adapters::database::{DbError, DbErrorKind};

/// Where a failure happened: opening a connection, or on one that was already open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    Connect,
    Session,
}

/// What the messages refer to.
#[derive(Debug, Clone)]
pub(super) struct Endpoint {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    /// The profile names a CA certificate file.
    pub has_ca_file: bool,
}

pub(super) fn classify(error: sqlx::Error, endpoint: &Endpoint, phase: Phase) -> DbError {
    let classified = match &error {
        sqlx::Error::Database(db) => {
            let code = db.code().map(|code| code.into_owned()).unwrap_or_default();
            from_sqlstate(&code, db.message(), endpoint).with_sqlstate(code)
        }
        sqlx::Error::Io(io) => from_io(io, endpoint, phase),
        sqlx::Error::Tls(tls) => from_tls(&tls.to_string(), endpoint),
        sqlx::Error::PoolTimedOut => DbError::new(
            DbErrorKind::ConnectTimeout,
            "The database did not respond in time.",
        ),
        sqlx::Error::PoolClosed => {
            DbError::new(DbErrorKind::ConnectionLost, "The connection was closed.")
        }
        _ => DbError::new(
            DbErrorKind::Other,
            "The database driver reported an unexpected error.",
        ),
    };
    classified.with_cause(error)
}

pub(super) fn connect_timeout(endpoint: &Endpoint) -> DbError {
    DbError::new(
        DbErrorKind::ConnectTimeout,
        format!(
            "{}:{} did not answer in time. Check the host and port, and that a firewall is not \
             dropping the connection.",
            endpoint.host, endpoint.port
        ),
    )
}

fn from_io(io: &io::Error, endpoint: &Endpoint, phase: Phase) -> DbError {
    let text = io.to_string();
    // The TLS library reports certificate problems as I/O errors carrying its own error.
    if io.kind() == io::ErrorKind::InvalidData
        && (text.contains("certificate") || text.contains("TLS") || text.contains("tls"))
    {
        return from_tls(&text, endpoint);
    }
    if io.kind() == io::ErrorKind::NotFound && endpoint.has_ca_file {
        return DbError::new(
            DbErrorKind::Tls,
            "The CA certificate file could not be found. Check its path in the connection.",
        );
    }
    let address = format!("{}:{}", endpoint.host, endpoint.port);
    if phase == Phase::Session {
        return DbError::new(
            DbErrorKind::ConnectionLost,
            "The connection to the database was lost.",
        );
    }
    let message = match io.kind() {
        io::ErrorKind::ConnectionRefused => format!(
            "Nothing is accepting connections at {address}. Check the port and that PostgreSQL is \
             running."
        ),
        io::ErrorKind::TimedOut => format!("{address} did not answer in time."),
        _ if text.contains("lookup") || text.contains("resolve") => format!(
            "The host {} could not be found. Check its spelling and your network.",
            endpoint.host
        ),
        _ => format!("Could not reach {address}. Check the host, port and your network."),
    };
    DbError::new(DbErrorKind::Unreachable, message)
}

/// Classifies a TLS failure from the driver's description of it.
pub(super) fn from_tls(text: &str, endpoint: &Endpoint) -> DbError {
    let message = if text.contains("does not support TLS") {
        "The server does not accept TLS connections. Ask its administrator to enable TLS, or turn \
         TLS off for this connection if the network is trusted."
            .to_owned()
    } else if text.contains("NotValidForName") || text.contains("not valid for name") {
        format!(
            "The server's certificate is not valid for {}. Connect using a name listed on the \
             certificate, or use verify-ca.",
            endpoint.host
        )
    } else if text.contains("UnknownIssuer") || text.contains("unknown issuer") {
        "The server's certificate is not signed by a trusted authority. Add the CA certificate \
         to this connection, or check that you are reaching the right server."
            .to_owned()
    } else if text.contains("Expired") || text.contains("expired") {
        "The server's certificate has expired.".to_owned()
    } else if text.contains("No such file") || text.contains("os error 2") {
        "The CA certificate file could not be found.".to_owned()
    } else {
        "A secure (TLS) connection could not be set up with the server.".to_owned()
    };
    DbError::new(DbErrorKind::Tls, message)
}

/// Classifies a server error by its SQLSTATE code.
pub(super) fn from_sqlstate(code: &str, server_message: &str, endpoint: &Endpoint) -> DbError {
    use DbErrorKind as Kind;
    let (kind, message) = match code {
        "28P01" => (
            Kind::Authentication,
            format!(
                "The server rejected the password for user {}.",
                endpoint.username
            ),
        ),
        // Covers unknown roles and pg_hba.conf refusals; the server's text says which.
        "28000" => (
            Kind::Rejected,
            format!("The server refused the connection: {server_message}."),
        ),
        "3D000" => (
            Kind::DatabaseNotFound,
            format!(
                "The database {} does not exist on this server.",
                endpoint.database
            ),
        ),
        "42501" => (
            Kind::PermissionDenied,
            format!("The database refused this: {server_message}."),
        ),
        "25006" => (
            Kind::ReadOnly,
            "This connection is read-only, so the database refused the change.".to_owned(),
        ),
        "57014" if server_message.contains("statement timeout") => (
            Kind::StatementTimeout,
            "The statement ran longer than the statement timeout and was stopped.".to_owned(),
        ),
        "57014" => (Kind::Cancelled, "The statement was cancelled.".to_owned()),
        "55P03" => (
            Kind::LockTimeout,
            "The statement waited longer than the lock timeout and was stopped.".to_owned(),
        ),
        "53300" => (
            Kind::ServerUnavailable,
            "The server has no free connection slots. Try again shortly.".to_owned(),
        ),
        "57P03" => (
            Kind::ServerUnavailable,
            "The server is starting up or shutting down. Try again shortly.".to_owned(),
        ),
        "57P01" | "57P02" => (
            Kind::ConnectionLost,
            "The server ended the connection.".to_owned(),
        ),
        other if other.starts_with("08") => (
            Kind::ConnectionLost,
            "The connection to the database failed.".to_owned(),
        ),
        other => (
            Kind::Other,
            format!("The database reported an error (SQLSTATE {other})."),
        ),
    };
    DbError::new(kind, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint() -> Endpoint {
        Endpoint {
            host: "db.internal".into(),
            port: 5432,
            database: "orders".into(),
            username: "reader".into(),
            has_ca_file: true,
        }
    }

    #[test]
    fn sqlstates_map_to_kinds() {
        let cases = [
            (
                "28P01",
                "password authentication failed",
                DbErrorKind::Authentication,
            ),
            ("28000", "no pg_hba.conf entry", DbErrorKind::Rejected),
            (
                "3D000",
                "database does not exist",
                DbErrorKind::DatabaseNotFound,
            ),
            (
                "42501",
                "permission denied for table t",
                DbErrorKind::PermissionDenied,
            ),
            (
                "25006",
                "cannot execute INSERT in a read-only transaction",
                DbErrorKind::ReadOnly,
            ),
            (
                "57014",
                "canceling statement due to statement timeout",
                DbErrorKind::StatementTimeout,
            ),
            (
                "57014",
                "canceling statement due to user request",
                DbErrorKind::Cancelled,
            ),
            (
                "55P03",
                "canceling statement due to lock timeout",
                DbErrorKind::LockTimeout,
            ),
            (
                "53300",
                "too many connections",
                DbErrorKind::ServerUnavailable,
            ),
            ("08006", "connection failure", DbErrorKind::ConnectionLost),
            ("42601", "syntax error", DbErrorKind::Other),
        ];
        for (code, message, kind) in cases {
            assert_eq!(
                from_sqlstate(code, message, &endpoint()).kind(),
                kind,
                "{code}"
            );
        }
    }

    #[test]
    fn messages_name_what_to_check() {
        assert_eq!(
            from_sqlstate("28P01", "", &endpoint()).message(),
            "The server rejected the password for user reader."
        );
        assert_eq!(
            from_sqlstate("3D000", "", &endpoint()).message(),
            "The database orders does not exist on this server."
        );
        let refused = from_io(
            &io::Error::from(io::ErrorKind::ConnectionRefused),
            &endpoint(),
            Phase::Connect,
        );
        assert_eq!(refused.kind(), DbErrorKind::Unreachable);
        assert!(refused.message().contains("db.internal:5432"));
    }

    #[test]
    fn io_errors_on_open_sessions_are_lost_connections() {
        let error = from_io(
            &io::Error::from(io::ErrorKind::ConnectionReset),
            &endpoint(),
            Phase::Session,
        );

        assert_eq!(error.kind(), DbErrorKind::ConnectionLost);
    }

    #[test]
    fn certificate_errors_inside_io_errors_are_tls_failures() {
        let untrusted = io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid peer certificate: UnknownIssuer",
        );
        let missing_ca = io::Error::from(io::ErrorKind::NotFound);

        let untrusted = from_io(&untrusted, &endpoint(), Phase::Connect);
        let missing_ca = from_io(&missing_ca, &endpoint(), Phase::Connect);

        assert_eq!(untrusted.kind(), DbErrorKind::Tls);
        assert!(untrusted.message().contains("trusted authority"));
        assert_eq!(missing_ca.kind(), DbErrorKind::Tls);
        assert!(missing_ca.message().contains("CA certificate file"));
    }

    #[test]
    fn tls_failures_explain_the_certificate_problem() {
        let unknown = from_tls("invalid peer certificate: UnknownIssuer", &endpoint());
        let name = from_tls("invalid peer certificate: NotValidForName", &endpoint());
        let plain = from_tls("server does not support TLS", &endpoint());

        assert!(
            unknown
                .message()
                .contains("not signed by a trusted authority")
        );
        assert!(name.message().contains("not valid for db.internal"));
        assert!(plain.message().contains("does not accept TLS"));
        for error in [unknown, name, plain] {
            assert_eq!(error.kind(), DbErrorKind::Tls);
        }
    }
}
