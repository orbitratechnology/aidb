//! Provider-independent model conversations: what the agent sends to an AI provider and what it
//! gets back. Adapters translate these to each provider's wire format; nothing here knows about
//! HTTP or a particular API.
//!
//! Everything a provider returns is untrusted: text is shown as text, and tool calls are checked
//! against the tool registry before anything acts on them (the agent's job).

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Most tools one request may offer.
pub const MAX_TOOLS: usize = 64;
/// Most tool calls accepted in one response; more is treated as a malformed response.
pub const MAX_TOOL_CALLS: usize = 64;
/// Most bytes of text, tool input and provider data accepted in one response.
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

/// Which wire protocol a provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// `POST /chat/completions` as OpenAI defines it; also Ollama, LM Studio, llama.cpp, vLLM.
    OpenAiCompatible,
    /// Anthropic's Messages API.
    Anthropic,
}

/// How a provider can be made to answer in JSON that follows a schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredOutput {
    /// The provider constrains output to the schema.
    JsonSchema,
    /// The provider guarantees JSON but not the schema; the schema goes in the instructions.
    JsonObject,
    /// Neither: the schema goes in the instructions and the reply is checked afterwards.
    Instructions,
}

/// What an adapter can do with its current settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilities {
    pub kind: ProviderKind,
    pub streaming: bool,
    pub tool_calls: bool,
    pub structured_output: StructuredOutput,
    /// Whether dropping a request stops generation on the provider's side. True for HTTP
    /// streaming APIs: closing the connection ends the request.
    pub cancellation: bool,
}

/// A model the provider offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub display_name: Option<String>,
    /// Input tokens the model accepts, when the provider says.
    pub context_window: Option<u32>,
    /// Output tokens the model can produce, when the provider says.
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

/// One message of a conversation. System instructions are not messages; see [`ChatRequest`].
#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub role: Role,
    pub parts: Vec<Part>,
}

impl Message {
    #[must_use]
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            parts: vec![Part::Text(text.into())],
        }
    }

    /// The text parts joined, without tool calls or provider data.
    #[must_use]
    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|part| match part {
                Part::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// The tool calls in this message, in order.
    pub fn tool_calls(&self) -> impl Iterator<Item = &ToolCall> {
        self.parts.iter().filter_map(|part| match part {
            Part::ToolCall(call) => Some(call),
            _ => None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Text(String),
    /// A call the model asked for (assistant messages only).
    ToolCall(ToolCall),
    /// The answer to a call (user messages only).
    ToolResult(ToolResult),
    /// Something only the provider that produced it understands, such as a reasoning block that
    /// must be sent back unchanged. Other providers skip it.
    ProviderData(ProviderData),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    /// The provider's id for the call; results refer to it.
    pub id: String,
    pub name: String,
    pub input: ToolInput,
}

/// The arguments of a tool call, as the model wrote them.
#[derive(Debug, Clone, PartialEq)]
pub enum ToolInput {
    Json(Value),
    /// Not valid JSON. Never run such a call: answer it with an error result instead.
    Malformed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub call_id: String,
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderData {
    pub provider: ProviderKind,
    pub value: Value,
}

/// A tool the model may call.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    /// When and why to call it; models choose tools by this.
    pub description: String,
    /// JSON Schema of the arguments (an object schema).
    pub input_schema: Value,
}

/// The shape the final answer must have.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum OutputFormat {
    #[default]
    Text,
    /// JSON following `schema`; `name` identifies the schema to providers that ask for one.
    Json { name: String, schema: Value },
}

/// One request to a model.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub model: String,
    pub system: Option<String>,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub output: OutputFormat,
    /// Upper bound on output tokens; adapters pick a default when not set.
    pub max_output_tokens: Option<u32>,
}

/// Why a request is malformed. These are bugs in the caller, never user input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RequestError {
    #[error("no model is set")]
    NoModel,
    #[error("a conversation must start with a user message")]
    MustStartWithUser,
    #[error("too many tools ({0}, at most {MAX_TOOLS})")]
    TooManyTools(usize),
    #[error("`{0}` is not a valid tool name")]
    ToolName(String),
    #[error("the tool `{0}` is offered twice")]
    DuplicateTool(String),
    #[error("the schema of `{0}` must be a JSON object schema")]
    ToolSchema(String),
    #[error("an output schema must be a JSON object schema")]
    OutputSchema,
}

/// Names both APIs accept: letters, digits, `_` and `-`, at most 64.
static TOOL_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_-]{1,64}$").expect("tool name pattern must compile"));

fn is_object_schema(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("object")
}

