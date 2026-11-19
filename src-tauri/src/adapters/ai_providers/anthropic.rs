//! Anthropic's Messages API (`POST /v1/messages`), streamed. Reasoning blocks come back as
//! provider data and are sent back unchanged on the next turn, as the API requires.

use std::collections::BTreeMap;

use reqwest::RequestBuilder;
use reqwest::header::HeaderValue;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value, json};

use super::{
    AiProvider, Budget, Flow, JsonBody, ProviderError, ProviderErrorKind, ProviderSettings,
    endpoint, error_detail, event_json, http_client, read_events, read_json, send,
};
use crate::domain::ai::{
    ChatEvent, ChatRequest, ChatResponse, Message, ModelInfo, OutputFormat, Part,
    ProviderCapabilities, ProviderData, ProviderKind, Role, StopReason, StructuredOutput, ToolCall,
    ToolInput, Usage,
};

/// The API version this adapter is written against.
pub const API_VERSION: &str = "2026-10-21";
/// Default API root.
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
/// Output limit when a request sets none. Every current model allows at least this much.
pub const DEFAULT_MAX_TOKENS: u32 = 32_000;
/// Most models read from the model list.
const MAX_MODELS: usize = 1000;
/// Pages of the model list read at most.
const MAX_MODEL_PAGES: usize = 10;

#[derive(Debug)]
pub struct AnthropicProvider {
    http: reqwest::Client,
    settings: ProviderSettings,
    key: HeaderValue,
}

impl AnthropicProvider {
    /// # Errors
    ///
    /// When there is no usable API key, or the HTTP client cannot be built.
    pub fn new(settings: ProviderSettings) -> Result<Self, ProviderError> {
        let key = settings
            .api_key
            .as_ref()
            .map(SecretString::expose_secret)
            .filter(|key| !key.trim().is_empty())
            .and_then(|key| HeaderValue::from_str(key.trim()).ok())
            .ok_or_else(|| {
                ProviderError::new(
                    ProviderErrorKind::Unauthorized,
                    "Anthropic needs an API key.",
                )
            })?;
        let mut key = key;
        key.set_sensitive(true);
        Ok(Self {
            http: http_client(&settings)?,
            settings,
            key,
        })
    }

    fn authorized(&self, request: RequestBuilder) -> RequestBuilder {
        request
            .header("x-api-key", self.key.clone())
            .header("anthropic-version", API_VERSION)
    }
}

/// The request body for `request`.
fn body(request: &ChatRequest) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(request.model));
    body.insert(
        "max_tokens".into(),
        json!(request.max_output_tokens.unwrap_or(DEFAULT_MAX_TOKENS)),
    );
    body.insert("stream".into(), json!(true));
    // Caches the longest stable prefix (tools, instructions, earlier turns) automatically, which
    // makes the agent's repeated turns cheaper and faster.
    body.insert("cache_control".into(), json!({"type": "ephemeral"}));
    if let Some(system) = request.system.as_deref().filter(|s| !s.is_empty()) {
        body.insert("system".into(), json!(system));
    }
    body.insert(
        "messages".into(),
        Value::Array(request.messages.iter().map(wire_message).collect()),
    );
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.input_schema,
                })
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }
    if let OutputFormat::Json { schema, .. } = &request.output {
        body.insert(
            "output_config".into(),
            json!({"format": {"type": "json_schema", "schema": schema}}),
        );
    }
    Value::Object(body)
}

fn wire_message(message: &Message) -> Value {
    let mut blocks = Vec::new();
    match message.role {
        Role::User => {
            // Tool results must come first in the message that answers the calls.
            for part in &message.parts {
                if let Part::ToolResult(result) = part {
                    let mut block = json!({
                        "type": "tool_result",
                        "tool_use_id": result.call_id,
                        "content": result.content,
                    });
                    if result.is_error {
                        block["is_error"] = json!(true);
                    }
                    blocks.push(block);
                }
            }
            for part in &message.parts {
                if let Part::Text(text) = part
                    && !text.is_empty()
                {
                    blocks.push(json!({"type": "text", "text": text}));
                }
            }
        }
        Role::Assistant => {
            for part in &message.parts {
                match part {
                    Part::Text(text) if !text.is_empty() => {
                        blocks.push(json!({"type": "text", "text": text}));
                    }
                    Part::ToolCall(call) => {
                        // A malformed call is answered with an error result; its input still has
                        // to be an object here.
                        let input = match &call.input {
                            ToolInput::Json(value) if value.is_object() => value.clone(),
                            _ => json!({}),
                        };
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": call.id,
                            "name": call.name,
                            "input": input,
                        }));
                    }
                    Part::ProviderData(data) if data.provider == ProviderKind::Anthropic => {
                        blocks.push(data.value.clone());
                    }
                    _ => {}
                }
            }
        }
    }
    let role = match message.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    json!({"role": role, "content": blocks})
}

