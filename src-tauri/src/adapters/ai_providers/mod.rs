//! AI provider clients. Each adapter turns a provider-independent [`ChatRequest`] into one
//! streamed HTTP request and the stream back into a [`ChatResponse`]. Adapters never see
//! database connections or credentials other than their own API key.
//!
//! Requests go straight from this process to the configured endpoint: TLS through rustls with
//! the operating system's trust store, no redirects followed, bounded connect and idle times,
//! and every response size-capped. Dropping a request's future closes the connection, which is
//! how a stream is cancelled.

pub mod anthropic;
pub mod openai_compatible;
pub mod sse;

use std::fmt;
use std::future::Future;
use std::sync::Once;
use std::time::Duration;

use reqwest::header::{HeaderMap, RETRY_AFTER};
use reqwest::{RequestBuilder, Response, StatusCode};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use url::Url;

use crate::domain::ai::{
    ChatEvent, ChatRequest, ChatResponse, MAX_RESPONSE_BYTES, MAX_TOOL_CALLS, ModelInfo,
    ProviderCapabilities,
};
use crate::errors::{AppError, ErrorCode, redact};
use sse::{SseDecoder, SseError, SseEvent};

pub use anthropic::AnthropicProvider;
pub use openai_compatible::OpenAiCompatibleProvider;

/// Longest wait for a TCP and TLS connection.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Largest single streamed event; a whole tool call can arrive in one.
const MAX_EVENT_BYTES: usize = 1024 * 1024;
/// Largest non-streamed body read (model lists, error bodies).
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
/// Longest provider-supplied text kept in an error message.
const MAX_DETAIL_CHARS: usize = 300;

/// A model provider. Implementations are cheap to share; one request runs per call.
pub trait AiProvider: Send + Sync {
    fn capabilities(&self) -> ProviderCapabilities;

    /// The models the endpoint offers.
    fn list_models(&self) -> impl Future<Output = Result<Vec<ModelInfo>, ProviderError>> + Send;

    /// Sends `request` and streams the answer, reporting text and finished tool calls to
    /// `on_event` as they arrive. Dropping the future cancels the request.
    fn chat(
        &self,
        request: &ChatRequest,
        on_event: &mut (dyn FnMut(ChatEvent) + Send),
    ) -> impl Future<Output = Result<ChatResponse, ProviderError>> + Send;
}

/// Where and how to reach a provider. The API key stays wrapped until it goes into a header.
#[derive(Clone)]
pub struct ProviderSettings {
    /// The API root, such as `https://api.anthropic.com` or `http://localhost:11434/v1`.
    pub base_url: Url,
    pub api_key: Option<SecretString>,
    /// Longest silence while waiting for the next part of a response.
    pub idle_timeout: Duration,
}

impl fmt::Debug for ProviderSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderSettings")
            .field("base_url", &self.base_url.as_str())
            .field("api_key", &self.api_key.as_ref().map(|_| "[redacted]"))
            .field("idle_timeout", &self.idle_timeout)
            .finish()
    }
}

/// What went wrong talking to a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorKind {
    /// The provider rejected the request as malformed or too large.
    InvalidRequest,
    /// The API key is missing, wrong or revoked.
    Unauthorized,
    /// The key works but may not do this.
    Forbidden,
    /// No such model or endpoint.
    NotFound,
    /// Too many requests, or a spending limit.
    RateLimited,
    /// The provider is overloaded or down for now.
    Overloaded,
    /// The provider failed.
    Server,
    /// The endpoint could not be reached, or the connection broke.
    Network,
    Timeout,
    Cancelled,
    /// The answer was not what the protocol promises (bad JSON, redirects, too large).
    InvalidResponse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    pub kind: ProviderErrorKind,
    /// Safe to show: written here, or the provider's own message redacted and shortened.
    pub message: String,
    /// How long the provider asked to wait before retrying.
    pub retry_after: Option<Duration>,
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ProviderError {}

