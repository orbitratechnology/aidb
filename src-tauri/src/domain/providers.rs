//! AI provider profiles: which model service the agent may use, and how to reach it.
//!
//! A profile never holds the API key: `secret_ref` names an entry in the OS credential store.
//! Endpoints are trusted only over HTTPS, except this machine (such as a local Ollama), unless
//! the user explicitly acknowledges a plain-HTTP connection to another machine.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

use super::ai::{ProviderKind, StructuredOutput};
use super::connections::is_loopback_host;
use crate::errors::AppError;

/// Stable identifier of a saved provider profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, type = "string"))]
pub struct ProviderId(Uuid);

impl ProviderId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ProviderId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.hyphenated().fmt(f)
    }
}

impl FromStr for ProviderId {
    type Err = uuid::Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(text).map(Self)
    }
}

impl Serialize for ProviderId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ProviderId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// The kinds of provider a profile can be. Each has its protocol and sensible defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
pub enum ProviderPreset {
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "openai")]
    OpenAi,
    /// A local Ollama server.
    #[serde(rename = "ollama")]
    Ollama,
    /// Any other server that speaks the OpenAI Chat Completions protocol.
    #[serde(rename = "openai_compatible")]
    OpenAiCompatible,
}

impl ProviderPreset {
    /// Wire name, shared with the local store.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::Ollama => "ollama",
            Self::OpenAiCompatible => "openai_compatible",
        }
    }

    #[must_use]
    pub const fn protocol(self) -> ProviderKind {
        match self {
            Self::Anthropic => ProviderKind::Anthropic,
            Self::OpenAi | Self::Ollama | Self::OpenAiCompatible => ProviderKind::OpenAiCompatible,
        }
    }

    /// The endpoint used when the profile does not set one. A generic server has none.
    #[must_use]
    pub const fn default_endpoint(self) -> Option<&'static str> {
        match self {
            Self::Anthropic => Some("https://api.anthropic.com"),
            Self::OpenAi => Some("https://api.openai.com/v1"),
            Self::Ollama => Some("http://localhost:11434/v1"),
            Self::OpenAiCompatible => None,
        }
    }

    /// Whether requests need an API key. A generic server may or may not; Ollama does not.
    #[must_use]
    pub const fn requires_key(self) -> bool {
        matches!(self, Self::Anthropic | Self::OpenAi)
    }

    /// What a profile can do before "Test provider" has checked it.
    #[must_use]
    pub const fn assumed_capabilities(self) -> ProviderFeatures {
        ProviderFeatures {
            tool_calls: true,
            structured_output: StructuredOutput::JsonSchema,
        }
    }
}

impl FromStr for ProviderPreset {
    type Err = UnknownPreset;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "anthropic" => Ok(Self::Anthropic),
            "openai" => Ok(Self::OpenAi),
            "ollama" => Ok(Self::Ollama),
            "openai_compatible" => Ok(Self::OpenAiCompatible),
            other => Err(UnknownPreset(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unknown provider kind `{0}`")]
pub struct UnknownPreset(String);

/// What a provider was found to support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ProviderFeatures {
    pub tool_calls: bool,
    pub structured_output: StructuredOutput,
}

/// The result of the last "Test provider", kept with the profile until its endpoint or model
/// changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckedFeatures {
    #[serde(flatten)]
    pub features: ProviderFeatures,
    pub checked_at: String,
}

/// Limits shared with the local store's CHECK constraints.
pub mod limits {
    pub const NAME_MAX: usize = 100;
    pub const MODEL_MAX: usize = 200;
    pub const ENDPOINT_MAX: usize = 2048;
    pub const REQUEST_TIMEOUT_SECS: std::ops::RangeInclusive<u32> = 5..=600;
    pub const CONTEXT_BUDGET_TOKENS: std::ops::RangeInclusive<u32> = 1_000..=10_000_000;
    /// Longest API key accepted, in bytes; within every OS store's limit.
    pub const API_KEY_MAX_BYTES: usize = 4096;
}

/// Editable fields of a provider profile, validated before they are stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderDraft {
    pub name: String,
    pub kind: ProviderPreset,
    /// The API root. `None` uses the kind's default.
    pub endpoint: Option<String>,
    pub model: String,
    /// Longest wait for the provider to start or continue an answer.
    pub request_timeout_secs: u32,
    /// Most input tokens the agent may send in one request; `None` leaves it to the agent.
    pub context_budget_tokens: Option<u32>,
    /// The user accepted plain HTTP to another machine. Ignored (and cleared) otherwise.
    pub allow_insecure_http: bool,
}