#[derive(Debug)]
enum Block {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        input: String,
    },
    Thinking {
        thinking: String,
        signature: String,
    },
    /// Any other block (such as redacted reasoning), kept as received.
    Other(Value),
}

#[derive(Debug, Default)]
struct Accumulator {
    blocks: BTreeMap<u64, Block>,
    stop_reason: Option<String>,
    usage: Usage,
    model: Option<String>,
    finished: bool,
    budget: Budget,
}

fn stream_error(error: &Value) -> ProviderError {
    let kind = match error.get("type").and_then(Value::as_str) {
        Some("overloaded_error") => ProviderErrorKind::Overloaded,
        Some("rate_limit_error") => ProviderErrorKind::RateLimited,
        Some("invalid_request_error" | "request_too_large") => ProviderErrorKind::InvalidRequest,
        Some("authentication_error") => ProviderErrorKind::Unauthorized,
        Some("permission_error" | "billing_error") => ProviderErrorKind::Forbidden,
        Some("not_found_error") => ProviderErrorKind::NotFound,
        _ => ProviderErrorKind::Server,
    };
    let detail = error_detail(error.to_string().as_bytes()).unwrap_or_else(|| "no details".into());
    ProviderError::new(
        kind,
        format!("The provider stopped while answering. It said: {detail}"),
    )
}

impl Accumulator {
    fn handle(
        &mut self,
        event: &Value,
        on_event: &mut (dyn FnMut(ChatEvent) + Send),
    ) -> Result<Flow, ProviderError> {
        let index = event.get("index").and_then(Value::as_u64).unwrap_or(0);
        match event.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                let message = &event["message"];
                self.model = message["model"].as_str().map(str::to_owned);
                let usage = &message["usage"];
                // Cached input is reported separately from the rest.
                let input = [
                    "input_tokens",
                    "cache_creation_input_tokens",
                    "cache_read_input_tokens",
                ]
                .iter()
                .filter_map(|field| usage[field].as_u64())
                .reduce(|a, b| a + b);
                self.usage.input_tokens = input;
            }
            Some("content_block_start") => {
                let block = &event["content_block"];
                self.budget.take(block.to_string().len())?;
                let block = match block["type"].as_str() {
                    Some("text") => Block::Text(block["text"].as_str().unwrap_or("").to_owned()),
                    Some("tool_use") => {
                        self.budget.tool_call()?;
                        Block::ToolUse {
                            id: block["id"].as_str().unwrap_or("").to_owned(),
                            name: block["name"].as_str().unwrap_or("").to_owned(),
                            input: String::new(),
                        }
                    }
                    Some("thinking") => Block::Thinking {
                        thinking: block["thinking"].as_str().unwrap_or("").to_owned(),
                        signature: block["signature"].as_str().unwrap_or("").to_owned(),
                    },
                    _ => Block::Other(block.clone()),
                };
                self.blocks.insert(index, block);
            }
            Some("content_block_delta") => {
                let delta = &event["delta"];
                let block = self.blocks.get_mut(&index);
                match (delta["type"].as_str(), block) {
                    (Some("text_delta"), Some(Block::Text(text))) => {
                        let piece = delta["text"].as_str().unwrap_or("");
                        self.budget.take(piece.len())?;
                        text.push_str(piece);
                        if !piece.is_empty() {
                            on_event(ChatEvent::Text(piece.to_owned()));
                        }
                    }
                    (Some("input_json_delta"), Some(Block::ToolUse { input, .. })) => {
                        let piece = delta["partial_json"].as_str().unwrap_or("");
                        self.budget.take(piece.len())?;
                        input.push_str(piece);
                    }
                    (Some("thinking_delta"), Some(Block::Thinking { thinking, .. })) => {
                        let piece = delta["thinking"].as_str().unwrap_or("");
                        self.budget.take(piece.len())?;
                        thinking.push_str(piece);
                    }
                    (Some("signature_delta"), Some(Block::Thinking { signature, .. })) => {
                        let piece = delta["signature"].as_str().unwrap_or("");
                        self.budget.take(piece.len())?;
                        signature.push_str(piece);
                    }
                    // Citations and deltas for blocks this adapter does not track.
                    _ => {}
                }
            }
            Some("content_block_stop") => {
                if let Some(Block::ToolUse { id, name, input }) = self.blocks.get(&index) {
                    on_event(ChatEvent::ToolCall(tool_call(id, name, input)));
                }
            }
            Some("message_delta") => {
                if let Some(reason) = event["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(reason.to_owned());
                }
                if let Some(output) = event["usage"]["output_tokens"].as_u64() {
                    self.usage.output_tokens = Some(output);
                }
            }
            Some("message_stop") => {
                self.finished = true;
                return Ok(Flow::Done);
            }
            Some("error") => return Err(stream_error(&event["error"])),
            // `ping` and event types added later.
            _ => {}
        }
        Ok(Flow::Continue)
    }

    fn finish(self) -> Result<ChatResponse, ProviderError> {
        if !self.finished {
            return Err(ProviderError::new(
                ProviderErrorKind::Network,
                "The provider's answer ended early.",
            ));
        }
        let parts = self
            .blocks
            .into_values()
            .filter_map(|block| match block {
                Block::Text(text) if text.is_empty() => None,
                Block::Text(text) => Some(Part::Text(text)),
                Block::ToolUse { id, name, input } => {
                    Some(Part::ToolCall(tool_call(&id, &name, &input)))
                }
                Block::Thinking {
                    thinking,
                    signature,
                } => Some(Part::ProviderData(ProviderData {
                    provider: ProviderKind::Anthropic,
                    value: json!({"type": "thinking", "thinking": thinking, "signature": signature}),
                })),
                Block::Other(value) => Some(Part::ProviderData(ProviderData {
                    provider: ProviderKind::Anthropic,
                    value,
                })),
            })
            .collect();
        let stop_reason = match self.stop_reason.as_deref() {
            Some("end_turn" | "stop_sequence") => StopReason::EndTurn,
            Some("tool_use") => StopReason::ToolUse,
            Some("max_tokens" | "model_context_window_exceeded") => StopReason::MaxTokens,
            Some("refusal") => StopReason::Refusal,
            Some(other) => StopReason::Other(other.to_owned()),
            None => StopReason::Other("unknown".into()),
        };
        Ok(ChatResponse {
            message: Message {
                role: Role::Assistant,
                parts,
            },
            stop_reason,
            usage: self.usage,
            model: self.model,
        })
    }
}