impl ChatRequest {
    /// # Errors
    ///
    /// When the request could not be sent to any provider as it is.
    pub fn validate(&self) -> Result<(), RequestError> {
        if self.model.trim().is_empty() {
            return Err(RequestError::NoModel);
        }
        if self.messages.first().map(|m| m.role) != Some(Role::User) {
            return Err(RequestError::MustStartWithUser);
        }
        if self.tools.len() > MAX_TOOLS {
            return Err(RequestError::TooManyTools(self.tools.len()));
        }
        let mut names = std::collections::HashSet::new();
        for tool in &self.tools {
            if !TOOL_NAME.is_match(&tool.name) {
                return Err(RequestError::ToolName(tool.name.clone()));
            }
            if !names.insert(tool.name.as_str()) {
                return Err(RequestError::DuplicateTool(tool.name.clone()));
            }
            if !is_object_schema(&tool.input_schema) {
                return Err(RequestError::ToolSchema(tool.name.clone()));
            }
        }
        if let OutputFormat::Json { schema, .. } = &self.output
            && !is_object_schema(schema)
        {
            return Err(RequestError::OutputSchema);
        }
        Ok(())
    }
}

/// Why the model stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// It finished its answer.
    EndTurn,
    /// It wants the tool calls in the message run.
    ToolUse,
    /// It hit the output limit: the answer, or a tool call, may be cut short.
    MaxTokens,
    /// It declined, or the provider's filter stopped it. Never run tool calls from such a turn.
    Refusal,
    /// Something else, as the provider named it.
    Other(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// A finished response.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatResponse {
    /// The assistant message, to append to the conversation as is.
    pub message: Message,
    pub stop_reason: StopReason,
    pub usage: Usage,
    /// The model that answered, as the provider reported it.
    pub model: Option<String>,
}

impl ChatResponse {
    /// Tool calls that may be considered for running: none when the turn was cut short or
    /// refused, since their arguments may be incomplete.
    pub fn runnable_tool_calls(&self) -> impl Iterator<Item = &ToolCall> {
        let usable = matches!(self.stop_reason, StopReason::ToolUse | StopReason::EndTurn);
        self.message.tool_calls().filter(move |_| usable)
    }
}

/// Progress while a response streams in.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatEvent {
    /// More answer text.
    Text(String),
    /// A tool call, once its arguments are complete.
    ToolCall(ToolCall),
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn request() -> ChatRequest {
        ChatRequest {
            model: "m".into(),
            system: None,
            messages: vec![Message::user("hi")],
            tools: vec![ToolSpec {
                name: "list_tables".into(),
                description: "Lists tables".into(),
                input_schema: json!({"type": "object", "properties": {}}),
            }],
            output: OutputFormat::Text,
            max_output_tokens: None,
        }
    }

    #[test]
    fn requests_are_checked_before_sending() {
        assert_eq!(request().validate(), Ok(()));

        let mut r = request();
        r.model = " ".into();
        assert_eq!(r.validate(), Err(RequestError::NoModel));

        let mut r = request();
        r.messages[0].role = Role::Assistant;
        assert_eq!(r.validate(), Err(RequestError::MustStartWithUser));

        let mut r = request();
        r.tools[0].name = "list tables".into();
        assert!(matches!(r.validate(), Err(RequestError::ToolName(_))));

        let mut r = request();
        r.tools.push(r.tools[0].clone());
        assert!(matches!(r.validate(), Err(RequestError::DuplicateTool(_))));

        let mut r = request();
        r.tools[0].input_schema = json!({"type": "string"});
        assert!(matches!(r.validate(), Err(RequestError::ToolSchema(_))));

        let mut r = request();
        r.output = OutputFormat::Json {
            name: "plan".into(),
            schema: json!(true),
        };
        assert_eq!(r.validate(), Err(RequestError::OutputSchema));
    }

    #[test]
    fn cut_short_or_refused_turns_have_no_runnable_calls() {
        let call = ToolCall {
            id: "1".into(),
            name: "list_tables".into(),
            input: ToolInput::Json(json!({})),
        };
        let mut response = ChatResponse {
            message: Message {
                role: Role::Assistant,
                parts: vec![Part::Text("ok".into()), Part::ToolCall(call)],
            },
            stop_reason: StopReason::ToolUse,
            usage: Usage::default(),
            model: None,
        };
        assert_eq!(response.runnable_tool_calls().count(), 1);
        assert_eq!(response.message.text(), "ok");
        for reason in [StopReason::MaxTokens, StopReason::Refusal] {
            response.stop_reason = reason;
            assert_eq!(response.runnable_tool_calls().count(), 0);
        }
    }
}