impl ProviderDraft {
    /// A draft with the kind's defaults and a two-minute timeout.
    #[must_use]
    pub fn new(name: impl Into<String>, kind: ProviderPreset, model: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind,
            endpoint: None,
            model: model.into(),
            request_timeout_secs: 120,
            context_budget_tokens: None,
            allow_insecure_http: false,
        }
    }

    /// Trims text fields, normalizes the endpoint and checks every limit and the trust rule.
    ///
    /// # Errors
    ///
    /// The first [`ProviderDraftError`] found.
    pub fn validated(mut self) -> Result<Self, ProviderDraftError> {
        use ProviderDraftError as E;
        self.name = self.name.trim().to_owned();
        self.model = self.model.trim().to_owned();
        self.endpoint = self
            .endpoint
            .map(|endpoint| endpoint.trim().to_owned())
            .filter(|endpoint| !endpoint.is_empty());

        check_text(&self.name, "Name", limits::NAME_MAX)?;
        check_text(&self.model, "Model", limits::MODEL_MAX)?;
        let endpoint = match (&self.endpoint, self.kind.default_endpoint()) {
            (Some(endpoint), _) => parse_endpoint(endpoint)?,
            (None, Some(default)) => parse_endpoint(default)?,
            (None, None) => return Err(E::MissingEndpoint),
        };
        // Store the normalized form, and nothing when it is the kind's default.
        self.endpoint = self.endpoint.as_ref().map(|_| endpoint.to_string());
        if self.endpoint.as_deref() == self.kind.default_endpoint() {
            self.endpoint = None;
        }
        if !limits::REQUEST_TIMEOUT_SECS.contains(&self.request_timeout_secs) {
            return Err(E::Invalid("The timeout must be between 5 and 600 seconds"));
        }
        if let Some(budget) = self.context_budget_tokens
            && !limits::CONTEXT_BUDGET_TOKENS.contains(&budget)
        {
            return Err(E::Invalid(
                "The context budget must be between 1,000 and 10,000,000 tokens",
            ));
        }
        if !is_insecure_remote(&endpoint) {
            self.allow_insecure_http = false;
        } else if !self.allow_insecure_http {
            return Err(E::InsecureEndpoint);
        }
        Ok(self)
    }

    /// The API root requests go to.
    ///
    /// # Errors
    ///
    /// When the stored endpoint is not a usable URL (a damaged profile).
    pub fn endpoint_url(&self) -> Result<Url, ProviderDraftError> {
        match (&self.endpoint, self.kind.default_endpoint()) {
            (Some(endpoint), _) => parse_endpoint(endpoint),
            (None, Some(default)) => parse_endpoint(default),
            (None, None) => Err(ProviderDraftError::MissingEndpoint),
        }
    }

    /// Whether prompts would cross the network unencrypted to another machine.
    #[must_use]
    pub fn is_insecure_remote(&self) -> bool {
        self.endpoint_url()
            .is_ok_and(|url| is_insecure_remote(&url))
    }
}

fn check_text(value: &str, field: &'static str, max: usize) -> Result<(), ProviderDraftError> {
    if value.is_empty() {
        Err(ProviderDraftError::Missing(field))
    } else if value.chars().count() > max {
        Err(ProviderDraftError::TooLong(field, max))
    } else if value.chars().any(char::is_control) {
        Err(ProviderDraftError::ControlCharacters(field))
    } else {
        Ok(())
    }
}