impl ProviderError {
    #[must_use]
    pub fn new(kind: ProviderErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            retry_after: None,
        }
    }

    #[must_use]
    pub fn cancelled() -> Self {
        Self::new(ProviderErrorKind::Cancelled, "The request was cancelled.")
    }

    #[must_use]
    pub fn timeout() -> Self {
        Self::new(
            ProviderErrorKind::Timeout,
            "The provider took too long to answer.",
        )
    }

    fn invalid_response(message: impl Into<String>) -> Self {
        Self::new(ProviderErrorKind::InvalidResponse, message)
    }

    /// The error with every occurrence of `secret` removed, for errors carrying text the
    /// provider sent: an endpoint could echo the key back.
    #[must_use]
    pub fn without_secret(mut self, secret: Option<&SecretString>) -> Self {
        if let Some(secret) = secret.map(|s| s.expose_secret().trim())
            && !secret.is_empty()
            && self.message.contains(secret)
        {
            self.message = self.message.replace(secret, crate::errors::REDACTED);
        }
        self
    }

    /// Whether the same request may succeed later.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(
            self.kind,
            ProviderErrorKind::RateLimited
                | ProviderErrorKind::Overloaded
                | ProviderErrorKind::Server
                | ProviderErrorKind::Network
                | ProviderErrorKind::Timeout
        )
    }
}

impl From<ProviderError> for AppError {
    fn from(error: ProviderError) -> Self {
        use ProviderErrorKind as Kind;
        let code = match error.kind {
            Kind::InvalidRequest => ErrorCode::InvalidInput,
            Kind::Unauthorized | Kind::Forbidden => ErrorCode::PermissionDenied,
            Kind::NotFound => ErrorCode::NotFound,
            Kind::Timeout => ErrorCode::Timeout,
            Kind::Cancelled => ErrorCode::Cancelled,
            Kind::RateLimited
            | Kind::Overloaded
            | Kind::Server
            | Kind::Network
            | Kind::InvalidResponse => ErrorCode::Unavailable,
        };
        let retryable = error.is_retryable();
        Self::new(code, error.message.clone()).with_retryable(retryable)
    }
}

