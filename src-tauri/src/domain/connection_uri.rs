//! PostgreSQL connection URIs as a convenience input for the connection form.
//!
//! The URI is parsed into separate fields and never kept or logged as a whole: it can carry a
//! password. Errors describe what is wrong without repeating any part of the input.

use percent_encoding::percent_decode_str;
use secrecy::{ExposeSecret, SecretString};
use serde::{Serialize, Serializer};
use thiserror::Error;
use url::{Host, Url};

use super::connections::SslMode;
use crate::errors::AppError;

const DEFAULT_PORT: u16 = 5432;

/// Fields read from a URI. Anything the URI leaves out is `None`, so the form keeps its value.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ParsedConnectionUri {
    pub host: String,
    pub port: u16,
    pub database: Option<String>,
    pub username: Option<String>,
    pub ssl_mode: Option<SslMode>,
    pub ca_cert_path: Option<String>,
    pub connect_timeout_secs: Option<u32>,
    /// Sent back once so the form can fill its password field; never stored with the profile.
    pub password: Option<FormPassword>,
    /// A display name built from the database and host.
    pub suggested_name: String,
    /// What was adjusted or ignored, for the user to review.
    pub notes: Vec<String>,
}

/// A password on its way to the connection form. Serializes in clear; `Debug` stays redacted.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, type = "string"))]
pub struct FormPassword(pub SecretString);

impl Serialize for FormPassword {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0.expose_secret())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UriError {
    #[error("Enter a URI that starts with postgres:// or postgresql://.")]
    NotPostgres,
    #[error("This is not a valid PostgreSQL URI.")]
    Malformed,
    #[error("The URI has no host. Socket connections are not supported.")]
    NoHost,
    #[error("The URI lists several hosts. Enter one host per connection.")]
    MultipleHosts,
    #[error("The URI has an invalid {0}.")]
    Invalid(&'static str),
    #[error(
        "The URI's sslmode is not one of disable, allow, prefer, require, verify-ca or verify-full."
    )]
    UnknownSslMode,
}

impl From<UriError> for AppError {
    fn from(error: UriError) -> Self {
        Self::invalid_input(error.to_string())
    }
}

/// Splits a `postgres://` or `postgresql://` URI into connection fields.
///
/// # Errors
///
/// A [`UriError`] that never includes the input.
pub fn parse(uri: &str) -> Result<ParsedConnectionUri, UriError> {
    let url = Url::parse(uri.trim()).map_err(|_| UriError::Malformed)?;
    if !matches!(url.scheme(), "postgres" | "postgresql") {
        return Err(UriError::NotPostgres);
    }

    let mut parsed = ParsedConnectionUri {
        host: authority_host(&url)?.unwrap_or_default(),
        port: url.port().unwrap_or(DEFAULT_PORT),
        database: non_empty(decode(url.path().trim_start_matches('/'), "database name")?),
        username: non_empty(decode(url.username(), "user name")?),
        ssl_mode: None,
        ca_cert_path: None,
        connect_timeout_secs: None,
        password: url
            .password()
            .map(|raw| decode(raw, "password"))
            .transpose()?
            .filter(|password| !password.is_empty())
            .map(|password| FormPassword(SecretString::from(password))),
        suggested_name: String::new(),
        notes: Vec::new(),
    };

    let mut ignored = Vec::new();
    for (key, value) in url.query_pairs() {
        apply_parameter(&mut parsed, &key, &value, &mut ignored)?;
    }
    if !ignored.is_empty() {
        ignored.sort_unstable();
        ignored.dedup();
        parsed.notes.push(format!(
            "Ignored unsupported options: {}.",
            ignored.join(", ")
        ));
    }
    if parsed.host.is_empty() {
        return Err(UriError::NoHost);
    }
    if parsed.host.contains(',') {
        return Err(UriError::MultipleHosts);
    }
    parsed.suggested_name = match &parsed.database {
        Some(database) => format!("{database} on {}", parsed.host),
        None => parsed.host.clone(),
    };
    Ok(parsed)
}

fn authority_host(url: &Url) -> Result<Option<String>, UriError> {
    Ok(match url.host() {
        None => None,
        Some(Host::Ipv6(address)) => Some(address.to_string()),
        Some(Host::Ipv4(address)) => Some(address.to_string()),
        Some(Host::Domain(domain)) => non_empty(decode(domain, "host")?),
    })
}

fn apply_parameter(
    parsed: &mut ParsedConnectionUri,
    key: &str,
    value: &str,
    ignored: &mut Vec<String>,
) -> Result<(), UriError> {
    match key {
        "sslmode" => {
            parsed.ssl_mode = Some(match value {
                "disable" => SslMode::Disable,
                "require" => SslMode::Require,
                "verify-ca" => SslMode::VerifyCa,
                "verify-full" => SslMode::VerifyFull,
                "allow" | "prefer" => {
                    parsed.notes.push(format!(
                        "sslmode={value} can fall back to an unencrypted connection, which aidb \
                         never does. Using require instead."
                    ));
                    SslMode::Require
                }
                _ => return Err(UriError::UnknownSslMode),
            });
        }
        "sslrootcert" => parsed.ca_cert_path = non_empty(value.to_owned()),
        "connect_timeout" => match value.parse() {
            Ok(seconds) => parsed.connect_timeout_secs = Some(seconds),
            Err(_) => return Err(UriError::Invalid("connect_timeout")),
        },
        "host" => parsed.host = value.to_owned(),
        "port" => parsed.port = value.parse().map_err(|_| UriError::Invalid("port"))?,
        "dbname" => parsed.database = non_empty(value.to_owned()),
        "user" => parsed.username = non_empty(value.to_owned()),
        "password" => {
            parsed.password =
                non_empty(value.to_owned()).map(|password| FormPassword(password.into()));
        }
        // Only the option's name is reported; its value could be a secret.
        other => ignored.push(other.to_owned()),
    }
    Ok(())
}

