//! Saving, listing and deleting AI provider profiles, their API keys, and "Test provider".
//!
//! Every profile is validated here before it is stored. API keys go to the credential store; the
//! local database records the reference only when the key is durable (OS store). A profile that
//! would send prompts over plain HTTP to another machine is saved only with explicit consent, and
//! that rule is checked again whenever a client is built from a stored profile.

use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::{ai, run_blocking};
use crate::adapters::ai_providers::openai_compatible::{OpenAiCompatibleOptions, OutputLimitField};
use crate::adapters::ai_providers::{
    AiProvider, AnthropicProvider, OpenAiCompatibleProvider, ProviderClient, ProviderError,
    ProviderErrorKind, ProviderSettings,
};
use crate::adapters::credentials::{CredentialBackend, Credentials, SecretRef};
use crate::adapters::local_store::{LocalStore, StoreError};
use crate::domain::ai::{
    ChatRequest, ChatResponse, Message, ModelInfo, OutputFormat, ProviderKind, StructuredOutput,
    ToolInput, ToolSpec,
};
use crate::domain::providers::{
    CheckedFeatures, ProviderDraft, ProviderFeatures, ProviderId, ProviderPreset, ProviderProfile,
    limits,
};
use crate::errors::{AppError, AppResult, ErrorCode};

/// Most models returned by a test; the rest are counted.
const MAX_LISTED_MODELS: usize = 200;
/// Output allowance for test requests: room for a model that reasons before answering.
const TEST_MAX_TOKENS: u32 = 1024;

/// Where a provider's API key is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum KeyStorage {
    /// In the OS credential store; survives restarts.
    OsStore,
    /// In memory until aidb quits.
    SessionOnly,
    NotSaved,
}

/// A saved profile as the UI sees it. Never includes the key or its store reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ProviderDetails {
    pub id: ProviderId,
    #[serde(flatten)]
    pub settings: ProviderDraft,
    /// The API root requests go to: the profile's own, or the kind's default.
    pub endpoint_url: String,
    pub key: KeyStorage,
    pub key_required: bool,
    /// Plain HTTP to another machine, which the user accepted.
    pub insecure_remote: bool,
    /// What requests may rely on: checked by "Test provider", or assumed until then.
    pub features: ProviderFeatures,
    /// When "Test provider" last checked the features; `None` if they are assumed.
    pub checked_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl ProviderDetails {
    fn new(profile: ProviderProfile, credentials: &Credentials) -> Self {
        let key = if profile.secret_ref.is_some() {
            KeyStorage::OsStore
        } else if credentials.holds_in_session(&SecretRef::provider(profile.id)) {
            KeyStorage::SessionOnly
        } else {
            KeyStorage::NotSaved
        };
        Self {
            id: profile.id,
            endpoint_url: profile
                .settings
                .endpoint_url()
                .map(|url| url.to_string())
                .unwrap_or_default(),
            key,
            key_required: profile.settings.kind.requires_key(),
            insecure_remote: profile.settings.is_insecure_remote(),
            features: profile.features(),
            checked_at: profile.checked.as_ref().map(|c| c.checked_at.clone()),
            settings: profile.settings,
            created_at: profile.created_at,
            updated_at: profile.updated_at,
        }
    }
}

fn profile_error(error: StoreError, name: &str) -> AppError {
    match error {
        StoreError::Duplicate(_) => AppError::new(
            ErrorCode::Conflict,
            format!("A provider named “{name}” already exists."),
        ),
        StoreError::NotFound => AppError::not_found("The provider no longer exists."),
        other => other.into(),
    }
}

/// One saved profile.
///
/// # Errors
///
/// `not_found` for an unknown profile; `storage` if the local database cannot be read.
pub async fn get(store: &LocalStore, id: ProviderId) -> AppResult<ProviderProfile> {
    store
        .providers()
        .get(id)
        .await?
        .ok_or_else(|| AppError::not_found("The provider no longer exists."))
}

/// Every saved profile, ordered by name.
///
/// # Errors
///
/// `storage` if the local database cannot be read.
pub async fn list(
    store: &LocalStore,
    credentials: &Credentials,
) -> AppResult<Vec<ProviderDetails>> {
    Ok(store
        .providers()
        .list()
        .await?
        .into_iter()
        .map(|profile| ProviderDetails::new(profile, credentials))
        .collect())
}

