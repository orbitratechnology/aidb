//! Secret scanning, a release gate: no database password or provider API key may reach the
//! files the app writes (local database and its write-ahead log, logs, exports) or the
//! webview.
//!
//! The test drives the commands through the real IPC path with unique canary secrets: a
//! PostgreSQL role whose password is a canary, and a provider whose key is a canary, served by
//! a hostile endpoint that echoes the key back in its errors. It parses a URI holding the
//! password, tests and opens connections with right and wrong passwords, saves the password,
//! runs and saves queries (one failing), exports rows and profiles, tests the provider and asks
//! the assistant a question. Then it scans every file under the app's directories, and every
//! command response that is not meant to carry the secret, for the canaries: as written, in hex
//! and in base64 at every alignment.
//!
//! It installs the global logger, so it runs on its own:
//!
//! ```sh
//! scripts/postgres-fixture.sh start && source target/postgres-fixture/env
//! AIDB_LOG=trace cargo test -p aidb --lib secret_scan -- --ignored
//! ```

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sqlx::Connection;
use tauri::Manager;
use tauri::test::mock_builder;

use super::handler;
use super::tests::{invoke, window};
use crate::adapters::ai_providers::test_server::{Reply, TestServer};
use crate::adapters::credentials::Credentials;
use crate::adapters::local_store::LocalStore;
use crate::domain::agent::ask::AskRequest;
use crate::domain::catalog::ObjectName;
use crate::domain::connection_uri;
use crate::domain::connections::ConnectionId;
use crate::domain::export::{ExportFormat, ExportRequest};
use crate::services::assistant::ask::{self, Workspace};
use crate::services::{connections, table_data};
use crate::state::{AppPaths, AppState, LocalStoreState};

