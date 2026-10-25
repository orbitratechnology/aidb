//! Thin Tauri command adapters. Every command must:
//!
//! - take typed `Deserialize` inputs and validate them before doing any work;
//! - return [`AppResult`](crate::errors::AppResult), never raw driver or provider errors;
//! - delegate to services and keep business logic out of this module;
//! - use `#[tracing::instrument(skip_all, err(Debug))]` when fallible, so arguments (which may
//!   hold secrets) are never logged and each error is logged once, redacted.

mod app;

use tauri::Runtime;
use tauri::ipc::Invoke;

/// The complete list of commands the webview may invoke.
pub(crate) fn handler<R: Runtime>() -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    tauri::generate_handler![app::app_info]
}