/// Validates and saves a new profile.
///
/// # Errors
///
/// `invalid_input` for a bad field or an unconfirmed plain-HTTP remote endpoint, `conflict` if
/// the name is taken, `storage` if saving fails.
pub async fn create(
    store: &LocalStore,
    credentials: &Credentials,
    draft: ProviderDraft,
) -> AppResult<ProviderDetails> {
    let draft = draft.validated()?;
    let profile = store
        .providers()
        .insert(&draft)
        .await
        .map_err(|error| profile_error(error, &draft.name))?;
    tracing::info!(provider = %profile.id, kind = draft.kind.as_str(), "provider created");
    Ok(ProviderDetails::new(profile, credentials))
}

/// Validates and saves changes to a profile. Checked features are kept only while the kind,
/// endpoint and model stay the same.
///
/// # Errors
///
/// As [`create`], plus `not_found` if the profile no longer exists.
pub async fn update(
    store: &LocalStore,
    credentials: &Credentials,
    id: ProviderId,
    draft: ProviderDraft,
) -> AppResult<ProviderDetails> {
    let draft = draft.validated()?;
    let profile = store
        .providers()
        .update(id, &draft)
        .await
        .map_err(|error| profile_error(error, &draft.name))?;
    tracing::info!(provider = %id, "provider updated");
    Ok(ProviderDetails::new(profile, credentials))
}

/// Deletes a profile and its API key.
///
/// # Errors
///
/// `not_found` if the profile does not exist, `credential_store` or `storage` on failure.
pub async fn delete(
    store: &LocalStore,
    credentials: &Credentials,
    id: ProviderId,
) -> AppResult<()> {
    if !store.providers().delete(id).await? {
        return Err(AppError::not_found("The provider no longer exists."));
    }
    super::settings::forget_provider(store, id).await?;
    let (credentials, key) = (credentials.clone(), SecretRef::provider(id));
    if let Err(error) = run_blocking(move || credentials.forget(&key)).await? {
        // The profile is gone; a leftover key is reported in the log, not to the user.
        tracing::warn!(provider = %id, error = %error, "could not remove a deleted provider's key");
    }
    tracing::info!(provider = %id, "provider deleted");
    Ok(())
}

/// Stores a provider's API key and records where it lives. Returns where it was stored.
///
/// # Errors
///
/// `invalid_input` for an empty, oversized or malformed key, `not_found` if the profile does not
/// exist, `credential_store` or `storage` if either store fails. On failure nothing is left
/// behind.
pub async fn save_key(
    store: &LocalStore,
    credentials: &Credentials,
    id: ProviderId,
    key: SecretString,
) -> AppResult<CredentialBackend> {
    let trimmed = key.expose_secret().trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid_input("The API key is empty."));
    }
    if trimmed.len() > limits::API_KEY_MAX_BYTES {
        return Err(AppError::invalid_input(format!(
            "The API key must be at most {} bytes.",
            limits::API_KEY_MAX_BYTES
        )));
    }
    // Keys travel in an HTTP header: printable ASCII without spaces.
    if !trimmed.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(AppError::invalid_input(
            "The API key contains characters keys never have, such as spaces.",
        ));
    }
    let key = SecretString::from(trimmed.to_owned());
    get(store, id).await?;

    let entry = SecretRef::provider(id);
    {
        let (credentials, entry) = (credentials.clone(), entry.clone());
        run_blocking(move || credentials.store(&entry, &key)).await??;
    }
    let durable = credentials.backend() == CredentialBackend::OsStore;
    if let Err(error) = store
        .providers()
        .set_secret_ref(id, durable.then(|| entry.as_str()))
        .await
    {
        let (credentials, entry) = (credentials.clone(), entry.clone());
        if let Err(cleanup) = run_blocking(move || credentials.forget(&entry)).await? {
            tracing::warn!(provider = %id, error = %cleanup, "could not remove an orphaned key");
        }
        return Err(error.into());
    }
    tracing::info!(provider = %id, backend = ?credentials.backend(), "provider key saved");
    Ok(credentials.backend())
}

