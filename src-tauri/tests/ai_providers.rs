//! Contract tests against real AI endpoints. Every adapter must pass the same checks: list
//! models, stream text, call a tool and use its result, answer in JSON that follows a schema,
//! and stop when cancelled.
//!
//! All tests are ignored by default and panic when their endpoint's variables are missing, so
//! CI cannot pass them by accident. Run one provider at a time:
//!
//! ```text
//! # Any OpenAI-compatible server: Ollama, llama.cpp's llama-server (with --jinja), vLLM, OpenAI.
//! AIDB_TEST_OPENAI_URL=http://127.0.0.1:11434/v1 AIDB_TEST_OPENAI_MODEL=qwen2.5:7b \
//!   cargo test -p aidb --test ai_providers -- --ignored openai_compatible
//! # Optional: AIDB_TEST_OPENAI_KEY for hosted endpoints.
//!
//! AIDB_TEST_ANTHROPIC_KEY=sk-ant-... cargo test -p aidb --test ai_providers -- --ignored anthropic
//! # Optional: AIDB_TEST_ANTHROPIC_MODEL (default claude-opus-5-5), AIDB_TEST_ANTHROPIC_URL.
//! ```
//!
//! These tests spend tokens on hosted providers: a few small requests per run.

use std::sync::Arc;
use std::time::Duration;

use aidb_lib::adapters::ai_providers::anthropic::DEFAULT_BASE_URL;
use aidb_lib::adapters::ai_providers::openai_compatible::OpenAiCompatibleOptions;
use aidb_lib::adapters::ai_providers::{
    AiProvider, AnthropicProvider, OpenAiCompatibleProvider, ProviderClient, ProviderErrorKind,
    ProviderSettings,
};
use aidb_lib::domain::ai::{
    ChatEvent, ChatRequest, Message, OutputFormat, Part, Role, StopReason, ToolInput, ToolResult,
    ToolSpec,
};
use aidb_lib::services::ai::chat;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::sync::Notify;
use url::Url;

const TIME_LIMIT: Duration = Duration::from_secs(180);

struct Endpoint {
    provider: ProviderClient,
    model: String,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn openai_compatible() -> Endpoint {
    let url = env("AIDB_TEST_OPENAI_URL").expect("set AIDB_TEST_OPENAI_URL");
    let model = env("AIDB_TEST_OPENAI_MODEL").expect("set AIDB_TEST_OPENAI_MODEL");
    let provider = OpenAiCompatibleProvider::new(
        ProviderSettings {
            base_url: Url::parse(&url).expect("AIDB_TEST_OPENAI_URL is a URL"),
            api_key: env("AIDB_TEST_OPENAI_KEY").map(SecretString::from),
            idle_timeout: Duration::from_secs(120),
        },
        OpenAiCompatibleOptions::default(),
    )
    .expect("provider");
    Endpoint {
        provider: ProviderClient::OpenAiCompatible(provider),
        model,
    }
}

fn anthropic() -> Endpoint {
    let key = env("AIDB_TEST_ANTHROPIC_KEY").expect("set AIDB_TEST_ANTHROPIC_KEY");
    let url = env("AIDB_TEST_ANTHROPIC_URL").unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());
    let provider = AnthropicProvider::new(ProviderSettings {
        base_url: Url::parse(&url).expect("AIDB_TEST_ANTHROPIC_URL is a URL"),
        api_key: Some(SecretString::from(key)),
        idle_timeout: Duration::from_secs(120),
    })
    .expect("provider");
    Endpoint {
        provider: ProviderClient::Anthropic(provider),
        model: env("AIDB_TEST_ANTHROPIC_MODEL").unwrap_or_else(|| "claude-opus-5-5".to_owned()),
    }
}

fn request(endpoint: &Endpoint, prompt: &str) -> ChatRequest {
    ChatRequest {
        model: endpoint.model.clone(),
        system: Some("You are a terse assistant inside a database client.".into()),
        messages: vec![Message::user(prompt)],
        tools: Vec::new(),
        output: OutputFormat::Text,
        max_output_tokens: Some(2048),
    }
}

async fn lists_models(endpoint: &Endpoint) {
    let models = endpoint.provider.list_models().await.expect("models");
    assert!(!models.is_empty(), "no models listed");
}

