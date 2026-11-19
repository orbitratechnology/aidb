//! Thin Tauri command adapters. Every command must:
//!
//! - take typed `Deserialize` inputs and validate them before doing any work;
//! - return [`AppResult`](crate::errors::AppResult), never raw driver or provider errors;
//! - delegate to services and keep business logic out of this module;
//! - use `#[tracing::instrument(skip_all, err(Debug))]` when fallible, so arguments (which may
//!   hold secrets) are never logged and each error is logged once, redacted.
//!
//! Adding a command takes three steps: register it in [`handler`], list it in `build.rs` (which
//! generates its `allow-*` permission), and grant that permission in a capability file. A command
//! missing from a capability is rejected by Tauri before it runs.

mod app;
mod assistant;
mod audit;
mod catalog;
mod connections;
mod data_changes;
mod diagnostics;
mod export;
mod external;
mod library;
mod providers;
mod query;
mod sessions;
mod settings;
mod table_data;

use tauri::Runtime;
use tauri::ipc::Invoke;

/// The complete list of commands the webview may invoke.
pub(crate) fn handler<R: Runtime>() -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        app::app_info,
        diagnostics::diagnostics,
        external::open_external_url,
        settings::get_settings,
        settings::update_settings,
        settings::storage_summary,
        settings::clear_local_data,
        connections::list_connections,
        connections::create_connection,
        connections::update_connection,
        connections::delete_connection,
        connections::save_connection_password,
        connections::forget_connection_password,
        connections::parse_connection_uri,
        connections::export_connections,
        connections::preview_connection_import,
        connections::import_connections,
        sessions::test_connection,
        sessions::connect,
        sessions::disconnect,
        sessions::active_sessions,
        catalog::catalog_overview,
        catalog::refresh_catalog,
        catalog::describe_object,
        table_data::read_table_page,
        table_data::cancel_operation,
        export::export_table,
        query::analyze_sql,
        query::run_query,
        library::list_query_history,
        library::delete_query_history_entry,
        library::list_saved_queries,
        library::save_query,
        library::delete_saved_query,
        providers::list_providers,
        providers::create_provider,
        providers::update_provider,
        providers::delete_provider,
        providers::save_provider_key,
        providers::forget_provider_key,
        providers::test_provider,
        assistant::preview_assistant_context,
        assistant::ask_assistant,
        assistant::cancel_assistant_task,
        audit::list_audit_events,
        data_changes::preview_data_change,
        data_changes::apply_data_change,
        data_changes::cancel_data_change
    ]
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use tauri::Manager;
    use tauri::ipc::{CallbackFn, InvokeBody};
    use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder};
    use tauri::webview::InvokeRequest;
    use tauri::{App, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

    use super::handler;

    fn app() -> App<MockRuntime> {
        mock_builder()
            .plugin(tauri_plugin_dialog::init())
            .invoke_handler(handler())
            .build(tauri::generate_context!())
            .expect("mock app builds")
    }

    fn window(app: &App<MockRuntime>, label: &str) -> WebviewWindow<MockRuntime> {
        app.get_webview_window(label).unwrap_or_else(|| {
            WebviewWindowBuilder::new(app, label, WebviewUrl::default())
                .build()
                .expect("mock window builds")
        })
    }

    fn invoke(window: &WebviewWindow<MockRuntime>, cmd: &str, body: Value) -> Result<Value, Value> {
        let request = InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().expect("app URL parses"),
            body: InvokeBody::Json(body),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        };
        get_ipc_response(window, request)
            .map(|body| body.deserialize::<Value>().expect("response deserializes"))
    }

    #[test]
    fn granted_commands_run_in_the_main_window() {
        let app = app();
        let main = window(&app, "main");

        let info = invoke(&main, "app_info", json!({})).expect("app_info is allowed");

        assert_eq!(info["name"], "aidb");
    }

    #[test]
    fn command_errors_cross_the_boundary_as_structured_payloads() {
        let app = app();
        let main = window(&app, "main");

        let error = invoke(
            &main,
            "open_external_url",
            json!({ "url": "file:///etc/passwd" }),
        )
        .expect_err("unsafe URL is rejected");

        assert_eq!(error["code"], "invalid_input");
        assert_eq!(error["retryable"], false);
    }

    #[test]
    fn connection_uris_are_parsed_without_state_or_logging() {
        let app = app();
        let main = window(&app, "main");

        let parsed = invoke(
            &main,
            "parse_connection_uri",
            json!({ "uri": "postgres://reader:pw@db.internal:6432/orders?sslmode=require" }),
        )
        .expect("parse_connection_uri is allowed");
        let rejected = invoke(
            &main,
            "parse_connection_uri",
            json!({ "uri": "mysql://reader:pw@db.internal/orders" }),
        )
        .expect_err("not PostgreSQL");

        assert_eq!(parsed["host"], "db.internal");
        assert_eq!(parsed["port"], 6432);
        assert_eq!(parsed["sslMode"], "require");
        assert_eq!(parsed["password"], "pw");
        assert_eq!(rejected["code"], "invalid_input");
        assert!(
            !rejected["message"]
                .as_str()
                .unwrap_or_default()
                .contains("pw")
        );
    }

    /// Nothing the webview can call approves, applies or runs a change: such commands do not
    /// exist, and no capability grants one. Approvals are issued and used inside the core only.
    #[test]
    fn nothing_in_the_webview_can_approve_or_write() {
        let app = app();
        let main = window(&app, "main");
        for cmd in [
            "approve",
            "approve_change",
            "request_approval",
            "consume_approval",
            "apply_change",
            "execute_write",
            "run_write",
            "run_sql",
            "plugin:sql|execute",
            "plugin:shell|execute",
        ] {
            let error = invoke(&main, cmd, json!({})).expect_err(cmd);
            let text = error.as_str().unwrap_or_default().to_lowercase();
            assert!(
                text.contains("not allowed") || text.contains("not found"),
                "{cmd}: {error}"
            );
        }

        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../../capabilities/default.json"))
                .expect("capability parses");
        let granted = capability["permissions"]
            .as_array()
            .expect("permissions")
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>();
        // The one write path: the user applies a change previewed in the core, which holds the
        // approval and its token. Nothing else may approve, execute or write.
        const USER_APPROVED_WRITES: [&str; 1] = ["allow-apply-data-change"];
        for permission in granted
            .iter()
            .filter(|permission| !USER_APPROVED_WRITES.contains(permission))
        {
            for word in ["approve", "apply", "execute", "write", "consume", "policy"] {
                assert!(!permission.contains(word), "{permission} grants {word}");
            }
        }
        assert!(granted.contains(&"allow-run-query"), "reads stay available");
    }

    /// The dialog plugin is registered (the core uses it), but no capability grants it.
    #[test]
    fn dialog_and_file_plugins_are_not_reachable_from_the_webview() {
        let app = app();
        let main = window(&app, "main");

        for cmd in [
            "plugin:dialog|open",
            "plugin:dialog|save",
            "plugin:fs|read_file",
        ] {
            let error = invoke(&main, cmd, json!({})).expect_err(cmd);
            assert!(
                error
                    .as_str()
                    .is_some_and(|text| text.contains("not allowed")),
                "{cmd}: {error}"
            );
        }
    }

    /// Runs the session commands against the PostgreSQL fixture through the real IPC path, to
    /// check argument names and response shapes. See `tests/postgres.rs` for the fixture.
    #[test]
    #[ignore = "needs PostgreSQL: run scripts/postgres-fixture.sh start"]
    fn session_commands_work_over_ipc() {
        use crate::adapters::credentials::Credentials;
        use crate::adapters::local_store::test_support::temp_store;
        use crate::domain::connection_uri;
        use crate::domain::connections::{ConnectionDraft, SslMode};
        use crate::state::{AppPaths, AppState, LocalStoreState};
        use secrecy::ExposeSecret;

        let url = std::env::var("AIDB_TEST_POSTGRES_URL").expect("fixture env is loaded");
        let parsed = connection_uri::parse(&url).expect("fixture URL parses");
        let password = parsed.password.expect("fixture password").0;
        let (dir, store) = tauri::async_runtime::block_on(temp_store());
        let mut settings = ConnectionDraft::new(
            "Fixture",
            parsed.host,
            parsed.database.expect("database"),
            parsed.username.expect("user"),
        );
        settings.port = parsed.port;
        settings.ssl_mode = SslMode::Disable;
        let saved = tauri::async_runtime::block_on(store.connections().insert(&settings))
            .expect("profile saved");
        let state = AppState::new(
            AppPaths::under(dir.path()),
            LocalStoreState::Ready(store),
            Credentials::session_only("tests"),
        );
        let app = mock_builder()
            .manage(state)
            .invoke_handler(handler())
            .build(tauri::generate_context!())
            .expect("mock app builds");
        let main = window(&app, "main");
        let connection = serde_json::to_value(&settings).expect("draft serializes");

        let report = invoke(
            &main,
            "test_connection",
            json!({
                "connection": connection,
                "password": password.expose_secret(),
                "savedId": null,
                "allowUnencrypted": false,
            }),
        )
        .expect("test_connection");
        let without_password = invoke(
            &main,
            "connect",
            json!({ "id": saved.id.to_string(), "password": null }),
        )
        .expect_err("no password saved");
        let session = invoke(
            &main,
            "connect",
            json!({ "id": saved.id.to_string(), "password": password.expose_secret() }),
        )
        .expect("connect");
        let open = invoke(&main, "active_sessions", json!({})).expect("active_sessions");
        let catalog = invoke(
            &main,
            "catalog_overview",
            json!({ "connectionId": saved.id.to_string() }),
        )
        .expect("catalog_overview");
        let refreshed = invoke(
            &main,
            "refresh_catalog",
            json!({ "connectionId": saved.id.to_string() }),
        )
        .expect("refresh_catalog");
        let missing = invoke(
            &main,
            "describe_object",
            json!({
                "connectionId": saved.id.to_string(),
                "object": { "schema": "public", "name": "aidb_no_such_table" },
            }),
        )
        .expect_err("no such table");
        let page = invoke(
            &main,
            "read_table_page",
            json!({
                "connectionId": saved.id.to_string(),
                "request": {
                    "object": { "schema": "pg_catalog", "name": "pg_am" },
                    "filters": [{ "column": "amname", "operator": "equals", "value": "btree" }],
                    "limit": 5,
                },
            }),
        )
        .expect("read_table_page");
        let bad_column = invoke(
            &main,
            "read_table_page",
            json!({
                "connectionId": saved.id.to_string(),
                "request": {
                    "object": { "schema": "pg_catalog", "name": "pg_am" },
                    "sort": [{ "column": "amname; DROP TABLE x", "direction": "asc" }],
                },
            }),
        )
        .expect_err("unknown column");
        let cancelled = invoke(
            &main,
            "cancel_operation",
            json!({
                "connectionId": saved.id.to_string(),
                "operationId": "6f1c2a40-0000-4000-8000-000000000000",
            }),
        )
        .expect("cancel_operation");
        let query = invoke(
            &main,
            "run_query",
            json!({
                "connectionId": saved.id.to_string(),
                "request": { "sql": "SELECT 1 AS one;" },
            }),
        )
        .expect("run_query");
        let refused = invoke(
            &main,
            "run_query",
            json!({
                "connectionId": saved.id.to_string(),
                "request": { "sql": "DROP TABLE pg_am" },
            }),
        )
        .expect("refusals are outcomes");
        let audit = invoke(
            &main,
            "list_audit_events",
            json!({ "filter": { "connectionId": saved.id.to_string(), "limit": 10 } }),
        )
        .expect("list_audit_events");
        let blocked_change = invoke(
            &main,
            "preview_data_change",
            json!({
                "connectionId": saved.id.to_string(),
                "change": {
                    "kind": "delete",
                    "table": { "schema": "pg_catalog", "name": "pg_am" },
                    "filters": [{ "column": "amname", "operator": "equals", "value": "none" }],
                },
            }),
        )
        .expect_err("the connection is read-only");
        let unfiltered_change = invoke(
            &main,
            "preview_data_change",
            json!({
                "connectionId": saved.id.to_string(),
                "change": {
                    "kind": "delete",
                    "table": { "schema": "pg_catalog", "name": "pg_am" },
                    "filters": [],
                },
            }),
        )
        .expect_err("a delete needs a filter");
        let never_previewed = invoke(
            &main,
            "apply_data_change",
            json!({
                "connectionId": saved.id.to_string(),
                "approvalId": "6f9619ff-8b86-4d01-b42d-00c04fc964ff",
            }),
        )
        .expect_err("nothing to apply");
        let closed =
            invoke(&main, "disconnect", json!({ "id": saved.id.to_string() })).expect("disconnect");
        let after = invoke(&main, "active_sessions", json!({})).expect("active_sessions");
        let export_closed = invoke(
            &main,
            "export_table",
            json!({
                "connectionId": saved.id.to_string(),
                "request": { "object": { "schema": "pg_catalog", "name": "pg_am" }, "format": "csv" },
            }),
        )
        .expect_err("the connection is closed, so no dialog opens");

        assert_eq!(report["server"]["readOnly"], true);
        assert!(report["roundTripMs"].is_u64());
        assert_eq!(without_password["code"], "permission_denied");
        assert_eq!(session["connectionId"], saved.id.to_string());
        assert_eq!(session["capabilities"]["cancelStatements"], true);
        assert_eq!(open.as_array().map(Vec::len), Some(1));
        assert_eq!(catalog["database"], "postgres");
        assert!(
            catalog["fingerprint"]
                .as_str()
                .is_some_and(|f| f.len() == 32)
        );
        assert_eq!(refreshed["fingerprint"], catalog["fingerprint"]);
        assert_eq!(missing["code"], "not_found");
        assert_eq!(page["rows"].as_array().map(Vec::len), Some(1));
        assert_eq!(page["hasMore"], false);
        assert_eq!(bad_column["code"], "invalid_input");
        assert_eq!(cancelled, false);
        assert_eq!(query["status"], "completed");
        assert_eq!(query["rows"], json!([["1"]]));
        assert_eq!(refused["status"], "refused");
        assert_eq!(refused["kind"], "schema");
        // The refused change is on record by fingerprint, never by text.
        let event = &audit[0];
        assert_eq!(event["operation"], "sql_refused");
        assert_eq!(event["status"], "rejected");
        assert_eq!(event["risk"], "R4");
        assert_eq!(event["fingerprint"].as_str().map(str::len), Some(64));
        assert!(!audit.to_string().contains("DROP"), "{audit}");
        assert_eq!(blocked_change["code"], "permission_denied");
        assert!(
            blocked_change["message"]
                .as_str()
                .is_some_and(|m| m.contains("read-only")),
            "{blocked_change}"
        );
        assert_eq!(unfiltered_change["code"], "invalid_input");
        assert_eq!(never_previewed["code"], "permission_denied");
        assert_eq!(closed, true);
        assert_eq!(after.as_array().map(Vec::len), Some(0));
        assert_eq!(export_closed["code"], "unavailable");
    }

    #[test]
    fn sql_is_analyzed_without_a_connection() {
        let app = app();
        let main = window(&app, "main");

        let analysis = invoke(
            &main,
            "analyze_sql",
            json!({ "sql": "SELECT 1;\nDROP TABLE t" }),
        )
        .expect("analyze_sql");

        assert_eq!(analysis["statements"][0]["kind"], "read");
        assert_eq!(analysis["statements"][0]["reason"], Value::Null);
        assert_eq!(analysis["statements"][1]["kind"], "schema");
        assert_eq!(analysis["statements"][1]["span"]["start"]["line"], 2);
        assert!(
            analysis["statements"][1].get("text").is_none(),
            "internal fields stay internal"
        );
    }

    #[test]
    fn saved_queries_and_history_work_over_ipc() {
        use crate::adapters::credentials::Credentials;
        use crate::adapters::local_store::test_support::temp_store;
        use crate::state::{AppPaths, AppState, LocalStoreState};

        let (dir, store) = tauri::async_runtime::block_on(temp_store());
        let state = AppState::new(
            AppPaths::under(dir.path()),
            LocalStoreState::Ready(store),
            Credentials::session_only("tests"),
        );
        let app = mock_builder()
            .manage(state)
            .invoke_handler(handler())
            .build(tauri::generate_context!())
            .expect("mock app builds");
        let main = window(&app, "main");

        let saved = invoke(
            &main,
            "save_query",
            json!({ "draft": { "name": " Top customers ", "sql": "SELECT 1" } }),
        )
        .expect("save_query");
        let id = saved["id"].as_str().expect("id").to_owned();
        let renamed = invoke(
            &main,
            "save_query",
            json!({ "draft": { "id": id, "name": "Best customers", "sql": "SELECT 2" } }),
        )
        .expect("update");
        let listed = invoke(&main, "list_saved_queries", json!({ "search": "best" }))
            .expect("list_saved_queries");
        invoke(&main, "delete_saved_query", json!({ "id": id })).expect("delete_saved_query");
        let gone = invoke(&main, "delete_saved_query", json!({ "id": id })).expect_err("gone");
        let invalid = invoke(
            &main,
            "save_query",
            json!({ "draft": { "name": "", "sql": "SELECT 1" } }),
        )
        .expect_err("no name");
        let history = invoke(
            &main,
            "list_query_history",
            json!({ "connectionId": "6f1c2a40-0000-4000-8000-000000000000" }),
        )
        .expect("list_query_history");

        assert_eq!(saved["name"], "Top customers");
        assert_eq!(renamed["sql"], "SELECT 2");
        assert_eq!(listed.as_array().map(Vec::len), Some(1));
        assert_eq!(gone["code"], "not_found");
        assert_eq!(invalid["code"], "invalid_input");
        assert_eq!(history, json!([]));
    }

    #[test]
    fn provider_profiles_work_over_ipc_without_echoing_keys() {
        use crate::adapters::credentials::Credentials;
        use crate::adapters::local_store::test_support::temp_store;
        use crate::state::{AppPaths, AppState, LocalStoreState};

        const KEY: &str = "sk-ant-ipc-test-key-0123456789";
        let (dir, store) = tauri::async_runtime::block_on(temp_store());
        let state = AppState::new(
            AppPaths::under(dir.path()),
            LocalStoreState::Ready(store),
            Credentials::session_only("tests"),
        );
        let app = mock_builder()
            .manage(state)
            .invoke_handler(handler())
            .build(tauri::generate_context!())
            .expect("mock app builds");
        let main = window(&app, "main");
        let draft = json!({
            "name": "Claude",
            "kind": "anthropic",
            "endpoint": null,
            "model": "claude-opus-5-5",
            "requestTimeoutSecs": 120,
            "contextBudgetTokens": null,
            "allowInsecureHttp": false,
        });

        let created = invoke(&main, "create_provider", json!({ "provider": draft }))
            .expect("create_provider");
        let id = created["id"].as_str().expect("id").to_owned();
        let untested = invoke(&main, "test_provider", json!({ "id": id })).expect_err("no key");
        let backend = invoke(&main, "save_provider_key", json!({ "id": id, "key": KEY }))
            .expect("save_provider_key");
        let listed = invoke(&main, "list_providers", json!({})).expect("list_providers");
        let insecure = invoke(
            &main,
            "update_provider",
            json!({ "id": id, "provider": {
                "name": "Claude", "kind": "openai_compatible",
                "endpoint": "http://198.51.100.7/v1", "model": "m",
                "requestTimeoutSecs": 120, "contextBudgetTokens": null, "allowInsecureHttp": false,
            }}),
        )
        .expect_err("needs consent");
        let forgotten =
            invoke(&main, "forget_provider_key", json!({ "id": id })).expect("forget_provider_key");
        invoke(&main, "delete_provider", json!({ "id": id })).expect("delete_provider");
        let gone = invoke(&main, "delete_provider", json!({ "id": id })).expect_err("gone");

        assert_eq!(created["endpointUrl"], "https://api.anthropic.com/");
        assert_eq!(created["key"], "not_saved");
        assert_eq!(untested["code"], "invalid_input");
        assert_eq!(backend, "session_only");
        assert_eq!(listed[0]["key"], "session_only");
        assert!(!listed.to_string().contains(KEY));
        assert_eq!(insecure["code"], "invalid_input", "{insecure}");
        assert_eq!(forgotten, true);
        assert_eq!(gone["code"], "not_found");
    }

    #[test]
    fn unknown_commands_are_rejected() {
        let app = app();
        let main = window(&app, "main");

        assert!(invoke(&main, "run_sql", json!({ "sql": "drop table users" })).is_err());
    }

    #[test]
    fn windows_without_a_capability_cannot_invoke_commands() {
        let app = app();
        let other = window(&app, "untrusted");

        assert!(invoke(&other, "app_info", json!({})).is_err());
    }

    #[test]
    fn plugin_commands_that_were_never_granted_are_rejected() {
        let app = app();
        let main = window(&app, "main");

        let result = invoke(
            &main,
            "plugin:opener|open_url",
            json!({ "url": "https://example.com" }),
        );

        assert!(result.is_err());
    }
}
