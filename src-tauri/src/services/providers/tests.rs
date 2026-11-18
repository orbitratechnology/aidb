use secrecy::SecretString;

use super::*;
use crate::adapters::ai_providers::test_server::{Reply, TestServer};
use crate::adapters::local_store::test_support::temp_store;
use crate::domain::providers::ProviderPreset;

fn credentials() -> Credentials {
    Credentials::session_only("tests")
}

fn local(endpoint: &str) -> ProviderDraft {
    ProviderDraft {
        endpoint: Some(endpoint.to_owned()),
        ..ProviderDraft::new("Local", ProviderPreset::OpenAiCompatible, "qwen2.5")
    }
}

fn sse_text(text: &str) -> Reply {
    let chunk =
        json!({"choices": [{"index": 0, "delta": {"content": text}, "finish_reason": "stop"}]});
    Reply::sse(&[&format!("data: {chunk}"), "data: [DONE]"])
}

fn sse_tool_call() -> Reply {
    let chunk = json!({"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": "c1",
        "function": {"name": "report_status", "arguments": "{\"status\":\"ok\"}"}}]},
        "finish_reason": "tool_calls"}]});
    Reply::sse(&[&format!("data: {chunk}"), "data: [DONE]"])
}

fn models() -> Reply {
    Reply::json(
        200,
        r#"{"object":"list","data":[{"id":"qwen2.5"},{"id":"llama3.2"}]}"#,
    )
}