/// Removes a provider's API key. Returns whether one existed.
///
/// # Errors
///
/// `credential_store` or `storage` if either store fails.
pub async fn forget_key(
    store: &LocalStore,
    credentials: &Credentials,
    id: ProviderId,
) -> AppResult<bool> {
    let existed = {
        let (credentials, entry) = (credentials.clone(), SecretRef::provider(id));
        run_blocking(move || credentials.forget(&entry)).await??
    };
    match store.providers().set_secret_ref(id, None).await {
        Ok(()) | Err(StoreError::NotFound) => {}
        Err(error) => return Err(error.into()),
    }
    tracing::info!(provider = %id, existed, "provider key forgotten");
    Ok(existed)
}

/// A client for a saved profile, with the features it may rely on.
///
/// # Errors
///
/// `not_found` for an unknown profile; `invalid_input` when a required key is missing or the
/// stored endpoint is no longer acceptable; `credential_store` if the key cannot be read.
pub async fn client(
    store: &LocalStore,
    credentials: &Credentials,
    id: ProviderId,
) -> AppResult<(ProviderProfile, ProviderClient)> {
    let profile = get(store, id).await?;
    let features = profile.features();
    let client = build_client(&profile, credentials, features).await?;
    Ok((profile, client))
}

async fn build_client(
    profile: &ProviderProfile,
    credentials: &Credentials,
    features: ProviderFeatures,
) -> AppResult<ProviderClient> {
    // The stored row is re-checked: a damaged or hand-edited profile must not bypass the rule.
    let settings = profile.settings.clone().validated()?;
    let base_url = settings.endpoint_url()?;
    let key = {
        let (credentials, entry) = (credentials.clone(), SecretRef::provider(profile.id));
        run_blocking(move || credentials.load(&entry)).await??
    };
    if key.is_none() && settings.kind.requires_key() {
        return Err(AppError::invalid_input(
            "This provider needs an API key. Save one first.",
        ));
    }
    let settings_for_adapter = ProviderSettings {
        base_url,
        api_key: key,
        idle_timeout: Duration::from_secs(u64::from(settings.request_timeout_secs)),
    };
    let client = match settings.kind.protocol() {
        ProviderKind::Anthropic => {
            ProviderClient::Anthropic(AnthropicProvider::new(settings_for_adapter)?)
        }
        ProviderKind::OpenAiCompatible => {
            ProviderClient::OpenAiCompatible(OpenAiCompatibleProvider::new(
                settings_for_adapter,
                OpenAiCompatibleOptions {
                    tool_calls: features.tool_calls,
                    structured_output: features.structured_output,
                    output_limit: if settings.kind == ProviderPreset::OpenAi {
                        OutputLimitField::MaxCompletionTokens
                    } else {
                        OutputLimitField::MaxTokens
                    },
                },
            )?)
        }
    };
    Ok(client)
}

/// One check of "Test provider".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    /// The endpoint answers and accepts the key.
    Endpoint,
    /// The configured model is offered.
    Model,
    /// The model answers a short prompt.
    Answer,
    ToolCalls,
    StructuredOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum CheckOutcome {
    Passed,
    /// Works, with a limitation the detail explains.
    Warning,
    Failed,
    /// Not checked, because an earlier check failed or the provider cannot tell.
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ProviderCheck {
    pub kind: CheckKind,
    pub outcome: CheckOutcome,
    pub detail: String,
}

/// What "Test provider" found. Requests use fixed, non-sensitive prompts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct ProviderTestReport {
    pub checks: Vec<ProviderCheck>,
    /// Models the endpoint lists, up to 200.
    pub models: Vec<ModelInfo>,
    /// How long the short answer took.
    pub answer_ms: Option<u32>,
    /// The features found and saved with the profile, when the model answered.
    pub features: Option<ProviderFeatures>,
}

struct Report {
    checks: Vec<ProviderCheck>,
}

impl Report {
    fn add(&mut self, kind: CheckKind, outcome: CheckOutcome, detail: impl Into<String>) {
        self.checks.push(ProviderCheck {
            kind,
            outcome,
            detail: detail.into(),
        });
    }

    fn skip_rest(&mut self, kinds: &[CheckKind]) {
        for &kind in kinds {
            self.add(
                kind,
                CheckOutcome::Skipped,
                "Skipped because an earlier check failed.",
            );
        }
    }
}

fn test_request(profile: &ProviderProfile, prompt: &str) -> ChatRequest {
    ChatRequest {
        model: profile.settings.model.clone(),
        system: Some(
            "You are being checked by a database client. Follow the instructions exactly.".into(),
        ),
        messages: vec![Message::user(prompt)],
        tools: Vec::new(),
        output: OutputFormat::Text,
        max_output_tokens: Some(TEST_MAX_TOKENS),
    }
}

