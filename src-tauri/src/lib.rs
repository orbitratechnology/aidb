//! aidb native core.
//!
//! The Rust core owns database connections, credentials, policy enforcement and local
//! persistence. The webview reaches it only through the commands registered in [`commands`].
//!
//! Layout:
//! - `commands`: thin IPC adapters (validation, delegation, error mapping).
//! - [`domain`]: types and rules without I/O (connection profiles, ...).
//! - [`adapters`]: I/O: the local SQLite metadata store and the OS credential store.
//! - [`services`]: use cases that coordinate domain rules and adapters.
//! - [`errors`]: the error contract sent to the webview, and secret redaction.
//! - [`logging`]: local, redacted diagnostic logs.
//! - `navigation`: keeps the webview on the app's own content.
//! - [`state`]: state shared with commands.
//!
//! More adapters (PostgreSQL, AI providers) land as each feature is built. Commands never hold
//! business logic.

pub mod adapters;
mod app;
mod commands;
pub mod domain;
pub mod errors;
pub mod logging;
mod navigation;
pub mod services;
pub mod state;

pub use app::run;
