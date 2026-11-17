//! The OpenAI Chat Completions protocol (`POST /chat/completions`), which OpenAI, Ollama,
//! LM Studio, llama.cpp, vLLM and many gateways speak. Servers differ in what they support, so
//! tool calls and the structured-output mode are settings rather than assumptions.

use std::collections::BTreeMap;

use reqwest::RequestBuilder;
use secrecy::ExposeSecret;
use serde_json::{Map, Value, json};

use super::{
    AiProvider, Budget, Flow, JsonBody, ProviderError, ProviderErrorKind, ProviderSettings,
    endpoint, error_detail, event_json, http_client, read_events, read_json, send,
};
use crate::domain::ai::{
    ChatEvent, ChatRequest, ChatResponse, Message, ModelInfo, OutputFormat, Part,
    ProviderCapabilities, ProviderKind, Role, StopReason, StructuredOutput, ToolCall, ToolInput,
    Usage,
};

/// Most models read from a model list.
const MAX_MODELS: usize = 1000;

/// What the server supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenAiCompatibleOptions {
    pub tool_calls: bool,
    pub structured_output: StructuredOutput,
}

impl Default for OpenAiCompatibleOptions {
    fn default() -> Self {
        Self {
            tool_calls: true,
            structured_output: StructuredOutput::JsonSchema,
        }
    }
}

#[derive(Debug)]
pub struct OpenAiCompatibleProvider {
    http: reqwest::Client,
    settings: ProviderSettings,
    options: OpenAiCompatibleOptions,
}

impl OpenAiCompatibleProvider {
    /// # Errors
    ///
    /// When the HTTP client cannot be built.
    pub fn new(
        settings: ProviderSettings,
        options: OpenAiCompatibleOptions,
    ) -> Result<Self, ProviderError> {
        Ok(Self {
            http: http_client(&settings)?,
            settings,
            options,
        })
    }

    fn authorized(&self, request: RequestBuilder) -> RequestBuilder {
        match &self.settings.api_key {
            // `bearer_auth` marks the header sensitive, so it is never logged.
            Some(key) => request.bearer_auth(key.expose_secret()),
            None => request,
        }
    }

    fn body(&self, request: &ChatRequest) -> Result<Value, ProviderError> {
        if !request.tools.is_empty() && !self.options.tool_calls {
            return Err(ProviderError::new(
                ProviderErrorKind::InvalidRequest,
                "This provider is set up without tool calls.",
            ));
        }
        let mut messages = Vec::new();
        let schema_instructions = match (&request.output, self.options.structured_output) {
            (
                OutputFormat::Json { schema, .. },
                StructuredOutput::JsonObject | StructuredOutput::Instructions,
            ) => Some(format!(
                "Reply with only a JSON object that follows this JSON Schema, with no other text:\n{schema}"
            )),
            _ => None,
        };
        let system = [request.system.clone(), schema_instructions]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n\n");
        if !system.is_empty() {
            messages.push(json!({"role": "system", "content": system}));
        }
        for message in &request.messages {
            wire_messages(message, &mut messages);
        }

        let mut body = Map::new();
        body.insert("model".into(), json!(request.model));
        body.insert("messages".into(), Value::Array(messages));
        body.insert("stream".into(), json!(true));
        body.insert("stream_options".into(), json!({"include_usage": true}));
        if !request.tools.is_empty() {
            let tools: Vec<Value> = request
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.input_schema,
                        }
                    })
                })
                .collect();
            body.insert("tools".into(), Value::Array(tools));
        }
        if let OutputFormat::Json { name, schema } = &request.output {
            match self.options.structured_output {
                StructuredOutput::JsonSchema => {
                    body.insert(
                        "response_format".into(),
                        json!({
                            "type": "json_schema",
                            "json_schema": {"name": name, "schema": schema, "strict": true},
                        }),
                    );
                }
                StructuredOutput::JsonObject => {
                    body.insert("response_format".into(), json!({"type": "json_object"}));
                }
                StructuredOutput::Instructions => {}
            }
        }
        if let Some(max) = request.max_output_tokens {
            body.insert("max_tokens".into(), json!(max));
        }
        Ok(Value::Object(body))
    }
}