#[tokio::test]
async fn profiles_are_saved_with_keys_kept_apart() {
    let (_dir, store) = temp_store().await;
    let credentials = credentials();

    let created = create(&store, &credentials, local("http://localhost:8080/v1"))
        .await
        .expect("create");
    assert_eq!(created.key, KeyStorage::NotSaved);
    assert!(!created.key_required);
    assert_eq!(created.endpoint_url, "http://localhost:8080/v1");
    assert_eq!(created.checked_at, None);

    let backend = save_key(
        &store,
        &credentials,
        created.id,
        SecretString::from("  sk-local-0123456789  "),
    )
    .await
    .expect("save key");
    assert_eq!(backend, CredentialBackend::SessionOnly);
    let listed = list(&store, &credentials).await.expect("list");
    assert_eq!(listed[0].key, KeyStorage::SessionOnly);
    // Session-only keys leave nothing on disk.
    let profile = store
        .providers()
        .get(created.id)
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(profile.secret_ref, None);
    let stored = credentials
        .load(&SecretRef::provider(created.id))
        .expect("load")
        .expect("key");
    assert_eq!(stored.expose_secret(), "sk-local-0123456789");

    let mut renamed = local("http://localhost:8080/v1");
    renamed.name = "Local llama".into();
    let updated = update(&store, &credentials, created.id, renamed)
        .await
        .expect("update");
    assert_eq!(updated.settings.name, "Local llama");
    assert_eq!(updated.key, KeyStorage::SessionOnly);

    let duplicate = create(&store, &credentials, {
        let mut draft = local("http://localhost:9000/v1");
        draft.name = "LOCAL LLAMA".into();
        draft
    })
    .await
    .expect_err("duplicate");
    assert_eq!(duplicate.code(), ErrorCode::Conflict);

    delete(&store, &credentials, created.id)
        .await
        .expect("delete");
    assert!(!credentials.holds_in_session(&SecretRef::provider(created.id)));
    assert_eq!(
        delete(&store, &credentials, created.id)
            .await
            .expect_err("gone")
            .code(),
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn keys_and_endpoints_are_checked() {
    let (_dir, store) = temp_store().await;
    let credentials = credentials();
    let created = create(&store, &credentials, local("http://localhost:8080/v1"))
        .await
        .expect("create");

    for bad in ["   ", "sk with spaces", &"k".repeat(5000), "sk-\u{e9}"] {
        let error = save_key(&store, &credentials, created.id, SecretString::from(bad))
            .await
            .expect_err("invalid key");
        assert_eq!(error.code(), ErrorCode::InvalidInput, "{bad}");
    }
    assert_eq!(
        save_key(
            &store,
            &credentials,
            ProviderId::new(),
            SecretString::from("sk-1")
        )
        .await
        .expect_err("unknown")
        .code(),
        ErrorCode::NotFound
    );

    let insecure = create(&store, &credentials, {
        let mut draft = local("http://10.1.2.3:8000/v1");
        draft.name = "Remote".into();
        draft
    })
    .await
    .expect_err("needs consent");
    assert_eq!(insecure.code(), ErrorCode::InvalidInput);
    let accepted = create(&store, &credentials, {
        let mut draft = local("http://10.1.2.3:8000/v1");
        draft.name = "Remote".into();
        draft.allow_insecure_http = true;
        draft
    })
    .await
    .expect("accepted");
    assert!(accepted.insecure_remote);
}

#[tokio::test]
async fn hosted_providers_need_a_key_before_use() {
    let (_dir, store) = temp_store().await;
    let credentials = credentials();
    let created = create(
        &store,
        &credentials,
        ProviderDraft::new("Claude", ProviderPreset::Anthropic, "claude-opus-5-5"),
    )
    .await
    .expect("create");
    assert!(created.key_required);
    assert_eq!(created.endpoint_url, "https://api.anthropic.com/");
    let error = client(&store, &credentials, created.id)
        .await
        .expect_err("no key");
    assert_eq!(error.code(), ErrorCode::InvalidInput);
    let error = test(&store, &credentials, created.id)
        .await
        .expect_err("no key");
    assert_eq!(error.code(), ErrorCode::InvalidInput);
}

#[tokio::test]
async fn a_damaged_profile_cannot_bypass_the_trust_rule() {
    let (_dir, store) = temp_store().await;
    let credentials = credentials();
    let created = create(&store, &credentials, local("http://localhost:8080/v1"))
        .await
        .expect("create");
    sqlx::query("UPDATE provider_profiles SET endpoint = 'http://203.0.113.9/v1' WHERE id = ?")
        .bind(created.id.to_string())
        .execute(store.pool())
        .await
        .expect("tamper");
    let error = client(&store, &credentials, created.id)
        .await
        .expect_err("refused");
    assert_eq!(error.code(), ErrorCode::InvalidInput);
}

#[tokio::test]
async fn testing_a_capable_server_saves_its_features() {
    let mut server = TestServer::start(vec![
        models(),
        sse_text("ready"),
        sse_tool_call(),
        sse_text(r#"{"status":"ok","number":42}"#),
    ])
    .await;
    let (_dir, store) = temp_store().await;
    let credentials = credentials();
    let created = create(&store, &credentials, local(server.url.as_str()))
        .await
        .expect("create");

    let report = test(&store, &credentials, created.id)
        .await
        .expect("report");

    let outcomes: Vec<(CheckKind, CheckOutcome)> =
        report.checks.iter().map(|c| (c.kind, c.outcome)).collect();
    assert_eq!(
        outcomes,
        vec![
            (CheckKind::Endpoint, CheckOutcome::Passed),
            (CheckKind::Model, CheckOutcome::Passed),
            (CheckKind::Answer, CheckOutcome::Passed),
            (CheckKind::ToolCalls, CheckOutcome::Passed),
            (CheckKind::StructuredOutput, CheckOutcome::Passed),
        ]
    );
    assert_eq!(report.models.len(), 2);
    assert!(report.answer_ms.is_some());
    let features = ProviderFeatures {
        tool_calls: true,
        structured_output: StructuredOutput::JsonSchema,
    };
    assert_eq!(report.features, Some(features));
    let details = list(&store, &credentials).await.expect("list");
    assert_eq!(details[0].features, features);
    assert!(details[0].checked_at.is_some());

    // The checks send fixed prompts to the configured model, and the schema in strict mode.
    let _ = server.request().await;
    let answer = server.request().await.json();
    assert_eq!(answer["model"], json!("qwen2.5"));
    assert_eq!(answer["max_tokens"], json!(TEST_MAX_TOKENS));
    let tools = server.request().await.json();
    assert_eq!(
        tools["tools"][0]["function"]["name"],
        json!("report_status")
    );
    let structured = server.request().await.json();
    assert_eq!(structured["response_format"]["type"], json!("json_schema"));
}

#[tokio::test]
async fn testing_falls_back_to_what_a_server_supports() {
    let server = TestServer::start(vec![
        // No model list, tools rejected, JSON schemas rejected, JSON mode works.
        Reply::json(404, "404 page not found"),
        sse_text("ready"),
        Reply::json(400, r#"{"error":{"message":"tools are not supported"}}"#),
        Reply::json(
            400,
            r#"{"error":{"message":"response_format json_schema unsupported"}}"#,
        ),
        sse_text(r#"{"status":"ok","number":42}"#),
    ])
    .await;
    let (_dir, store) = temp_store().await;
    let credentials = credentials();
    let created = create(&store, &credentials, local(server.url.as_str()))
        .await
        .expect("create");

    let report = test(&store, &credentials, created.id)
        .await
        .expect("report");

    let outcomes: Vec<(CheckKind, CheckOutcome)> =
        report.checks.iter().map(|c| (c.kind, c.outcome)).collect();
    assert_eq!(
        outcomes,
        vec![
            (CheckKind::Endpoint, CheckOutcome::Warning),
            (CheckKind::Model, CheckOutcome::Skipped),
            (CheckKind::Answer, CheckOutcome::Passed),
            (CheckKind::ToolCalls, CheckOutcome::Failed),
            (CheckKind::StructuredOutput, CheckOutcome::Warning),
        ]
    );
    assert_eq!(
        report.features,
        Some(ProviderFeatures {
            tool_calls: false,
            structured_output: StructuredOutput::JsonObject,
        })
    );
}

#[tokio::test]
async fn a_rejected_key_stops_the_test_early() {
    let server = TestServer::start(vec![Reply::json(
        401,
        r#"{"error":{"message":"Incorrect API key provided: sk-wrong-key-123"}}"#,
    )])
    .await;
    let (_dir, store) = temp_store().await;
    let credentials = credentials();
    let created = create(&store, &credentials, local(server.url.as_str()))
        .await
        .expect("create");
    save_key(
        &store,
        &credentials,
        created.id,
        SecretString::from("sk-wrong-key-123"),
    )
    .await
    .expect("key");

    let report = test(&store, &credentials, created.id)
        .await
        .expect("report");

    assert_eq!(report.checks[0].outcome, CheckOutcome::Failed);
    assert!(!report.checks[0].detail.contains("sk-wrong-key-123"));
    assert!(
        report.checks[1..]
            .iter()
            .all(|c| c.outcome == CheckOutcome::Skipped)
    );
    assert_eq!(report.features, None);
    let profile = store
        .providers()
        .get(created.id)
        .await
        .expect("get")
        .expect("exists");
    assert_eq!(profile.checked, None);
}

/// Distinctive enough that finding it anywhere means it leaked.
const LEAK_KEY: &str = "sk-leakcheck-9f8e7d6c5b4a3210";

fn assert_absent_from_files(dir: &std::path::Path) {
    for entry in std::fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("entry").path();
        if path.is_file() {
            let bytes = std::fs::read(&path).expect("read file");
            assert!(
                !bytes
                    .windows(LEAK_KEY.len())
                    .any(|window| window == LEAK_KEY.as_bytes()),
                "key found in {}",
                path.display()
            );
        }
    }
}

#[derive(Clone, Default)]
struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .expect("capture lock")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn keys_never_reach_the_database_files_or_logs() {
    let capture = Capture::default();
    let writer = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || writer.clone())
        .finish();
    // See connection_secrets: a second, idle dispatcher keeps callsites from being cached as
    // disabled by tests running in parallel.
    let _idle = tracing::Dispatch::new(tracing_subscriber::registry());
    let _guard = tracing::subscriber::set_default(subscriber);

    let mut server = TestServer::start(vec![Reply::json(
        401,
        &format!(r#"{{"error":{{"message":"Incorrect API key provided: {LEAK_KEY}"}}}}"#),
    )])
    .await;
    let (dir, store) = temp_store().await;
    let credentials = Credentials::in_memory_as_os_store();
    let created = create(&store, &credentials, local(server.url.as_str()))
        .await
        .expect("create");

    let backend = save_key(
        &store,
        &credentials,
        created.id,
        SecretString::from(LEAK_KEY),
    )
    .await
    .expect("save key");
    let report = test(&store, &credentials, created.id)
        .await
        .expect("report");
    let details = list(&store, &credentials).await.expect("list");
    forget_key(&store, &credentials, created.id)
        .await
        .expect("forget");

    assert_eq!(backend, CredentialBackend::OsStore);
    assert_eq!(details[0].key, KeyStorage::OsStore);
    // The key was used: it went to the provider in the authorization header, and nowhere else.
    let sent = server.request().await;
    assert_eq!(
        sent.header("authorization"),
        Some(format!("Bearer {LEAK_KEY}").as_str())
    );
    assert!(!sent.body.contains(LEAK_KEY));
    assert!(
        !serde_json::to_string(&report)
            .expect("json")
            .contains(LEAK_KEY)
    );
    assert!(
        !serde_json::to_string(&details)
            .expect("json")
            .contains(LEAK_KEY)
    );
    assert_absent_from_files(dir.path());
    let logs = String::from_utf8(capture.0.lock().expect("lock").clone()).expect("utf-8 logs");
    assert!(logs.contains("provider key saved"), "{logs}");
    assert!(!logs.contains(LEAK_KEY), "{logs}");
}

#[tokio::test]
async fn deleting_the_chosen_provider_turns_ai_off() {
    use crate::domain::settings::{AiSettings, SettingsPatch};
    use crate::services::settings;

    let (_dir, store) = temp_store().await;
    let credentials = credentials();
    let created = create(&store, &credentials, local("http://localhost:8080/v1"))
        .await
        .expect("create");
    let on = AiSettings {
        enabled: true,
        provider_id: Some(created.id),
    };
    settings::update(
        &store,
        SettingsPatch {
            ai: Some(on),
            ..SettingsPatch::default()
        },
    )
    .await
    .expect("turn on");

    delete(&store, &credentials, created.id)
        .await
        .expect("delete");

    assert_eq!(
        settings::load(&store).await.expect("load").ai,
        AiSettings::default()
    );
}
