use std::time::Duration;

use secrecy::SecretString;
use serde_json::json;

use super::*;
use crate::adapters::ai_providers::test_server::{Reply, TestServer};
use crate::domain::ai::{ToolResult, ToolSpec};

fn provider(server: &TestServer, options: OpenAiCompatibleOptions) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(
        ProviderSettings {
            base_url: server.url.clone(),
            api_key: Some(SecretString::from("sk-test-key")),
            idle_timeout: Duration::from_secs(5),
        },
        options,
    )
    .expect("provider")
}

fn request() -> ChatRequest {
    ChatRequest {
        model: "qwen2.5".into(),
        system: Some("You help with PostgreSQL.".into()),
        messages: vec![Message::user("How many orders?")],
        tools: vec![ToolSpec {
            name: "run_read_query".into(),
            description: "Runs a read-only query.".into(),
            input_schema: json!({"type": "object", "properties": {"sql": {"type": "string"}}}),
        }],
        output: OutputFormat::Text,
        max_output_tokens: Some(512),
    }
}

async fn chat(
    provider: &OpenAiCompatibleProvider,
    request: &ChatRequest,
) -> (Result<ChatResponse, ProviderError>, Vec<ChatEvent>) {
    let mut events = Vec::new();
    let result = provider
        .chat(request, &mut |event| events.push(event))
        .await;
    (result, events)
}

#[tokio::test]
async fn streams_text_and_assembles_tool_calls() {
    let mut server = TestServer::start(vec![Reply::sse(&[
        r#"data: {"model":"qwen2.5","choices":[{"index":0,"delta":{"role":"assistant","content":"Let me "}}]}"#,
        r#"data: {"choices":[{"index":0,"delta":{"content":"check."}}]}"#,
        r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"run_read_query","arguments":"{\"sql\":"}}]}}]}"#,
        r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"SELECT count(*) FROM orders\"}"}}]}}]}"#,
        r#"data: {"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
        r#"data: {"choices":[],"usage":{"prompt_tokens":40,"completion_tokens":12}}"#,
        "data: [DONE]",
    ])])
    .await;
    let provider = provider(&server, OpenAiCompatibleOptions::default());

    let (result, events) = chat(&provider, &request()).await;
    let response = result.expect("response");

    let call = ToolCall {
        id: "call_1".into(),
        name: "run_read_query".into(),
        input: ToolInput::Json(json!({"sql": "SELECT count(*) FROM orders"})),
    };
    assert_eq!(
        events,
        vec![
            ChatEvent::Text("Let me ".into()),
            ChatEvent::Text("check.".into()),
            ChatEvent::ToolCall(call.clone()),
        ]
    );
    assert_eq!(
        response.message.parts,
        vec![Part::Text("Let me check.".into()), Part::ToolCall(call)]
    );
    assert_eq!(response.stop_reason, StopReason::ToolUse);
    assert_eq!(
        response.usage,
        Usage {
            input_tokens: Some(40),
            output_tokens: Some(12)
        }
    );
    assert_eq!(response.model.as_deref(), Some("qwen2.5"));

    let sent = server.request().await;
    assert_eq!(
        (sent.method.as_str(), sent.path.as_str()),
        ("POST", "/v1/chat/completions")
    );
    assert_eq!(sent.header("authorization"), Some("Bearer sk-test-key"));
    let body = sent.json();
    assert_eq!(
        body["messages"],
        json!([
            {"role": "system", "content": "You help with PostgreSQL."},
            {"role": "user", "content": "How many orders?"},
        ])
    );
    assert_eq!(body["stream"], json!(true));
    assert_eq!(body["max_tokens"], json!(512));
    assert_eq!(
        body["tools"][0]["function"]["name"],
        json!("run_read_query")
    );
    assert!(body.get("response_format").is_none());
}

#[tokio::test]
async fn sends_history_with_tool_calls_and_results() {
    let mut server = TestServer::start(vec![Reply::sse(&[
        r#"data: {"choices":[{"index":0,"delta":{"content":"12 orders."},"finish_reason":"stop"}]}"#,
        "data: [DONE]",
    ])])
    .await;
    let provider = provider(&server, OpenAiCompatibleOptions::default());
    let mut request = request();
    request.messages.extend([
        Message {
            role: Role::Assistant,
            parts: vec![
                Part::ProviderData(crate::domain::ai::ProviderData {
                    provider: ProviderKind::Anthropic,
                    value: json!({"type": "thinking"}),
                }),
                Part::ToolCall(ToolCall {
                    id: "call_1".into(),
                    name: "run_read_query".into(),
                    input: ToolInput::Json(json!({"sql": "SELECT 1"})),
                }),
                Part::ToolCall(ToolCall {
                    id: "call_2".into(),
                    name: "run_read_query".into(),
                    input: ToolInput::Malformed("{\"sql\": ".into()),
                }),
            ],
        },
        Message {
            role: Role::User,
            parts: vec![
                Part::ToolResult(ToolResult {
                    call_id: "call_1".into(),
                    content: "[[12]]".into(),
                    is_error: false,
                }),
                Part::ToolResult(ToolResult {
                    call_id: "call_2".into(),
                    content: "invalid arguments".into(),
                    is_error: true,
                }),
            ],
        },
    ]);

    let (result, _) = chat(&provider, &request).await;
    assert_eq!(result.expect("response").stop_reason, StopReason::EndTurn);

    let body = server.request().await.json();
    assert_eq!(
        body["messages"],
        json!([
            {"role": "system", "content": "You help with PostgreSQL."},
            {"role": "user", "content": "How many orders?"},
            {"role": "assistant", "content": null, "tool_calls": [
                {"id": "call_1", "type": "function",
                 "function": {"name": "run_read_query", "arguments": "{\"sql\":\"SELECT 1\"}"}},
                {"id": "call_2", "type": "function",
                 "function": {"name": "run_read_query", "arguments": "{\"sql\": "}},
            ]},
            {"role": "tool", "tool_call_id": "call_1", "content": "[[12]]"},
            {"role": "tool", "tool_call_id": "call_2", "content": "Error: invalid arguments"},
        ])
    );
}

