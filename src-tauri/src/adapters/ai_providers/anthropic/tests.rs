use std::time::Duration;

use serde_json::json;

use super::*;
use crate::adapters::ai_providers::test_server::{Reply, TestServer};
use crate::domain::ai::{ToolResult, ToolSpec};

const KEY: &str = "sk-ant-test-key";

fn provider(server: &TestServer) -> AnthropicProvider {
    let mut base = server.url.clone();
    base.set_path("/");
    AnthropicProvider::new(ProviderSettings {
        base_url: base,
        api_key: Some(SecretString::from(KEY)),
        idle_timeout: Duration::from_secs(5),
    })
    .expect("provider")
}

fn request() -> ChatRequest {
    ChatRequest {
        model: "claude-opus-5-5".into(),
        system: Some("You help with PostgreSQL.".into()),
        messages: vec![Message::user("How many orders?")],
        tools: vec![ToolSpec {
            name: "run_read_query".into(),
            description: "Runs a read-only query.".into(),
            input_schema: json!({"type": "object", "properties": {"sql": {"type": "string"}}}),
        }],
        output: OutputFormat::Text,
        max_output_tokens: None,
    }
}

async fn chat(
    provider: &AnthropicProvider,
    request: &ChatRequest,
) -> (Result<ChatResponse, ProviderError>, Vec<ChatEvent>) {
    let mut events = Vec::new();
    let result = provider
        .chat(request, &mut |event| events.push(event))
        .await;
    (result, events)
}

fn event(name: &str, data: &serde_json::Value) -> String {
    format!("event: {name}\ndata: {data}")
}

fn tool_use_stream() -> Vec<String> {
    vec![
        event(
            "message_start",
            &json!({"type": "message_start", "message": {"id": "msg_1", "model": "claude-opus-5-5",
                "usage": {"input_tokens": 30, "cache_read_input_tokens": 100, "output_tokens": 1}}}),
        ),
        event(
            "content_block_start",
            &json!({"type": "content_block_start", "index": 0,
                "content_block": {"type": "thinking", "thinking": "", "signature": ""}}),
        ),
        event(
            "content_block_delta",
            &json!({"type": "content_block_delta", "index": 0,
                "delta": {"type": "signature_delta", "signature": "sig-abc"}}),
        ),
        event(
            "content_block_stop",
            &json!({"type": "content_block_stop", "index": 0}),
        ),
        event(
            "content_block_start",
            &json!({"type": "content_block_start", "index": 1,
                "content_block": {"type": "text", "text": ""}}),
        ),
        event("ping", &json!({"type": "ping"})),
        event(
            "content_block_delta",
            &json!({"type": "content_block_delta", "index": 1,
                "delta": {"type": "text_delta", "text": "Counting."}}),
        ),
        event(
            "content_block_stop",
            &json!({"type": "content_block_stop", "index": 1}),
        ),
        event(
            "content_block_start",
            &json!({"type": "content_block_start", "index": 2,
                "content_block": {"type": "tool_use", "id": "toolu_1", "name": "run_read_query", "input": {}}}),
        ),
        event(
            "content_block_delta",
            &json!({"type": "content_block_delta", "index": 2,
                "delta": {"type": "input_json_delta", "partial_json": "{\"sql\": \"SELECT"}}),
        ),
        event(
            "content_block_delta",
            &json!({"type": "content_block_delta", "index": 2,
                "delta": {"type": "input_json_delta", "partial_json": " count(*) FROM orders\"}"}}),
        ),
        event(
            "content_block_stop",
            &json!({"type": "content_block_stop", "index": 2}),
        ),
        event(
            "message_delta",
            &json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"},
                "usage": {"output_tokens": 42}}),
        ),
        event("message_stop", &json!({"type": "message_stop"})),
    ]
}

