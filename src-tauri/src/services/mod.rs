//! Use cases that coordinate domain rules with adapters. Commands call these; they never reach
//! adapters directly.

pub mod connection_secrets;
pub mod settings;

use crate::errors::{AppError, AppResult};

/// Runs blocking work (such as OS keychain calls) off the async executor.
async fn run_blocking<T, F>(work: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(AppError::internal)
}