/// One conversation message as Chat Completions messages: tool results become `tool` messages,
/// which must directly follow the assistant message that called them.
fn wire_messages(message: &Message, out: &mut Vec<Value>) {
    match message.role {
        Role::User => {
            for part in &message.parts {
                if let Part::ToolResult(result) = part {
                    let content = if result.is_error {
                        format!("Error: {}", result.content)
                    } else {
                        result.content.clone()
                    };
                    out.push(json!({
                        "role": "tool",
                        "tool_call_id": result.call_id,
                        "content": content,
                    }));
                }
            }
            let text = message.text();
            if !text.is_empty() {
                out.push(json!({"role": "user", "content": text}));
            }
        }
        Role::Assistant => {
            let text = message.text();
            let calls: Vec<Value> = message
                .tool_calls()
                .map(|call| {
                    let arguments = match &call.input {
                        ToolInput::Json(value) => value.to_string(),
                        ToolInput::Malformed(raw) => raw.clone(),
                    };
                    json!({
                        "id": call.id,
                        "type": "function",
                        "function": {"name": call.name, "arguments": arguments},
                    })
                })
                .collect();
            let mut wire = Map::new();
            wire.insert("role".into(), json!("assistant"));
            wire.insert(
                "content".into(),
                if text.is_empty() {
                    Value::Null
                } else {
                    json!(text)
                },
            );
            if !calls.is_empty() {
                wire.insert("tool_calls".into(), Value::Array(calls));
            }
            out.push(Value::Object(wire));
        }
    }
}

#[derive(Debug, Default)]
struct PendingCall {
    id: Option<String>,
    name: String,
    arguments: String,
}

/// The state of one streamed answer.
#[derive(Debug, Default)]
struct Accumulator {
    text: String,
    calls: BTreeMap<u64, PendingCall>,
    finish_reason: Option<String>,
    usage: Usage,
    model: Option<String>,
    budget: Budget,
}