#[tokio::test]
async fn structured_output_follows_the_configured_mode() {
    let schema = json!({"type": "object", "properties": {"answer": {"type": "string"}}});
    let mut request = request();
    request.tools.clear();
    request.output = OutputFormat::Json {
        name: "answer".into(),
        schema: schema.clone(),
    };
    let reply = || {
        Reply::sse(&[
            r#"data: {"choices":[{"index":0,"delta":{"content":"{\"answer\":\"12\"}"},"finish_reason":"stop"}]}"#,
            "data: [DONE]",
        ])
    };
    let mut server = TestServer::start(vec![reply(), reply(), reply()]).await;

    for (mode, expected) in [
        (
            StructuredOutput::JsonSchema,
            json!({"type": "json_schema", "json_schema": {"name": "answer", "schema": schema, "strict": true}}),
        ),
        (StructuredOutput::JsonObject, json!({"type": "json_object"})),
        (StructuredOutput::Instructions, Value::Null),
    ] {
        let provider = provider(
            &server,
            OpenAiCompatibleOptions {
                structured_output: mode,
                ..OpenAiCompatibleOptions::default()
            },
        );
        let (result, _) = chat(&provider, &request).await;
        assert_eq!(
            result.expect("response").message.text(),
            r#"{"answer":"12"}"#
        );
        let body = server.request().await.json();
        assert_eq!(body["response_format"], expected, "{mode:?}");
        let system = body["messages"][0]["content"].as_str().expect("system");
        assert_eq!(
            system.contains("JSON Schema"),
            mode != StructuredOutput::JsonSchema,
            "{mode:?}"
        );
    }
}

#[tokio::test]
async fn malformed_and_unnamed_calls_are_reported_not_run() {
    let mut server = TestServer::start(vec![
        Reply::sse(&[
            r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"name":"run_read_query","arguments":"{\"sql\": SELECT"}}]},"finish_reason":"stop"}]}"#,
            "data: [DONE]",
        ]),
        Reply::sse(&[
            r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{}"}}]},"finish_reason":"tool_calls"}]}"#,
            "data: [DONE]",
        ]),
    ])
    .await;
    let provider = provider(&server, OpenAiCompatibleOptions::default());

    let (result, _) = chat(&provider, &request()).await;
    let response = result.expect("response");
    // Some servers end tool calls with `stop`; ids are filled in when missing.
    assert_eq!(response.stop_reason, StopReason::ToolUse);
    let call = response.message.tool_calls().next().expect("call");
    assert_eq!(call.id, "call_0");
    assert_eq!(call.input, ToolInput::Malformed("{\"sql\": SELECT".into()));
    let _ = server.request().await;

    let (result, _) = chat(&provider, &request()).await;
    assert_eq!(
        result.expect_err("unnamed").kind,
        ProviderErrorKind::InvalidResponse
    );
}