async fn ask(
    client: &ProviderClient,
    request: &ChatRequest,
    limit: Duration,
) -> Result<ChatResponse, ProviderError> {
    ai::chat(client, request, limit, std::future::pending(), &mut |_| {}).await
}

/// Checks a saved profile: endpoint and key, model, a short answer, tool calls and structured
/// output. What it finds is saved with the profile.
///
/// # Errors
///
/// As [`client`]. Provider failures are reported as checks, not errors.
pub async fn test(
    store: &LocalStore,
    credentials: &Credentials,
    id: ProviderId,
) -> AppResult<ProviderTestReport> {
    use CheckKind as K;
    use CheckOutcome as O;

    let profile = get(store, id).await?;
    let assumed = profile.settings.kind.assumed_capabilities();
    let client = build_client(&profile, credentials, assumed).await?;
    let limit = Duration::from_secs(u64::from(profile.settings.request_timeout_secs));
    let mut report = Report { checks: Vec::new() };
    let mut models = Vec::new();

    match client.list_models().await {
        Ok(listed) => {
            report.add(
                K::Endpoint,
                O::Passed,
                match listed.len() {
                    1 => "Reached the endpoint. It lists 1 model.".to_owned(),
                    n => format!("Reached the endpoint. It lists {n} models."),
                },
            );
            if listed.iter().any(|m| m.id == profile.settings.model) {
                report.add(K::Model, O::Passed, "The model is offered.");
            } else if listed.is_empty() {
                report.add(K::Model, O::Skipped, "The endpoint lists no models.");
            } else {
                report.add(
                    K::Model,
                    O::Warning,
                    "The model is not in the endpoint's list. It may still work under another name.",
                );
            }
            models = listed;
        }
        Err(error) => match error.kind {
            ProviderErrorKind::NotFound
            | ProviderErrorKind::RateLimited
            | ProviderErrorKind::Overloaded
            | ProviderErrorKind::Server => {
                report.add(
                    K::Endpoint,
                    O::Warning,
                    format!("The model list is not available: {}", error.message),
                );
                report.add(
                    K::Model,
                    O::Skipped,
                    "The endpoint did not list its models.",
                );
            }
            _ => {
                report.add(K::Endpoint, O::Failed, error.message);
                report.skip_rest(&[K::Model, K::Answer, K::ToolCalls, K::StructuredOutput]);
                return Ok(finish(report, models, None, None));
            }
        },
    }

    let started = std::time::Instant::now();
    let answer = ask(
        &client,
        &test_request(&profile, "Reply with one word: ready"),
        limit,
    )
    .await;
    let answer_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
    match answer {
        Ok(response) if !response.message.text().trim().is_empty() => {
            report.add(
                K::Answer,
                O::Passed,
                format!(
                    "The model answered in {:.1} s.",
                    f64::from(answer_ms) / 1000.0
                ),
            );
        }
        Ok(_) => {
            report.add(K::Answer, O::Failed, "The model returned an empty answer.");
            report.skip_rest(&[K::ToolCalls, K::StructuredOutput]);
            return Ok(finish(report, models, Some(answer_ms), None));
        }
        Err(error) => {
            report.add(K::Answer, O::Failed, error.message);
            report.skip_rest(&[K::ToolCalls, K::StructuredOutput]);
            return Ok(finish(report, models, None, None));
        }
    }

    let tool_calls = check_tool_calls(&client, &profile, limit, &mut report).await;
    let structured_output =
        check_structured_output(&profile, credentials, limit, tool_calls, &mut report).await?;
    let features = ProviderFeatures {
        tool_calls,
        structured_output,
    };
    let checked = CheckedFeatures {
        features,
        checked_at: OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_default(),
    };
    store
        .providers()
        .set_checked(id, &checked)
        .await
        .map_err(|e| profile_error(e, ""))?;
    tracing::info!(provider = %id, ?features, "provider checked");
    Ok(finish(report, models, Some(answer_ms), Some(features)))
}

fn finish(
    report: Report,
    mut models: Vec<ModelInfo>,
    answer_ms: Option<u32>,
    features: Option<ProviderFeatures>,
) -> ProviderTestReport {
    models.truncate(MAX_LISTED_MODELS);
    ProviderTestReport {
        checks: report.checks,
        models,
        answer_ms,
        features,
    }
}