/// Installs `ring` as rustls's crypto provider, the one the PostgreSQL driver uses too.
/// reqwest's `rustls-no-provider` mode needs one installed before a client is built.
fn install_crypto_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        // An error means a provider is already installed, which is fine.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// The HTTP client every adapter uses.
fn http_client(settings: &ProviderSettings) -> Result<reqwest::Client, ProviderError> {
    install_crypto_provider();
    let mut builder = reqwest::Client::builder()
        .user_agent(concat!("aidb/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(settings.idle_timeout);
    // A local model stays local: requests to this machine never go through a proxy.
    if is_loopback(&settings.base_url) {
        builder = builder.no_proxy();
    }
    builder.build().map_err(|error| {
        tracing::warn!(error = %error, "could not build the HTTP client");
        ProviderError::new(
            ProviderErrorKind::Network,
            "The secure connection could not be set up.",
        )
    })
}

/// Whether `url` names this machine.
#[must_use]
pub fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(name)) => {
            name.eq_ignore_ascii_case("localhost")
                || name.to_ascii_lowercase().ends_with(".localhost")
        }
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// `path` under the API root, keeping the root's own path (`/v1`).
fn endpoint(base: &Url, path: &str) -> Url {
    let mut url = base.clone();
    url.set_query(None);
    url.set_fragment(None);
    let root = url.path().trim_end_matches('/').to_owned();
    url.set_path(&format!("{root}/{path}"));
    url
}

fn transport_error(error: &reqwest::Error) -> ProviderError {
    // The URL in reqwest's message names the endpoint the user configured; keys travel in
    // headers, never in URLs, so it carries no secret. The message is still redacted.
    tracing::debug!(error = %redact(&error.to_string()), "provider transport error");
    if error.is_timeout() {
        ProviderError::timeout()
    } else if error.is_connect() {
        ProviderError::new(
            ProviderErrorKind::Network,
            "The provider could not be reached. Check the address and your network.",
        )
    } else if error.is_body() || error.is_decode() {
        ProviderError::new(
            ProviderErrorKind::Network,
            "The connection to the provider broke while it was answering.",
        )
    } else {
        ProviderError::new(
            ProviderErrorKind::Network,
            "The request to the provider failed.",
        )
    }
}

/// Retries after the first attempt, as the official provider SDKs do by default.
const MAX_RETRIES: u32 = 2;
/// The first backoff; it doubles for each retry.
const FIRST_BACKOFF: Duration = Duration::from_millis(500);
/// A longer requested wait is not waited out: the error is returned instead.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(10);

/// Sends a request and returns the response if it succeeded; otherwise the provider's error.
///
/// Failures before any response arrives (connection errors, including a pooled connection the
/// server closed just as it was reused) and 408, 409, 429 and 5xx answers are retried up to
/// [`MAX_RETRIES`] times with backoff, honoring `retry-after` and `x-should-retry`. Nothing is
/// retried once a successful response has started, so a stream is never replayed.
///
/// `build` makes a fresh request for each attempt.
async fn send(build: impl Fn() -> RequestBuilder + Send) -> Result<Response, ProviderError> {
    let mut attempt = 0;
    loop {
        let (error, wait, should_retry) = match attempt_once(build()).await {
            Ok(response) => return Ok(response),
            Err(failure) => failure,
        };
        let wait = wait.unwrap_or(FIRST_BACKOFF * 2_u32.pow(attempt));
        if attempt >= MAX_RETRIES || !should_retry || wait > MAX_RETRY_WAIT {
            return Err(error);
        }
        tracing::debug!(
            attempt,
            wait_ms = wait.as_millis(),
            "retrying provider request"
        );
        tokio::time::sleep(wait).await;
        attempt += 1;
    }
}

/// One attempt. On failure: the error, the wait the provider asked for, and whether to retry.
async fn attempt_once(
    request: RequestBuilder,
) -> Result<Response, (ProviderError, Option<Duration>, bool)> {
    let mut response = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            // A timeout already waited the full idle time; anything else failed before the
            // provider answered.
            let retry = !error.is_timeout();
            return Err((transport_error(&error), None, retry));
        }
    };
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    if status.is_redirection() {
        return Err((
            ProviderError::invalid_response(
                "The endpoint redirected the request. Use the address it redirects to.",
            ),
            None,
            false,
        ));
    }
    let wait = retry_after(response.headers());
    let should_retry = match response
        .headers()
        .get("x-should-retry")
        .and_then(|v| v.to_str().ok())
    {
        Some("true") => true,
        Some("false") => false,
        _ => matches!(status.as_u16(), 408 | 409 | 429 | 500..=599),
    };
    let body = read_body(&mut response).await.unwrap_or_default();
    Err((status_error(status, &body, wait), wait, should_retry))
}

/// Reads a whole body, up to [`MAX_BODY_BYTES`].
async fn read_body(response: &mut Response) -> Result<Vec<u8>, ProviderError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| transport_error(&e))? {
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(ProviderError::invalid_response(
                "The provider's answer is too large.",
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Reads a JSON body (model lists).
async fn read_json(mut response: Response) -> Result<Value, ProviderError> {
    let body = read_body(&mut response).await?;
    serde_json::from_slice(&body)
        .map_err(|_| ProviderError::invalid_response("The provider's answer is not valid JSON."))
}

fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let seconds: u64 = headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(seconds.min(3600)))
}

/// The provider's own message from an error body, made safe to show.
fn error_detail(body: &[u8]) -> Option<String> {
    let text = match serde_json::from_slice::<Value>(body) {
        Ok(json) => {
            let error = json.get("error").unwrap_or(&json);
            error
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| error.as_str())?
                .to_owned()
        }
        // Plain-text errors are kept; HTML pages are not worth showing.
        Err(_) => {
            let text = std::str::from_utf8(body).ok()?.trim();
            if text.is_empty() || text.starts_with('<') {
                return None;
            }
            text.to_owned()
        }
    };
    let text = redact(&text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        return None;
    }
    Some(match text.char_indices().nth(MAX_DETAIL_CHARS) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text,
    })
}