#[tokio::test]
async fn streams_text_reasoning_and_tool_calls() {
    let stream = tool_use_stream();
    let refs: Vec<&str> = stream.iter().map(String::as_str).collect();
    let mut server = TestServer::start(vec![Reply::sse(&refs)]).await;
    let provider = provider(&server);

    let (result, events) = chat(&provider, &request()).await;
    let response = result.expect("response");

    let call = ToolCall {
        id: "toolu_1".into(),
        name: "run_read_query".into(),
        input: ToolInput::Json(json!({"sql": "SELECT count(*) FROM orders"})),
    };
    assert_eq!(
        events,
        vec![
            ChatEvent::Text("Counting.".into()),
            ChatEvent::ToolCall(call.clone())
        ]
    );
    assert_eq!(
        response.message.parts,
        vec![
            Part::ProviderData(ProviderData {
                provider: ProviderKind::Anthropic,
                value: json!({"type": "thinking", "thinking": "", "signature": "sig-abc"}),
            }),
            Part::Text("Counting.".into()),
            Part::ToolCall(call),
        ]
    );
    assert_eq!(response.stop_reason, StopReason::ToolUse);
    assert_eq!(
        response.usage,
        Usage {
            input_tokens: Some(130),
            output_tokens: Some(42)
        }
    );

    let sent = server.request().await;
    assert_eq!(
        (sent.method.as_str(), sent.path.as_str()),
        ("POST", "/v1/messages")
    );
    assert_eq!(sent.header("x-api-key"), Some(KEY));
    assert_eq!(sent.header("anthropic-version"), Some(API_VERSION));
    assert_eq!(sent.header("authorization"), None);
    let body = sent.json();
    assert_eq!(body["model"], json!("claude-opus-5-5"));
    assert_eq!(body["max_tokens"], json!(DEFAULT_MAX_TOKENS));
    assert_eq!(body["stream"], json!(true));
    assert_eq!(body["system"], json!("You help with PostgreSQL."));
    assert_eq!(body["cache_control"], json!({"type": "ephemeral"}));
    assert_eq!(
        body["messages"],
        json!([{"role": "user", "content": [{"type": "text", "text": "How many orders?"}]}])
    );
    assert_eq!(body["tools"][0]["input_schema"]["type"], json!("object"));
    // Sampling settings are not sent: API version 2026-10-21 rejects them.
    for field in ["temperature", "top_p", "top_k", "output_config"] {
        assert!(body.get(field).is_none(), "{field}");
    }
}

#[tokio::test]
async fn replays_history_with_reasoning_and_results_first() {
    let mut server = TestServer::start(vec![Reply::sse(&[
        &event(
            "message_start",
            &json!({"type": "message_start", "message": {"model": "claude-opus-5-5", "usage": {"input_tokens": 1}}}),
        ),
        &event(
            "message_delta",
            &json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 1}}),
        ),
        &event("message_stop", &json!({"type": "message_stop"})),
    ])])
    .await;
    let provider = provider(&server);
    let thinking = json!({"type": "thinking", "thinking": "", "signature": "sig-abc"});
    let mut request = request();
    request.messages.extend([
        Message {
            role: Role::Assistant,
            parts: vec![
                Part::ProviderData(ProviderData {
                    provider: ProviderKind::Anthropic,
                    value: thinking.clone(),
                }),
                Part::ProviderData(ProviderData {
                    provider: ProviderKind::OpenAiCompatible,
                    value: json!({"ignored": true}),
                }),
                Part::ToolCall(ToolCall {
                    id: "toolu_1".into(),
                    name: "run_read_query".into(),
                    input: ToolInput::Json(json!({"sql": "SELECT 1"})),
                }),
                Part::ToolCall(ToolCall {
                    id: "toolu_2".into(),
                    name: "run_read_query".into(),
                    input: ToolInput::Malformed("{\"sql".into()),
                }),
            ],
        },
        Message {
            role: Role::User,
            parts: vec![
                Part::Text("Also by month.".into()),
                Part::ToolResult(ToolResult {
                    call_id: "toolu_1".into(),
                    content: "[[12]]".into(),
                    is_error: false,
                }),
                Part::ToolResult(ToolResult {
                    call_id: "toolu_2".into(),
                    content: "invalid arguments".into(),
                    is_error: true,
                }),
            ],
        },
    ]);

    let (result, _) = chat(&provider, &request).await;
    let response = result.expect("response");
    assert_eq!(response.stop_reason, StopReason::EndTurn);
    assert!(response.message.parts.is_empty());

    let body = server.request().await.json();
    assert_eq!(
        body["messages"][1],
        json!({"role": "assistant", "content": [
            thinking,
            {"type": "tool_use", "id": "toolu_1", "name": "run_read_query", "input": {"sql": "SELECT 1"}},
            {"type": "tool_use", "id": "toolu_2", "name": "run_read_query", "input": {}},
        ]})
    );
    assert_eq!(
        body["messages"][2],
        json!({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "toolu_1", "content": "[[12]]"},
            {"type": "tool_result", "tool_use_id": "toolu_2", "content": "invalid arguments", "is_error": true},
            {"type": "text", "text": "Also by month."},
        ]})
    );
}