fn decode(raw: &str, part: &'static str) -> Result<String, UriError> {
    percent_decode_str(raw)
        .decode_utf8()
        .map(|text| text.into_owned())
        .map_err(|_| UriError::Invalid(part))
}

fn non_empty(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "s3cr3t-p@ss:w/rd";

    fn uri_with_password() -> String {
        format!(
            "postgresql://app_reader:{}@db.internal:6432/orders?sslmode=verify-ca&sslrootcert=/etc/ssl/ca.pem&connect_timeout=15",
            percent_encoding::utf8_percent_encode(SECRET, percent_encoding::NON_ALPHANUMERIC)
        )
    }

    #[test]
    fn full_uris_fill_every_field() {
        let parsed = parse(&uri_with_password()).expect("parses");

        assert_eq!(parsed.host, "db.internal");
        assert_eq!(parsed.port, 6432);
        assert_eq!(parsed.database.as_deref(), Some("orders"));
        assert_eq!(parsed.username.as_deref(), Some("app_reader"));
        assert_eq!(parsed.ssl_mode, Some(SslMode::VerifyCa));
        assert_eq!(parsed.ca_cert_path.as_deref(), Some("/etc/ssl/ca.pem"));
        assert_eq!(parsed.connect_timeout_secs, Some(15));
        assert_eq!(
            parsed.password.as_ref().map(|p| p.0.expose_secret()),
            Some(SECRET)
        );
        assert_eq!(parsed.suggested_name, "orders on db.internal");
        assert!(parsed.notes.is_empty());
    }

    #[test]
    fn minimal_uris_leave_missing_fields_empty() {
        let parsed = parse("postgres://localhost").expect("parses");

        assert_eq!(parsed.host, "localhost");
        assert_eq!(parsed.port, 5432);
        assert_eq!(parsed.database, None);
        assert_eq!(parsed.username, None);
        assert_eq!(parsed.ssl_mode, None);
        assert!(parsed.password.is_none());
        assert_eq!(parsed.suggested_name, "localhost");
    }

    #[test]
    fn ipv6_hosts_and_encoded_names_are_decoded() {
        let parsed = parse("postgres://report%20user@[2001:db8::7]/sales%2Ddata").expect("parses");

        assert_eq!(parsed.host, "2001:db8::7");
        assert_eq!(parsed.username.as_deref(), Some("report user"));
        assert_eq!(parsed.database.as_deref(), Some("sales-data"));
    }

    #[test]
    fn fallback_ssl_modes_become_require_with_a_note() {
        let parsed = parse("postgres://db.internal/orders?sslmode=prefer").expect("parses");

        assert_eq!(parsed.ssl_mode, Some(SslMode::Require));
        assert_eq!(parsed.notes.len(), 1);
    }

    #[test]
    fn unsupported_options_are_named_without_their_values() {
        let parsed = parse(
            "postgres://db.internal/orders?application_name=psql&sslpassword=hunter2&options=-c%20x",
        )
        .expect("parses");

        assert_eq!(
            parsed.notes,
            ["Ignored unsupported options: application_name, options, sslpassword."]
        );
        assert!(!format!("{parsed:?}").contains("hunter2"));
    }

    #[test]
    fn query_parameters_can_supply_connection_fields() {
        let parsed =
            parse("postgresql://db.internal?dbname=orders&user=reader&port=5433&password=pw")
                .expect("parses");

        assert_eq!(parsed.database.as_deref(), Some("orders"));
        assert_eq!(parsed.username.as_deref(), Some("reader"));
        assert_eq!(parsed.port, 5433);
        assert!(parsed.password.is_some());
    }

    #[test]
    fn errors_never_repeat_the_input() {
        let cases = [
            ("mysql://root:hunter2@db/app", UriError::NotPostgres),
            (
                "postgres:///app?user=root&password=hunter2",
                UriError::NoHost,
            ),
            (
                "postgres://root:hunter2@db1,db2/app",
                UriError::MultipleHosts,
            ),
            (
                "postgres://root:hunter2@db/app?sslmode=maybe",
                UriError::UnknownSslMode,
            ),
            (
                "postgres://root:hunter2@db/app?connect_timeout=soon",
                UriError::Invalid("connect_timeout"),
            ),
            ("not a uri hunter2", UriError::Malformed),
        ];
        for (uri, expected) in cases {
            let error = parse(uri).expect_err(uri);
            assert_eq!(error, expected, "{uri}");
            assert!(!error.to_string().contains("hunter2"));
            assert!(!AppError::from(error).message().contains("hunter2"));
        }
    }

    #[test]
    fn debug_output_redacts_the_password() {
        let parsed = parse(&uri_with_password()).expect("parses");

        assert!(!format!("{parsed:?}").contains(SECRET));
    }

    #[test]
    fn the_password_is_serialized_only_for_the_form() {
        let parsed = parse(&uri_with_password()).expect("parses");

        let value = serde_json::to_value(&parsed).expect("serializes");

        assert_eq!(value["password"], SECRET);
        assert_eq!(value["sslMode"], "verify-ca");
    }
}