fn status_error(status: StatusCode, body: &[u8], retry_after: Option<Duration>) -> ProviderError {
    use ProviderErrorKind as Kind;
    let (kind, lead) = match status.as_u16() {
        401 => (
            Kind::Unauthorized,
            "The provider did not accept the API key.",
        ),
        402 => (
            Kind::Forbidden,
            "The provider account has a billing problem.",
        ),
        403 => (Kind::Forbidden, "The API key may not do this."),
        404 => (
            Kind::NotFound,
            "The provider does not have that model or address.",
        ),
        408 => (Kind::Timeout, "The provider took too long to answer."),
        429 => (
            Kind::RateLimited,
            "The provider's rate or spending limit was reached.",
        ),
        503 | 529 => (
            Kind::Overloaded,
            "The provider is overloaded. Try again shortly.",
        ),
        500..=599 => (Kind::Server, "The provider had an internal error."),
        _ => (Kind::InvalidRequest, "The provider rejected the request."),
    };
    let message = match error_detail(body) {
        Some(detail) => format!("{lead} It said: {detail}"),
        None => format!("{lead} (HTTP {})", status.as_u16()),
    };
    ProviderError {
        kind,
        message,
        retry_after,
    }
}

/// Whether to keep reading a stream.
enum Flow {
    Continue,
    Done,
}

