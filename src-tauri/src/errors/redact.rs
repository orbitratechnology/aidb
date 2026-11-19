//! Scrubs secrets out of free text before it is logged, stored or shown.
//!
//! Driver and provider messages are untrusted text: they can echo connection strings, headers
//! or keys. Pattern-based redaction is defense in depth. Code must still never put a secret into
//! a message on purpose.

use std::borrow::Cow;
use std::sync::LazyLock;

use regex::Regex;

/// Placeholder that replaces a redacted value.
pub const REDACTED: &str = "[redacted]";

struct Rule {
    pattern: Regex,
    replacement: &'static str,
}

impl Rule {
    fn new(pattern: &str, replacement: &'static str) -> Self {
        Self {
            // The patterns are constants covered by tests; failing to compile one is a bug.
            pattern: Regex::new(pattern).expect("redaction pattern must compile"),
            replacement,
        }
    }
}

// Order matters: whole header values go before the generic key/value rule.
static RULES: LazyLock<[Rule; 5]> = LazyLock::new(|| {
    [
        // `Authorization: Bearer abc` — hide the whole header value.
        Rule::new(
            r"(?i)\b(authorization)(\s*[=:]\s*)[^\r\n,;]+",
            "${1}${2}[redacted]",
        ),
        // Bare `Bearer abc` / `Basic abc` credentials.
        Rule::new(
            r"(?i)\b(bearer|basic)\s+[a-z0-9._~+/=-]+",
            "${1} [redacted]",
        ),
        // `scheme://user:password@host` — keep the user and host, hide the password.
        Rule::new(
            r"(?i)\b([a-z][a-z0-9+.-]*://[^\s:/@]+):[^\s@/]+@",
            "${1}:[redacted]@",
        ),
        // `password=...`, `api_key: "..."` and similar pairs, quoted or not.
        Rule::new(
            r#"(?i)\b(password|passwd|pwd|secret|client[_-]?secret|token|access[_-]?token|refresh[_-]?token|api[_-]?key|access[_-]?key|private[_-]?key)\b(["']?\s*[=:]\s*)("[^"]*"|'[^']*'|[^\s,;&]+)"#,
            "${1}${2}[redacted]",
        ),
        // Well-known API key formats (OpenAI/Anthropic `sk-...`, Google `AIza...`).
        Rule::new(
            r"\b(?:sk-[A-Za-z0-9_-]{16,}|AIza[0-9A-Za-z_-]{35})\b",
            "[redacted]",
        ),
    ]
});

/// Returns `text` with credentials, tokens and keys replaced by [`REDACTED`].
///
/// Borrows the input when nothing needs redacting.
#[must_use]
pub fn redact(text: &str) -> Cow<'_, str> {
    let mut out = Cow::Borrowed(text);
    for rule in RULES.iter() {
        if rule.pattern.is_match(&out) {
            out = Cow::Owned(
                rule.pattern
                    .replace_all(&out, rule.replacement)
                    .into_owned(),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_password_in_connection_uri() {
        assert_eq!(
            redact("cannot reach postgres://alice:s3cr3t@db.internal:5432/app"),
            "cannot reach postgres://alice:[redacted]@db.internal:5432/app"
        );
    }

    #[test]
    fn hides_key_value_secrets_in_every_quoting_style() {
        assert_eq!(
            redact("host=db password=hunter2 user=bob"),
            "host=db password=[redacted] user=bob"
        );
        assert_eq!(
            redact(r#"{"api_key": "abc123", "model": "m"}"#),
            r#"{"api_key": [redacted], "model": "m"}"#
        );
        assert_eq!(redact("PWD='x y z'"), "PWD=[redacted]");
        assert_eq!(redact("access-token:abc"), "access-token:[redacted]");
    }

    #[test]
    fn hides_authorization_headers_and_bearer_tokens() {
        assert_eq!(
            redact("Authorization: Bearer eyJhbGciOi.abc.def"),
            "Authorization: [redacted]"
        );
        assert_eq!(
            redact("sent bearer abc.def-123 upstream"),
            "sent bearer [redacted] upstream"
        );
    }

    #[test]
    fn hides_well_known_api_key_formats() {
        let text = "key sk-ant-api03-AbCdEfGhIjKlMnOpQrStUv was rejected";
        assert_eq!(redact(text), "key [redacted] was rejected");
    }

    #[test]
    fn keeps_ordinary_database_messages_intact() {
        let text = r#"password authentication failed for user "alice""#;
        assert!(matches!(redact(text), Cow::Borrowed(_)));
        assert!(matches!(
            redact("relation \"orders\" does not exist"),
            Cow::Borrowed(_)
        ));
    }
}
