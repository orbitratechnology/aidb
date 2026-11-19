use tauri::{AppHandle, Runtime};
use tauri_plugin_opener::OpenerExt;
use url::{Host, Url};

use crate::errors::{AppError, AppResult, ErrorCode};

const MAX_URL_LENGTH: usize = 2048;

/// Opens a web page or email link in the user's default application.
///
/// Only `https`, loopback `http` and `mailto` URLs are accepted, and never with embedded
/// credentials. Everything else is rejected before the OS sees it.
#[tauri::command]
#[tracing::instrument(skip_all, err(Debug))]
pub fn open_external_url<R: Runtime>(app: AppHandle<R>, url: String) -> AppResult<()> {
    let url = validate_external_url(&url)?;
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|error| {
            AppError::new(
                ErrorCode::Unavailable,
                "The link could not be opened in your default browser.",
            )
            .with_cause(error)
            .with_retryable(false)
        })
}

fn validate_external_url(raw: &str) -> AppResult<Url> {
    if raw.len() > MAX_URL_LENGTH {
        return Err(AppError::invalid_input("The link is too long to open."));
    }
    let url = Url::parse(raw.trim())
        .map_err(|_| AppError::invalid_input("The link is not a valid URL."))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::invalid_input(
            "Links that contain credentials are not opened.",
        ));
    }
    let allowed = match url.scheme() {
        "https" => url.host().is_some(),
        "http" => is_loopback(&url),
        "mailto" => true,
        _ => false,
    };
    if allowed {
        Ok(url)
    } else {
        Err(AppError::invalid_input(
            "Only https, local http and email links can be opened.",
        ))
    }
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(ip)) => ip.is_loopback(),
        Some(Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_https_local_http_and_mailto() {
        for link in [
            "https://github.com/orbitratechnology/aidb/issues",
            "http://localhost:11434/api/tags",
            "http://127.0.0.1:8080/",
            "http://[::1]:3000/",
            "mailto:hello@example.com",
        ] {
            assert!(
                validate_external_url(link).is_ok(),
                "{link} should be accepted"
            );
        }
    }

    #[test]
    fn rejects_unsafe_schemes_remote_http_and_credentials() {
        for link in [
            "http://example.com/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "tauri://localhost/",
            "smb://server/share",
            "https://user:pass@example.com/",
            "https://token@example.com/",
            "not a url",
        ] {
            let error = validate_external_url(link).expect_err("link must be rejected");
            assert_eq!(error.code(), ErrorCode::InvalidInput, "{link}");
        }
    }

    #[test]
    fn rejects_oversized_links() {
        let link = format!("https://example.com/{}", "a".repeat(MAX_URL_LENGTH));
        assert!(validate_external_url(&link).is_err());
    }
}
