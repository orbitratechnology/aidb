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
mod connections;
mod diagnostics;
mod external;
mod sessions;
mod settings;

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
        sessions::active_sessions
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
        let closed =
            invoke(&main, "disconnect", json!({ "id": saved.id.to_string() })).expect("disconnect");
        let after = invoke(&main, "active_sessions", json!({})).expect("active_sessions");

        assert_eq!(report["server"]["readOnly"], true);
        assert!(report["roundTripMs"].is_u64());
        assert_eq!(without_password["code"], "permission_denied");
        assert_eq!(session["connectionId"], saved.id.to_string());
        assert_eq!(session["capabilities"]["cancelStatements"], true);
        assert_eq!(open.as_array().map(Vec::len), Some(1));
        assert_eq!(closed, true);
        assert_eq!(after.as_array().map(Vec::len), Some(0));
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