/// Whether the server accepts tools. A model that answers without calling is a warning: the
/// server supports tools, but that model may not use them reliably.
async fn check_tool_calls(
    client: &ProviderClient,
    profile: &ProviderProfile,
    limit: Duration,
    report: &mut Report,
) -> bool {
    let mut request = test_request(
        profile,
        "Call the report_status tool with status \"ok\". Do not answer in text.",
    );
    request.tools = vec![ToolSpec {
        name: "report_status".into(),
        description: "Reports the status of the check. Call it when asked to report a status."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {"status": {"type": "string", "enum": ["ok"]}},
            "required": ["status"],
            "additionalProperties": false,
        }),
    }];
    match ask(client, &request, limit).await {
        Ok(response) => {
            let called = response.runnable_tool_calls().any(|call| {
                call.name == "report_status"
                    && matches!(&call.input, ToolInput::Json(input) if input["status"] == "ok")
            });
            if called {
                report.add(
                    CheckKind::ToolCalls,
                    CheckOutcome::Passed,
                    "The model called a tool correctly.",
                );
            } else {
                report.add(
                    CheckKind::ToolCalls,
                    CheckOutcome::Warning,
                    "The endpoint accepts tools, but the model did not call one as asked. The agent may work poorly with this model.",
                );
            }
            true
        }
        Err(error) if error.kind == ProviderErrorKind::InvalidRequest => {
            report.add(
                CheckKind::ToolCalls,
                CheckOutcome::Failed,
                format!("The endpoint does not accept tools: {}", error.message),
            );
            false
        }
        Err(error) => {
            report.add(CheckKind::ToolCalls, CheckOutcome::Failed, error.message);
            false
        }
    }
}

/// The strongest structured-output mode that works: a JSON schema, JSON mode, or instructions.
async fn check_structured_output(
    profile: &ProviderProfile,
    credentials: &Credentials,
    limit: Duration,
    tool_calls: bool,
    report: &mut Report,
) -> AppResult<StructuredOutput> {
    let mut request = test_request(profile, "Report the status \"ok\" and the number 42.");
    request.output = OutputFormat::Json {
        name: "status_report".into(),
        schema: json!({
            "type": "object",
            "properties": {"status": {"type": "string"}, "number": {"type": "integer"}},
            "required": ["status", "number"],
            "additionalProperties": false,
        }),
    };
    let modes: &[StructuredOutput] = match profile.settings.kind.protocol() {
        ProviderKind::Anthropic => &[StructuredOutput::JsonSchema],
        ProviderKind::OpenAiCompatible => {
            &[StructuredOutput::JsonSchema, StructuredOutput::JsonObject]
        }
    };
    let mut last_problem = String::new();
    for &mode in modes {
        let features = ProviderFeatures {
            tool_calls,
            structured_output: mode,
        };
        let client = build_client(profile, credentials, features).await?;
        match ask(&client, &request, limit).await {
            Ok(response) => {
                let parsed: Option<Value> =
                    serde_json::from_str(response.message.text().trim()).ok();
                if parsed.as_ref().is_some_and(|v| v["number"] == 42) {
                    let (outcome, detail) = match mode {
                        StructuredOutput::JsonSchema => {
                            (CheckOutcome::Passed, "Answers follow a JSON schema.")
                        }
                        _ => (
                            CheckOutcome::Warning,
                            "Answers are JSON, but the endpoint does not enforce a schema; they are checked after the fact.",
                        ),
                    };
                    report.add(CheckKind::StructuredOutput, outcome, detail);
                    return Ok(mode);
                }
                last_problem = "The answer did not follow the requested JSON.".into();
            }
            Err(error) if error.kind == ProviderErrorKind::InvalidRequest => {
                last_problem = error.message;
            }
            Err(error) => {
                report.add(
                    CheckKind::StructuredOutput,
                    CheckOutcome::Failed,
                    error.message,
                );
                return Ok(StructuredOutput::Instructions);
            }
        }
    }
    report.add(
        CheckKind::StructuredOutput,
        CheckOutcome::Warning,
        format!(
            "No JSON mode worked ({}). The schema will be given in the instructions and answers checked after the fact.",
            last_problem.trim_end_matches('.')
        ),
    );
    Ok(StructuredOutput::Instructions)
}

#[cfg(test)]
mod tests;
