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
mod diagnostics;
mod external;

use tauri::Runtime;
use tauri::ipc::Invoke;

/// The complete list of commands the webview may invoke.
pub(crate) fn handler<R: Runtime>() -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        app::app_info,
        diagnostics::diagnostics,
        external::open_external_url
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