fn tool_call(id: &str, name: &str, input: &str) -> ToolCall {
    let input = if input.trim().is_empty() {
        ToolInput::Json(json!({}))
    } else {
        serde_json::from_str(input)
            .map_or_else(|_| ToolInput::Malformed(input.to_owned()), ToolInput::Json)
    };
    ToolCall {
        id: id.to_owned(),
        name: name.to_owned(),
        input,
    }
}

impl AiProvider for AnthropicProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            kind: ProviderKind::Anthropic,
            streaming: true,
            tool_calls: true,
            structured_output: StructuredOutput::JsonSchema,
            cancellation: true,
        }
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        self.read_models()
            .await
            .map_err(|e| e.without_secret(self.settings.api_key.as_ref()))
    }

    async fn chat(
        &self,
        request: &ChatRequest,
        on_event: &mut (dyn FnMut(ChatEvent) + Send),
    ) -> Result<ChatResponse, ProviderError> {
        self.stream_chat(request, on_event)
            .await
            .map_err(|e| e.without_secret(self.settings.api_key.as_ref()))
    }
}

impl AnthropicProvider {
    async fn read_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let mut models = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..MAX_MODEL_PAGES {
            let mut url = endpoint(&self.settings.base_url, "v1/models");
            url.query_pairs_mut().append_pair("limit", "1000");
            if let Some(after) = &after {
                url.query_pairs_mut().append_pair("after_id", after);
            }
            let page =
                read_json(send(|| self.authorized(self.http.get(url.clone()))).await?).await?;
            let data = page["data"].as_array().ok_or_else(|| {
                ProviderError::new(
                    ProviderErrorKind::InvalidResponse,
                    "The provider's model list is not in the expected form.",
                )
            })?;
            for model in data {
                let Some(id) = model["id"].as_str() else {
                    continue;
                };
                models.push(ModelInfo {
                    id: id.to_owned(),
                    display_name: model["display_name"].as_str().map(str::to_owned),
                    context_window: model["max_input_tokens"]
                        .as_u64()
                        .and_then(|n| u32::try_from(n).ok()),
                    max_output_tokens: model["max_tokens"]
                        .as_u64()
                        .and_then(|n| u32::try_from(n).ok()),
                });
            }
            after = page["last_id"].as_str().map(str::to_owned);
            if page["has_more"].as_bool() != Some(true)
                || after.is_none()
                || models.len() >= MAX_MODELS
            {
                break;
            }
        }
        models.truncate(MAX_MODELS);
        Ok(models)
    }

    async fn stream_chat(
        &self,
        request: &ChatRequest,
        on_event: &mut (dyn FnMut(ChatEvent) + Send),
    ) -> Result<ChatResponse, ProviderError> {
        let url = endpoint(&self.settings.base_url, "v1/messages");
        let body = body(request);
        let response = send(|| {
            self.authorized(self.http.post(url.clone()))
                .json_body(&body)
        })
        .await?;
        let mut state = Accumulator::default();
        read_events(response, |event| {
            state.handle(&event_json(&event)?, on_event)
        })
        .await?;
        state.finish()
    }
}

#[cfg(test)]
mod tests;