#[test]
#[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
fn secrets_never_reach_files_logs_or_the_webview() {
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let password = format!("pw-canary-{nonce}");
    let api_key = format!("sk-canary-{nonce}");
    let role = format!("aidb_scan_{}", &nonce[..12]);
    let table = format!("secret_scan_{}", &nonce[..12]);
    let admin_url = std::env::var("AIDB_TEST_POSTGRES_URL").expect("fixture env is loaded");
    let fixture = connection_uri::parse(&admin_url).expect("fixture URL parses");

    // The role and table come first, before the logger, so the statements that set the
    // password are not part of what is scanned.
    tauri::async_runtime::block_on(admin(
        &admin_url,
        &[
            format!("CREATE ROLE {role} LOGIN PASSWORD '{password}'"),
            format!("CREATE TABLE public.{table} (id int PRIMARY KEY, note text)"),
            format!("INSERT INTO public.{table} VALUES (1, 'one'), (2, 'two')"),
            format!("GRANT SELECT ON public.{table} TO {role}"),
        ],
    ));

    let dir = tempfile::tempdir().expect("temp dir");
    let paths = AppPaths::under(dir.path());
    let exports = dir.path().join("exports");
    for path in [paths.data_dir(), paths.log_dir(), exports.as_path()] {
        std::fs::create_dir_all(path).expect("directory");
    }
    crate::logging::init(paths.log_dir()).expect(
        "this test installs the global logger: run it on its own (cargo test --lib secret_scan)",
    );

    let store = tauri::async_runtime::block_on(LocalStore::open(paths.data_dir())).expect("store");
    let app = mock_builder()
        .manage(AppState::new(
            AppPaths::under(dir.path()),
            LocalStoreState::Ready(store),
            Credentials::session_only("secret-scan"),
        ))
        .invoke_handler(handler())
        .build(tauri::generate_context!())
        .expect("mock app builds");
    let main = window(&app, "main");
    // Responses the webview gets; none of them may carry a secret.
    let mut seen: Vec<(String, Value)> = Vec::new();
    let mut call = |cmd: &str, body: Value| -> Value {
        let response = match invoke(&main, cmd, body) {
            Ok(value) | Err(value) => value,
        };
        seen.push((cmd.to_owned(), response.clone()));
        response
    };

    // A pasted URI with the password: the parsed password goes back to the form, by design,
    // so this one response is not scanned.
    let uri = format!(
        "postgres://{role}:{password}@{}:{}/{}?sslmode=disable",
        fixture.host,
        fixture.port,
        fixture.database.as_deref().expect("database")
    );
    let parsed = invoke(&main, "parse_connection_uri", json!({ "uri": uri })).expect("parses");
    assert_eq!(parsed["password"], password.as_str());
    let draft = json!({
        "name": "Canary",
        "host": parsed["host"],
        "port": parsed["port"],
        "database": parsed["database"],
        "username": parsed["username"],
        "sslMode": "disable",
        "caCertPath": null,
        "connectTimeoutSecs": 10,
        "environment": "production",
        "readOnly": true,
    });
    // A malformed URI with the password in it.
    call(
        "parse_connection_uri",
        json!({ "uri": format!("postgres://{role}:{password}@[bad-host/db") }),
    );

    let wrong = format!("{password}-wrong");
    call(
        "test_connection",
        json!({ "connection": draft, "password": wrong, "savedId": null, "allowUnencrypted": true }),
    );
    call(
        "test_connection",
        json!({ "connection": draft, "password": password, "savedId": null, "allowUnencrypted": true }),
    );
    let created = call(
        "create_connection",
        json!({ "connection": draft, "allowUnencrypted": true }),
    );
    let id = created["id"]
        .as_str()
        .expect("connection created")
        .to_owned();
    call("connect", json!({ "id": id, "password": wrong }));
    call(
        "save_connection_password",
        json!({ "id": id, "password": password }),
    );
    call("connect", json!({ "id": id, "password": null }));

    for sql in [
        format!("SELECT * FROM public.{table} ORDER BY id"),
        "SELECT current_user, version()".to_owned(),
        format!("SELECT * FROM public.{table} WHERE nonsense"),
    ] {
        call(
            "run_query",
            json!({ "connectionId": id, "request": { "sql": sql } }),
        );
    }
    call(
        "save_query",
        json!({ "draft": { "connectionId": id, "name": "Canary rows",
            "sql": format!("SELECT * FROM public.{table}") } }),
    );
    call(
        "read_table_page",
        json!({ "connectionId": id, "request": { "object": { "schema": "public", "name": table }, "limit": 10 } }),
    );
    call("catalog_overview", json!({ "connectionId": id }));

    // Exports go through the same code as the commands, after the save dialog.
    let state = app.state::<AppState>();
    let connection_id: ConnectionId = serde_json::from_value(json!(id)).expect("id");
    for (format, name) in [
        (ExportFormat::Csv, "rows.csv"),
        (ExportFormat::Json, "rows.json"),
    ] {
        let request = ExportRequest {
            object: ObjectName {
                schema: "public".into(),
                name: table.clone(),
            },
            filters: Vec::new(),
            sort: Vec::new(),
            format,
            operation_id: None,
        };
        tauri::async_runtime::block_on(table_data::export(
            state.sessions(),
            connection_id,
            &request,
            &exports.join(name),
            1_000,
        ))
        .expect("rows exported");
    }
    let store = state.local_store().expect("store");
    let (profiles, _) = tauri::async_runtime::block_on(connections::export(store)).expect("export");
    std::fs::write(exports.join("connections.json"), profiles).expect("profiles written");

    // A hostile endpoint that echoes the key in its errors, in a body and in a header.
    let echo = || {
        let body = json!({ "error": { "message": format!("Incorrect API key provided: {api_key}"),
            "type": "invalid_request_error", "code": "invalid_api_key" } });
        Reply::json(401, &body.to_string())
            .with_header("www-authenticate", &format!("Bearer error=\"{api_key}\""))
    };
    let mut server = tauri::async_runtime::block_on(async {
        let mut replies = Vec::new();
        for _ in 0..8 {
            replies.push(echo());
            replies.push(Reply::json(
                500,
                &format!("internal error near key {api_key}"),
            ));
        }
        TestServer::start(replies).await
    });
    let provider = call(
        "create_provider",
        json!({ "provider": { "name": "Canary model", "kind": "openai_compatible",
            "endpoint": server.url.as_str(), "model": "canary", "requestTimeoutSecs": 10,
            "contextBudgetTokens": null, "allowInsecureHttp": false } }),
    );
    let provider_id = provider["id"]
        .as_str()
        .expect("provider created")
        .to_owned();
    call(
        "save_provider_key",
        json!({ "id": provider_id, "key": api_key }),
    );
    call("test_provider", json!({ "id": provider_id }));
    call(
        "update_settings",
        json!({ "patch": { "ai": { "enabled": true, "providerId": provider_id } } }),
    );
    let workspace = Workspace {
        store,
        credentials: state.credentials(),
        sessions: state.sessions(),
        cache: state.catalog(),
        running: state.assistant(),
        approvals: state.approvals(),
    };
    let answer = tauri::async_runtime::block_on(ask::ask(
        workspace,
        connection_id,
        AskRequest {
            question: "How many rows are in the canary table?".into(),
            selected: Vec::new(),
            attachments: Vec::new(),
            history: Vec::new(),
            share_data: false,
        },
        &mut |_| {},
    ));
    let answer = format!("{answer:?}");

    // Everything the webview lists afterwards.
    for (cmd, body) in [
        ("list_connections", json!({})),
        ("list_providers", json!({})),
        ("active_sessions", json!({})),
        ("get_settings", json!({})),
        ("diagnostics", json!({})),
        ("storage_summary", json!({})),
        ("list_saved_queries", json!({ "connectionId": id })),
        (
            "list_query_history",
            json!({ "connectionId": id, "search": null, "limit": 50 }),
        ),
        ("list_audit_events", json!({ "filter": { "limit": 100 } })),
    ] {
        call(cmd, body);
    }
    drop(call);
    seen.push(("ask_assistant".into(), Value::String(answer)));

    // Before closing: the newest writes may still be in the write-ahead log.
    let mut findings = scan_files(dir.path(), &[&password, &api_key]);
    call_disconnect_and_close(&app, &main, &id);
    findings.extend(scan_files(dir.path(), &[&password, &api_key]));
    for (cmd, response) in &seen {
        let text = response.to_string();
        for (secret, label) in [(&password, "password"), (&api_key, "API key")] {
            for (form, needle) in forms(secret) {
                if text.contains(&needle) {
                    findings.push(format!("{label} ({form}) in the {cmd} response: {text}"));
                }
            }
        }
    }

    // Controls, so that a pass means something: the key really went to the provider, the run
    // wrote logs and files, and the scanner finds a secret where one is.
    let first = tauri::async_runtime::block_on(server.request());
    assert_eq!(
        first.header("authorization"),
        Some(format!("Bearer {api_key}").as_str())
    );
    let logs = files_under(paths.log_dir());
    assert!(!logs.is_empty(), "no log files were written");
    let log_text: String = logs
        .iter()
        .map(|path| std::fs::read_to_string(path).unwrap_or_default())
        .collect();
    assert!(
        log_text.contains("test_connection"),
        "the commands' errors were not logged"
    );
    assert!(exports.join("rows.csv").exists() && exports.join("connections.json").exists());
    let control = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        control.path().join("leak.log"),
        format!(
            "auth=Basic {}",
            base64(format!("{role}:{password}").as_bytes())
        ),
    )
    .expect("control file");
    assert!(!scan_files(control.path(), &[&password]).is_empty());

    tauri::async_runtime::block_on(admin(
        &admin_url,
        &[
            format!("DROP TABLE public.{table}"),
            format!("DROP ROLE {role}"),
        ],
    ));
    findings.sort();
    findings.dedup();
    assert!(
        findings.is_empty(),
        "secrets found:\n{}",
        findings.join("\n")
    );
}