impl Accumulator {
    fn handle(
        &mut self,
        chunk: &Value,
        on_event: &mut (dyn FnMut(ChatEvent) + Send),
    ) -> Result<(), ProviderError> {
        if let Some(error) = chunk.get("error") {
            let detail = error_detail(error.to_string().as_bytes())
                .unwrap_or_else(|| "no details".to_owned());
            return Err(ProviderError::new(
                ProviderErrorKind::Server,
                format!("The provider failed while answering. It said: {detail}"),
            ));
        }
        if let Some(model) = chunk.get("model").and_then(Value::as_str) {
            self.model = Some(model.to_owned());
        }
        if let Some(usage) = chunk.get("usage").filter(|u| u.is_object()) {
            self.usage = Usage {
                input_tokens: usage.get("prompt_tokens").and_then(Value::as_u64),
                output_tokens: usage.get("completion_tokens").and_then(Value::as_u64),
            };
        }
        let Some(choice) = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            return Ok(());
        };
        if let Some(delta) = choice.get("delta") {
            if let Some(text) = delta.get("content").and_then(Value::as_str)
                && !text.is_empty()
            {
                self.budget.take(text.len())?;
                self.text.push_str(text);
                on_event(ChatEvent::Text(text.to_owned()));
            }
            for (position, call) in delta
                .get("tool_calls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                let index = call
                    .get("index")
                    .and_then(Value::as_u64)
                    .unwrap_or(position as u64);
                if !self.calls.contains_key(&index) {
                    self.budget.tool_call()?;
                }
                let pending = self.calls.entry(index).or_default();
                if let Some(id) = call.get("id").and_then(Value::as_str)
                    && !id.is_empty()
                {
                    pending.id = Some(id.to_owned());
                }
                let function = call.get("function");
                if let Some(name) = function.and_then(|f| f.get("name")).and_then(Value::as_str)
                    && pending.name.is_empty()
                {
                    self.budget.take(name.len())?;
                    pending.name = name.to_owned();
                }
                if let Some(arguments) = function
                    .and_then(|f| f.get("arguments"))
                    .and_then(Value::as_str)
                {
                    self.budget.take(arguments.len())?;
                    pending.arguments.push_str(arguments);
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(reason.to_owned());
        }
        Ok(())
    }

    fn finish(
        self,
        on_event: &mut (dyn FnMut(ChatEvent) + Send),
    ) -> Result<ChatResponse, ProviderError> {
        let Some(finish_reason) = self.finish_reason else {
            return Err(ProviderError::new(
                ProviderErrorKind::Network,
                "The provider's answer ended early.",
            ));
        };
        let mut parts = Vec::new();
        if !self.text.is_empty() {
            parts.push(Part::Text(self.text));
        }
        for (index, call) in self.calls {
            if call.name.is_empty() {
                return Err(ProviderError::new(
                    ProviderErrorKind::InvalidResponse,
                    "The model called a tool without naming it.",
                ));
            }
            let input = if call.arguments.trim().is_empty() {
                ToolInput::Json(json!({}))
            } else {
                serde_json::from_str(&call.arguments)
                    .map_or(ToolInput::Malformed(call.arguments), ToolInput::Json)
            };
            let call = ToolCall {
                id: call.id.unwrap_or_else(|| format!("call_{index}")),
                name: call.name,
                input,
            };
            on_event(ChatEvent::ToolCall(call.clone()));
            parts.push(Part::ToolCall(call));
        }
        let has_calls = parts.iter().any(|p| matches!(p, Part::ToolCall(_)));
        let stop_reason = match finish_reason.as_str() {
            "tool_calls" | "function_call" => StopReason::ToolUse,
            // Some servers say `stop` after tool calls.
            "stop" if has_calls => StopReason::ToolUse,
            "stop" => StopReason::EndTurn,
            "length" => StopReason::MaxTokens,
            "content_filter" => StopReason::Refusal,
            other => StopReason::Other(other.to_owned()),
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

impl AiProvider for OpenAiCompatibleProvider {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            kind: ProviderKind::OpenAiCompatible,
            streaming: true,
            tool_calls: self.options.tool_calls,
            structured_output: self.options.structured_output,
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

impl OpenAiCompatibleProvider {
    async fn read_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        let url = endpoint(&self.settings.base_url, "models");
        let response = send(|| self.authorized(self.http.get(url.clone()))).await?;
        let json = read_json(response).await?;
        let mut models: Vec<ModelInfo> = json
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ProviderError::new(
                    ProviderErrorKind::InvalidResponse,
                    "The provider's model list is not in the expected form.",
                )
            })?
            .iter()
            .filter_map(|model| model.get("id").and_then(Value::as_str))
            .take(MAX_MODELS)
            .map(|id| ModelInfo {
                id: id.to_owned(),
                display_name: None,
                context_window: None,
                max_output_tokens: None,
            })
            .collect();
        models.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(models)
    }

    async fn stream_chat(
        &self,
        request: &ChatRequest,
        on_event: &mut (dyn FnMut(ChatEvent) + Send),
    ) -> Result<ChatResponse, ProviderError> {
        let body = self.body(request)?;
        let url = endpoint(&self.settings.base_url, "chat/completions");
        let response = send(|| {
            self.authorized(self.http.post(url.clone()))
                .json_body(&body)
        })
        .await?;
        let mut state = Accumulator::default();
        read_events(response, |event| {
            if event.data.trim() == "[DONE]" {
                return Ok(Flow::Done);
            }
            state.handle(&event_json(&event)?, on_event)?;
            Ok(Flow::Continue)
        })
        .await?;
        state.finish(on_event)
    }
}

#[cfg(test)]
mod tests;
