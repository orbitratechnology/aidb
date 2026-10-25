//! aidb native core.
//!
//! The Rust core owns database connections, credentials, policy enforcement and local
//! persistence. The webview reaches it only through the commands registered in [`commands`].
//!
//! Layout:
//! - `commands`: thin IPC adapters (validation, delegation, error mapping).
//! - [`errors`]: the error contract sent to the webview, and secret redaction.
//! - [`logging`]: local, redacted diagnostic logs.
//! - [`state`]: state shared with commands.
//!
//! Feature code lands in `domain/` (rules and types), `services/` (use cases) and `adapters/`
//! (PostgreSQL, local store, credentials, AI providers) as each feature is built. Commands never
//! hold business logic.

mod app;
mod commands;
pub mod errors;
pub mod logging;
pub mod state;

pub use app::run;