async fn admin(url: &str, statements: &[String]) {
    let mut connection = sqlx::PgConnection::connect(&format!("{url}?sslmode=disable"))
        .await
        .expect("fixture connection");
    for statement in statements {
        sqlx::raw_sql(sqlx::AssertSqlSafe(statement.clone()))
            .execute(&mut connection)
            .await
            .expect(statement);
    }
    connection.close().await.expect("close");
}

fn call_disconnect_and_close(
    app: &tauri::App<tauri::test::MockRuntime>,
    main: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    id: &str,
) {
    let _ = invoke(main, "disconnect", json!({ "id": id }));
    let state = app.state::<AppState>();
    if let LocalStoreState::Ready(store) = state.local_store_state() {
        tauri::async_runtime::block_on(store.close());
    }
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found
}

fn scan_files(dir: &Path, secrets: &[&str]) -> Vec<String> {
    let mut findings = Vec::new();
    for path in files_under(dir) {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        for secret in secrets {
            for (form, needle) in forms(secret) {
                if contains(&bytes, needle.as_bytes()) {
                    let name = path
                        .strip_prefix(dir)
                        .unwrap_or(&path)
                        .display()
                        .to_string();
                    findings.push(format!("{secret} ({form}) in {name}"));
                }
            }
        }
    }
    findings
}

/// The secret as written, in hex, and in base64 at each of the three alignments it can take
/// inside a longer encoded value (the characters that depend on its neighbours are left out).
fn forms(secret: &str) -> Vec<(&'static str, String)> {
    let mut forms = vec![
        ("as written", secret.to_owned()),
        ("hex", secret.bytes().map(|b| format!("{b:02x}")).collect()),
    ];
    for offset in 0..3 {
        let mut input = vec![b'\0'; offset];
        input.extend_from_slice(secret.as_bytes());
        let encoded = base64(&input);
        // Skip the characters that also encode the padding bytes, and the last block.
        let start = (offset * 4).div_ceil(3);
        let end = encoded.len().saturating_sub(4);
        if end > start {
            let standard = encoded[start..end].to_owned();
            let url_safe = standard.replace('+', "-").replace('/', "_");
            if url_safe != standard {
                forms.push(("base64url", url_safe));
            }
            forms.push(("base64", standard));
        }
    }
    forms
}

fn base64(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let bytes = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

#[test]
fn the_encodings_find_a_secret_inside_longer_values() {
    let secret = "pw-canary-0123";
    for prefix in ["", "u", "us", "user:"] {
        let encoded = base64(format!("{prefix}{secret}:tail").as_bytes());
        assert!(
            forms(secret)
                .iter()
                .any(|(form, needle)| *form == "base64" && encoded.contains(needle.as_str())),
            "{prefix}: {encoded}"
        );
    }
    assert_eq!(base64(b"Man"), "TWFu");
    assert_eq!(base64(b"Ma"), "TWE=");
    let hex = forms(secret)
        .into_iter()
        .find(|(form, _)| *form == "hex")
        .expect("hex form")
        .1;
    assert!(hex.starts_with("70772d"));
}