#[tokio::test]
async fn errors_are_classified() {
    let mut server = TestServer::start(vec![
        Reply::json(
            401,
            r#"{"error":{"message":"Incorrect API key provided: sk-test-key"}}"#,
        ),
        Reply::json(429, r#"{"error":{"message":"Rate limit reached"}}"#)
            .with_header("retry-after", "20"),
        Reply::json(
            404,
            r#"{"error":"model \"nope\" not found, try pulling it first"}"#,
        ),
        Reply::json(
            200,
            r#"{"choices":[{"message":{"content":"not streamed"}}]}"#,
        ),
        Reply::sse(&[r#"data: {"choices":[{"index":0,"delta":{"content":"partial"}}]}"#]),
        Reply::sse(&[r#"data: {"error":{"message":"model crashed"}}"#]),
        Reply {
            status: 307,
            headers: vec![("location".into(), "https://elsewhere.example/v1".into())],
            chunks: vec![],
        },
    ])
    .await;
    let provider = provider(&server, OpenAiCompatibleOptions::default());

    let mut kinds = Vec::new();
    for _ in 0..7 {
        let (result, _) = chat(&provider, &request()).await;
        let error = result.expect_err("fails");
        assert!(!error.message.contains("sk-test-key"), "{}", error.message);
        kinds.push((error.kind, error.retry_after));
    }
    assert_eq!(
        kinds,
        vec![
            (ProviderErrorKind::Unauthorized, None),
            (
                ProviderErrorKind::RateLimited,
                Some(Duration::from_secs(20))
            ),
            (ProviderErrorKind::NotFound, None),
            (ProviderErrorKind::InvalidResponse, None),
            (ProviderErrorKind::Network, None),
            (ProviderErrorKind::Server, None),
            (ProviderErrorKind::InvalidResponse, None),
        ]
    );
    // The redirect was not followed: the server saw exactly the seven requests above.
    for _ in 0..7 {
        let _ = server.request().await;
    }
}

#[tokio::test]
async fn failures_before_an_answer_are_retried() {
    let mut server = TestServer::start(vec![
        // A pooled connection the server closed, then an overloaded server.
        Reply::hang_up(),
        Reply::json(503, r#"{"error":{"message":"busy"}}"#).with_header("retry-after", "0"),
        Reply::sse(&[
            r#"data: {"choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}]}"#,
            "data: [DONE]",
        ]),
        // Errors the provider says not to retry, and errors that never succeed.
        Reply::json(500, r#"{"error":{"message":"bad"}}"#).with_header("x-should-retry", "false"),
        Reply::json(429, "{}").with_header("retry-after", "0"),
        Reply::json(429, "{}").with_header("retry-after", "0"),
        Reply::json(429, r#"{"error":{"message":"still limited"}}"#)
            .with_header("retry-after", "0"),
    ])
    .await;
    let provider = provider(&server, OpenAiCompatibleOptions::default());

    let (result, _) = chat(&provider, &request()).await;
    assert_eq!(result.expect("third attempt").message.text(), "ok");
    for _ in 0..3 {
        let _ = server.request().await;
    }

    let (result, _) = chat(&provider, &request()).await;
    assert_eq!(
        result.expect_err("not retried").kind,
        ProviderErrorKind::Server
    );
    let _ = server.request().await;

    let (result, _) = chat(&provider, &request()).await;
    let error = result.expect_err("gave up");
    assert_eq!(error.kind, ProviderErrorKind::RateLimited);
    assert!(
        error.message.ends_with("still limited"),
        "{}",
        error.message
    );
}

#[tokio::test]
async fn tools_are_refused_when_the_server_has_none() {
    let server = TestServer::start(vec![]).await;
    let provider = provider(
        &server,
        OpenAiCompatibleOptions {
            tool_calls: false,
            structured_output: StructuredOutput::Instructions,
            output_limit: OutputLimitField::MaxCompletionTokens,
        },
    );
    let (result, _) = chat(&provider, &request()).await;
    assert_eq!(
        result.expect_err("refused").kind,
        ProviderErrorKind::InvalidRequest
    );
    assert!(!provider.capabilities().tool_calls);
}

#[tokio::test]
async fn oversized_answers_are_cut_off() {
    let piece = "x".repeat(900_000);
    let events: Vec<String> = (0..6)
        .map(|_| format!(r#"data: {{"choices":[{{"index":0,"delta":{{"content":"{piece}"}}}}]}}"#))
        .collect();
    let refs: Vec<&str> = events.iter().map(String::as_str).collect();
    let server = TestServer::start(vec![Reply::sse(&refs)]).await;
    let provider = provider(&server, OpenAiCompatibleOptions::default());
    let (result, events) = chat(&provider, &request()).await;
    assert_eq!(
        result.expect_err("too large").kind,
        ProviderErrorKind::InvalidResponse
    );
    assert!(events.len() < 6);
}

#[tokio::test]
async fn the_output_limit_uses_the_configured_field() {
    let reply = || {
        Reply::sse(&[
            r#"data: {"choices":[{"index":0,"delta":{"content":"ok"},"finish_reason":"stop"}]}"#,
            "data: [DONE]",
        ])
    };
    let mut server = TestServer::start(vec![reply()]).await;
    let provider = provider(
        &server,
        OpenAiCompatibleOptions {
            output_limit: OutputLimitField::MaxCompletionTokens,
            ..OpenAiCompatibleOptions::default()
        },
    );
    let (result, _) = chat(&provider, &request()).await;
    assert!(result.is_ok());
    let body = server.request().await.json();
    assert_eq!(body["max_completion_tokens"], json!(512));
    assert!(body.get("max_tokens").is_none());
}

#[tokio::test]
async fn lists_models() {
    let mut server = TestServer::start(vec![Reply::json(
        200,
        r#"{"object":"list","data":[{"id":"qwen2.5:7b","object":"model"},{"id":"llama3.2","object":"model"}]}"#,
    )])
    .await;
    let provider = provider(&server, OpenAiCompatibleOptions::default());
    let ids: Vec<String> = provider
        .list_models()
        .await
        .expect("models")
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(ids, vec!["llama3.2", "qwen2.5:7b"]);
    let sent = server.request().await;
    assert_eq!(
        (sent.method.as_str(), sent.path.as_str()),
        ("GET", "/v1/models")
    );
}