/// An `http` or `https` URL with a host and no credentials, query or fragment.
fn parse_endpoint(text: &str) -> Result<Url, ProviderDraftError> {
    use ProviderDraftError as E;
    if text.len() > limits::ENDPOINT_MAX {
        return Err(E::TooLong("Endpoint", limits::ENDPOINT_MAX));
    }
    let url = Url::parse(text).map_err(|_| {
        E::Invalid("The endpoint must be a full URL, such as https://api.example.com/v1")
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(E::Invalid(
            "The endpoint must start with https:// or http://",
        ));
    }
    if url.host().is_none() {
        return Err(E::Invalid("The endpoint must name a host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(E::Invalid(
            "The endpoint must not contain credentials; save the API key separately",
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(E::Invalid("The endpoint must not have a query or fragment"));
    }
    Ok(url)
}

/// Whether `url` names this machine.
#[must_use]
pub fn is_loopback_url(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(name)) => is_loopback_host(name),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

fn is_insecure_remote(url: &Url) -> bool {
    url.scheme() == "http" && !is_loopback_url(url)
}

/// A saved provider profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderProfile {
    pub id: ProviderId,
    pub settings: ProviderDraft,
    /// Reference to the API key in the OS credential store, if one is saved.
    pub secret_ref: Option<String>,
    pub checked: Option<CheckedFeatures>,
    pub created_at: String,
    pub updated_at: String,
}

impl ProviderProfile {
    /// What requests may rely on: the last checked features, or the kind's assumptions.
    #[must_use]
    pub fn features(&self) -> ProviderFeatures {
        match self.settings.kind {
            // The Messages API supports both on every current model.
            ProviderPreset::Anthropic => ProviderPreset::Anthropic.assumed_capabilities(),
            kind => self
                .checked
                .as_ref()
                .map_or_else(|| kind.assumed_capabilities(), |c| c.features),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProviderDraftError {
    #[error("{0} is required")]
    Missing(&'static str),
    #[error("{0} must be at most {1} characters")]
    TooLong(&'static str, usize),
    #[error("{0} must not contain control characters")]
    ControlCharacters(&'static str),
    #[error("{0}")]
    Invalid(&'static str),
    #[error("An endpoint is required for an OpenAI-compatible server")]
    MissingEndpoint,
    #[error(
        "The endpoint uses plain HTTP to another machine, so prompts and the API key would \
         cross the network unencrypted. Confirm this to save the provider"
    )]
    InsecureEndpoint,
}

impl From<ProviderDraftError> for AppError {
    fn from(error: ProviderDraftError) -> Self {
        Self::invalid_input(format!("{error}."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(kind: ProviderPreset, endpoint: Option<&str>) -> ProviderDraft {
        ProviderDraft {
            endpoint: endpoint.map(str::to_owned),
            ..ProviderDraft::new(" Local ", kind, " qwen2.5 ")
        }
    }

    #[test]
    fn presets_have_their_defaults() {
        let valid = draft(ProviderPreset::Ollama, None)
            .validated()
            .expect("valid");
        assert_eq!(
            (valid.name.as_str(), valid.model.as_str()),
            ("Local", "qwen2.5")
        );
        assert_eq!(valid.endpoint, None);
        assert_eq!(
            valid.endpoint_url().expect("url").as_str(),
            "http://localhost:11434/v1"
        );
        assert!(!valid.is_insecure_remote());
        assert_eq!(
            draft(ProviderPreset::OpenAiCompatible, None).validated(),
            Err(ProviderDraftError::MissingEndpoint)
        );
        assert!(ProviderPreset::Anthropic.requires_key());
        assert!(!ProviderPreset::Ollama.requires_key());
        for preset in [
            ProviderPreset::Anthropic,
            ProviderPreset::OpenAi,
            ProviderPreset::Ollama,
            ProviderPreset::OpenAiCompatible,
        ] {
            assert_eq!(preset.as_str().parse::<ProviderPreset>(), Ok(preset));
            // The wire name in IPC is the same as in the store.
            assert_eq!(
                serde_json::to_value(preset).expect("serializes"),
                serde_json::json!(preset.as_str())
            );
        }
        assert!("google".parse::<ProviderPreset>().is_err());
    }

    #[test]
    fn endpoints_are_normalized_and_checked() {
        let saved = draft(
            ProviderPreset::OpenAiCompatible,
            Some(" https://llm.example.com/v1 "),
        )
        .validated()
        .expect("valid");
        assert_eq!(
            saved.endpoint.as_deref(),
            Some("https://llm.example.com/v1")
        );
        // Typing the default endpoint stores nothing.
        let default = draft(ProviderPreset::OpenAi, Some("https://api.openai.com/v1"))
            .validated()
            .expect("valid");
        assert_eq!(default.endpoint, None);

        for bad in [
            "llm.example.com",
            "ftp://llm.example.com",
            "https://user:pw@llm.example.com",
            "https://llm.example.com/v1?key=abc",
            "https://llm.example.com/#x",
            "file:///etc/passwd",
        ] {
            assert!(
                draft(ProviderPreset::OpenAiCompatible, Some(bad))
                    .validated()
                    .is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn plain_http_needs_consent_only_for_other_machines() {
        for local in [
            "http://localhost:8080/v1",
            "http://127.0.0.1:11434/v1",
            "http://[::1]:8080/v1",
            "http://models.localhost/v1",
        ] {
            let valid = draft(ProviderPreset::OpenAiCompatible, Some(local))
                .validated()
                .expect(local);
            assert!(!valid.is_insecure_remote(), "{local}");
        }

        let remote = draft(
            ProviderPreset::OpenAiCompatible,
            Some("http://10.0.0.5:8000/v1"),
        );
        assert_eq!(
            remote.clone().validated(),
            Err(ProviderDraftError::InsecureEndpoint)
        );
        let accepted = ProviderDraft {
            allow_insecure_http: true,
            ..remote
        }
        .validated()
        .expect("accepted");
        assert!(accepted.allow_insecure_http);
        assert!(accepted.is_insecure_remote());

        // Consent is cleared when it is not needed, so it never carries over silently.
        let https = ProviderDraft {
            allow_insecure_http: true,
            ..draft(
                ProviderPreset::OpenAiCompatible,
                Some("https://10.0.0.5/v1"),
            )
        }
        .validated()
        .expect("valid");
        assert!(!https.allow_insecure_http);
    }

    #[test]
    fn limits_are_enforced() {
        let mut long = draft(ProviderPreset::Ollama, None);
        long.name = "n".repeat(101);
        assert!(matches!(
            long.validated(),
            Err(ProviderDraftError::TooLong("Name", 100))
        ));
        let mut empty = draft(ProviderPreset::Ollama, None);
        empty.model = "  ".into();
        assert_eq!(empty.validated(), Err(ProviderDraftError::Missing("Model")));
        let mut timeout = draft(ProviderPreset::Ollama, None);
        timeout.request_timeout_secs = 1;
        assert!(timeout.validated().is_err());
        let mut budget = draft(ProviderPreset::Ollama, None);
        budget.context_budget_tokens = Some(10);
        assert!(budget.validated().is_err());
        let mut control = draft(ProviderPreset::Ollama, None);
        control.model = "qwen\n".into();
        // Trailing whitespace is trimmed first; an inner control character is refused.
        assert!(control.validated().is_ok());
        let mut control = draft(ProviderPreset::Ollama, None);
        control.model = "qw\u{7}en".into();
        assert!(control.validated().is_err());
    }

    #[test]
    fn features_come_from_the_last_check() {
        let mut profile = ProviderProfile {
            id: ProviderId::new(),
            settings: draft(
                ProviderPreset::OpenAiCompatible,
                Some("http://localhost/v1"),
            ),
            secret_ref: None,
            checked: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        assert!(profile.features().tool_calls);
        profile.checked = Some(CheckedFeatures {
            features: ProviderFeatures {
                tool_calls: false,
                structured_output: StructuredOutput::JsonObject,
            },
            checked_at: "2026-11-17T00:00:00Z".into(),
        });
        assert_eq!(
            profile.features().structured_output,
            StructuredOutput::JsonObject
        );
    }
}