#[tokio::test]
async fn structured_output_and_stop_reasons() {
    let schema = json!({"type": "object", "properties": {"answer": {"type": "string"}}, "additionalProperties": false});
    let stream = |stop: &str| {
        let events = [
            event(
                "message_start",
                &json!({"type": "message_start", "message": {"usage": {"input_tokens": 1}}}),
            ),
            event(
                "message_delta",
                &json!({"type": "message_delta", "delta": {"stop_reason": stop}, "usage": {"output_tokens": 1}}),
            ),
            event("message_stop", &json!({"type": "message_stop"})),
        ];
        let refs: Vec<&str> = events.iter().map(String::as_str).collect();
        Reply::sse(&refs)
    };
    let mut server = TestServer::start(vec![
        stream("refusal"),
        stream("max_tokens"),
        stream("pause_turn"),
    ])
    .await;
    let provider = provider(&server);
    let mut request = request();
    request.output = OutputFormat::Json {
        name: "answer".into(),
        schema: schema.clone(),
    };
    request.max_output_tokens = Some(1024);

    let mut reasons = Vec::new();
    for _ in 0..3 {
        let (result, _) = chat(&provider, &request).await;
        reasons.push(result.expect("response").stop_reason);
    }
    assert_eq!(
        reasons,
        vec![
            StopReason::Refusal,
            StopReason::MaxTokens,
            StopReason::Other("pause_turn".into())
        ]
    );
    let body = server.request().await.json();
    assert_eq!(
        body["output_config"],
        json!({"format": {"type": "json_schema", "schema": schema}})
    );
    assert_eq!(body["max_tokens"], json!(1024));
}

#[tokio::test]
async fn errors_are_classified_and_never_echo_the_key() {
    let mut server = TestServer::start(vec![
        Reply::json(
            401,
            &format!(r#"{{"type":"error","error":{{"type":"authentication_error","message":"invalid x-api-key {KEY}"}}}}"#),
        ),
        Reply::json(529, r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#)
            .with_header("x-should-retry", "false"),
        Reply::sse(&[
            &event("message_start", &json!({"type": "message_start", "message": {}})),
            &event(
                "error",
                &json!({"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}),
            ),
        ]),
        Reply::sse(&[&event("message_start", &json!({"type": "message_start", "message": {}}))]),
    ])
    .await;
    let provider = provider(&server);

    let mut kinds = Vec::new();
    for _ in 0..4 {
        let (result, _) = chat(&provider, &request()).await;
        let error = result.expect_err("fails");
        assert!(!error.message.contains(KEY), "{}", error.message);
        kinds.push(error.kind);
    }
    assert_eq!(
        kinds,
        vec![
            ProviderErrorKind::Unauthorized,
            ProviderErrorKind::Overloaded,
            ProviderErrorKind::Overloaded,
            ProviderErrorKind::Network,
        ]
    );
    for _ in 0..4 {
        let _ = server.request().await;
    }
}

#[test]
fn a_key_is_required() {
    let settings = ProviderSettings {
        base_url: url::Url::parse(DEFAULT_BASE_URL).expect("url"),
        api_key: Some(SecretString::from("  ")),
        idle_timeout: Duration::from_secs(5),
    };
    assert_eq!(
        AnthropicProvider::new(settings).expect_err("no key").kind,
        ProviderErrorKind::Unauthorized
    );
}

#[tokio::test]
async fn lists_models_across_pages() {
    let mut server = TestServer::start(vec![
        Reply::json(
            200,
            r#"{"data":[{"id":"claude-opus-5-5","display_name":"Claude Opus 5.5","max_input_tokens":1000000,"max_tokens":128000}],"has_more":true,"last_id":"claude-opus-5-5"}"#,
        ),
        Reply::json(
            200,
            r#"{"data":[{"id":"claude-haiku-5-5","display_name":"Claude Haiku 5.5"}],"has_more":false,"last_id":"claude-haiku-5-5"}"#,
        ),
    ])
    .await;
    let provider = provider(&server);
    let models = provider.list_models().await.expect("models");
    assert_eq!(
        models[0],
        ModelInfo {
            id: "claude-opus-5-5".into(),
            display_name: Some("Claude Opus 5.5".into()),
            context_window: Some(1_000_000),
            max_output_tokens: Some(128_000),
        }
    );
    assert_eq!(models.len(), 2);
    assert_eq!(server.request().await.path, "/v1/models?limit=1000");
    assert_eq!(
        server.request().await.path,
        "/v1/models?limit=1000&after_id=claude-opus-5-5"
    );
}