async fn streams_text(endpoint: &Endpoint) {
    let mut events = Vec::new();
    let response = chat(
        &endpoint.provider,
        &request(endpoint, "Reply with one word: ready"),
        TIME_LIMIT,
        std::future::pending(),
        &mut |event| events.push(event),
    )
    .await
    .expect("response");
    assert_eq!(response.stop_reason, StopReason::EndTurn);
    let streamed: String = events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::Text(text) => Some(text.as_str()),
            ChatEvent::ToolCall(_) => None,
        })
        .collect();
    assert!(!streamed.trim().is_empty(), "no text streamed");
    assert_eq!(streamed, response.message.text());
    assert!(
        response.usage.output_tokens.is_some_and(|n| n > 0),
        "{:?}",
        response.usage
    );
}

async fn calls_a_tool_and_uses_the_result(endpoint: &Endpoint) {
    let mut request = request(
        endpoint,
        "How many rows does the table `orders` have? Use the count_rows tool, then answer with the number.",
    );
    request.tools = vec![ToolSpec {
        name: "count_rows".into(),
        description: "Counts the rows of a table. Call this whenever the user asks how many rows a table has.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {"table": {"type": "string", "description": "Table name"}},
            "required": ["table"],
            "additionalProperties": false,
        }),
    }];
    let response = chat(
        &endpoint.provider,
        &request,
        TIME_LIMIT,
        std::future::pending(),
        &mut |_| {},
    )
    .await
    .expect("first turn");
    assert_eq!(response.stop_reason, StopReason::ToolUse, "{response:?}");
    let call = response
        .runnable_tool_calls()
        .next()
        .expect("a tool call")
        .clone();
    assert_eq!(call.name, "count_rows");
    let ToolInput::Json(input) = &call.input else {
        panic!("malformed arguments: {:?}", call.input);
    };
    assert_eq!(input["table"], json!("orders"));

    request.messages.push(response.message);
    request.messages.push(Message {
        role: Role::User,
        parts: vec![Part::ToolResult(ToolResult {
            call_id: call.id,
            content: "4217".into(),
            is_error: false,
        })],
    });
    let response = chat(
        &endpoint.provider,
        &request,
        TIME_LIMIT,
        std::future::pending(),
        &mut |_| {},
    )
    .await
    .expect("second turn");
    assert_eq!(response.stop_reason, StopReason::EndTurn, "{response:?}");
    let text = response.message.text().replace(',', "");
    assert!(text.contains("4217"), "{text}");
}

async fn answers_in_json_following_a_schema(endpoint: &Endpoint) {
    let mut request = request(
        endpoint,
        "What is six times seven? Give the number and one word describing it.",
    );
    request.output = OutputFormat::Json {
        name: "answer".into(),
        schema: json!({
            "type": "object",
            "properties": {"value": {"type": "integer"}, "word": {"type": "string"}},
            "required": ["value", "word"],
            "additionalProperties": false,
        }),
    };
    let response = chat(
        &endpoint.provider,
        &request,
        TIME_LIMIT,
        std::future::pending(),
        &mut |_| {},
    )
    .await
    .expect("response");
    let answer: Value = serde_json::from_str(response.message.text().trim()).expect("JSON answer");
    assert_eq!(answer["value"], json!(42), "{answer}");
    assert!(answer["word"].is_string(), "{answer}");
}

async fn stops_when_cancelled(endpoint: &Endpoint) {
    let first_text = Arc::new(Notify::new());
    let signal = Arc::clone(&first_text);
    let started = std::time::Instant::now();
    let error = chat(
        &endpoint.provider,
        &request(endpoint, "Count from 1 to 2000, one number per line."),
        TIME_LIMIT,
        first_text.notified(),
        &mut move |event| {
            if matches!(event, ChatEvent::Text(_)) {
                signal.notify_one();
            }
        },
    )
    .await
    .expect_err("cancelled");
    assert_eq!(error.kind, ProviderErrorKind::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(60));
}

async fn contract(endpoint: Endpoint) {
    lists_models(&endpoint).await;
    streams_text(&endpoint).await;
    calls_a_tool_and_uses_the_result(&endpoint).await;
    answers_in_json_following_a_schema(&endpoint).await;
    stops_when_cancelled(&endpoint).await;
}

#[tokio::test]
#[ignore = "needs an OpenAI-compatible endpoint: set AIDB_TEST_OPENAI_URL and AIDB_TEST_OPENAI_MODEL"]
async fn openai_compatible_contract() {
    contract(openai_compatible()).await;
}

#[tokio::test]
#[ignore = "needs an Anthropic API key: set AIDB_TEST_ANTHROPIC_KEY"]
async fn anthropic_contract() {
    contract(anthropic()).await;
}