/// Feeds a streamed response's events to `handle` until it says it is done or the body ends.
/// Returns whether `handle` finished the stream.
async fn read_events(
    mut response: Response,
    mut handle: impl FnMut(SseEvent) -> Result<Flow, ProviderError> + Send,
) -> Result<bool, ProviderError> {
    let streams = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"));
    if !streams {
        let body = read_body(&mut response).await?;
        return Err(match error_detail(&body) {
            Some(detail) => ProviderError::invalid_response(format!(
                "The provider answered without streaming. It said: {detail}"
            )),
            None => ProviderError::invalid_response("The provider answered without streaming."),
        });
    }
    let mut decoder = SseDecoder::new(MAX_EVENT_BYTES);
    while let Some(chunk) = response.chunk().await.map_err(|e| transport_error(&e))? {
        let events = decoder.push(&chunk).map_err(|error| match error {
            SseError::TooLarge(_) => {
                ProviderError::invalid_response("The provider sent an oversized event.")
            }
            SseError::Encoding => {
                ProviderError::invalid_response("The provider's stream is not valid text.")
            }
        })?;
        for event in events {
            if let Flow::Done = handle(event)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Counts what a response accumulates, so a hostile or broken endpoint cannot exhaust memory.
#[derive(Debug, Default)]
struct Budget {
    bytes: usize,
    tool_calls: usize,
}

impl Budget {
    fn take(&mut self, bytes: usize) -> Result<(), ProviderError> {
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes > MAX_RESPONSE_BYTES {
            return Err(ProviderError::invalid_response(
                "The provider's answer is too large.",
            ));
        }
        Ok(())
    }

    fn tool_call(&mut self) -> Result<(), ProviderError> {
        self.tool_calls += 1;
        if self.tool_calls > MAX_TOOL_CALLS {
            return Err(ProviderError::invalid_response(
                "The model asked for too many tool calls at once.",
            ));
        }
        Ok(())
    }
}

/// JSON bodies without reqwest's `json` feature: `serde_json` is already a dependency.
trait JsonBody {
    fn json_body(self, body: &Value) -> Self;
}

impl JsonBody for RequestBuilder {
    fn json_body(self, body: &Value) -> Self {
        self.header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_string())
    }
}

/// Parses one streamed JSON event.
fn event_json(event: &SseEvent) -> Result<Value, ProviderError> {
    serde_json::from_str(&event.data)
        .map_err(|_| ProviderError::invalid_response("The provider sent an unreadable event."))
}

/// Either provider, chosen by a profile at run time.
#[derive(Debug)]
pub enum ProviderClient {
    OpenAiCompatible(OpenAiCompatibleProvider),
    Anthropic(AnthropicProvider),
}

impl AiProvider for ProviderClient {
    fn capabilities(&self) -> ProviderCapabilities {
        match self {
            Self::OpenAiCompatible(provider) => provider.capabilities(),
            Self::Anthropic(provider) => provider.capabilities(),
        }
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        match self {
            Self::OpenAiCompatible(provider) => provider.list_models().await,
            Self::Anthropic(provider) => provider.list_models().await,
        }
    }

    async fn chat(
        &self,
        request: &ChatRequest,
        on_event: &mut (dyn FnMut(ChatEvent) + Send),
    ) -> Result<ChatResponse, ProviderError> {
        match self {
            Self::OpenAiCompatible(provider) => provider.chat(request, on_event).await,
            Self::Anthropic(provider) => provider.chat(request, on_event).await,
        }
    }
}

#[cfg(test)]
pub(crate) mod test_server;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_keep_the_api_root_path() {
        let base = Url::parse("http://localhost:11434/v1/").expect("url");
        assert_eq!(
            endpoint(&base, "chat/completions").as_str(),
            "http://localhost:11434/v1/chat/completions"
        );
        let base = Url::parse("https://api.anthropic.com").expect("url");
        assert_eq!(
            endpoint(&base, "v1/messages").as_str(),
            "https://api.anthropic.com/v1/messages"
        );
    }

    #[test]
    fn loopback_hosts_are_recognized() {
        for (url, local) in [
            ("http://localhost:11434/v1", true),
            ("http://LOCALHOST", true),
            ("http://ollama.localhost", true),
            ("http://127.0.0.2:8080", true),
            ("http://[::1]:8080", true),
            ("http://10.0.0.5", false),
            ("https://api.anthropic.com", false),
            ("http://localhost.example.com", false),
        ] {
            assert_eq!(is_loopback(&Url::parse(url).expect("url")), local, "{url}");
        }
    }

    #[test]
    fn status_errors_are_classified_and_redacted() {
        let body = br#"{"error":{"type":"authentication_error","message":"invalid x-api-key sk-ant-api03-abcdefghijklmnopqrstuvwxyz"}}"#;
        let error = status_error(StatusCode::UNAUTHORIZED, body, None);
        assert_eq!(error.kind, ProviderErrorKind::Unauthorized);
        assert!(!error.message.contains("abcdefghij"), "{}", error.message);
        assert!(!error.is_retryable());

        let error = status_error(
            StatusCode::TOO_MANY_REQUESTS,
            br#"{"error":"slow down"}"#,
            Some(Duration::from_secs(7)),
        );
        assert_eq!(
            (error.kind, error.retry_after),
            (ProviderErrorKind::RateLimited, Some(Duration::from_secs(7)))
        );
        assert!(error.message.ends_with("It said: slow down"));
        assert!(error.is_retryable());

        let overloaded = StatusCode::from_u16(529).expect("status");
        assert_eq!(
            status_error(overloaded, b"<html>busy</html>", None).message,
            "The provider is overloaded. Try again shortly. (HTTP 529)"
        );
        assert_eq!(
            status_error(StatusCode::NOT_FOUND, b"model 'x' not found", None).kind,
            ProviderErrorKind::NotFound
        );
        let long = format!(r#"{{"message":"{}"}}"#, "word ".repeat(200));
        assert!(
            status_error(StatusCode::BAD_REQUEST, long.as_bytes(), None)
                .message
                .ends_with('…')
        );
    }

    #[test]
    fn settings_debug_hides_the_key() {
        let settings = ProviderSettings {
            base_url: Url::parse("https://api.anthropic.com").expect("url"),
            api_key: Some(SecretString::from("sk-secret-value")),
            idle_timeout: Duration::from_secs(60),
        };
        assert!(!format!("{settings:?}").contains("sk-secret"));
    }

    #[test]
    fn provider_errors_map_to_app_errors() {
        let error: AppError = ProviderError::new(ProviderErrorKind::Overloaded, "busy").into();
        assert_eq!(error.code(), ErrorCode::Unavailable);
        assert!(error.is_retryable());
        let error: AppError = ProviderError::new(ProviderErrorKind::Unauthorized, "no").into();
        assert_eq!(error.code(), ErrorCode::PermissionDenied);
        assert!(!error.is_retryable());
    }
}
