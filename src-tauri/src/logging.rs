//! Local diagnostic logging.
//!
//! Logs stay on this device. They must not contain secrets or row data: record structured,
//! non-sensitive fields, skip command arguments, and pass untrusted text through
//! [`redact`](crate::errors::redact).

use std::path::Path;

use thiserror::Error;
use tracing_appender::rolling::{InitError, RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::{SubscriberInitExt, TryInitError};
use tracing_subscriber::{EnvFilter, fmt};

use crate::errors::redact;

/// Environment variable that overrides the log filter, for example `AIDB_LOG=debug`.
pub const FILTER_ENV: &str = "AIDB_LOG";

const LOG_FILE_PREFIX: &str = "aidb";
const KEPT_LOG_FILES: usize = 7;

#[derive(Debug, Error)]
pub enum LoggingError {
    #[error("could not open the log directory")]
    Appender(#[from] InitError),
    #[error("a global logger is already installed")]
    AlreadyInstalled(#[from] TryInitError),
}

fn default_filter() -> EnvFilter {
    let fallback = if cfg!(debug_assertions) {
        "info,aidb_lib=debug"
    } else {
        "info"
    };
    EnvFilter::try_from_env(FILTER_ENV).unwrap_or_else(|_| EnvFilter::new(fallback))
}

/// Installs the global logger: a daily-rotated file in `log_dir`, plus stderr in debug builds.
/// Also routes panics into the log.
///
/// # Errors
///
/// Fails if the log file cannot be created or a logger is already installed. Callers should
/// keep running without file logs rather than refuse to start.
pub fn init(log_dir: &Path) -> Result<(), LoggingError> {
    let file = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILE_PREFIX)
        .filename_suffix("log")
        .max_log_files(KEPT_LOG_FILES)
        .build(log_dir)?;

    // Writes are synchronous so the last lines before a crash are never lost.
    let file_layer = fmt::layer().with_ansi(false).with_writer(file);
    let stderr_layer = cfg!(debug_assertions).then(|| fmt::layer().with_writer(std::io::stderr));

    tracing_subscriber::registry()
        .with(default_filter())
        .with(file_layer)
        .with(stderr_layer)
        .try_init()?;

    install_panic_hook();
    Ok(())
}

/// Installs a stderr-only logger when file logging is unavailable.
pub fn init_stderr_only() {
    let installed = tracing_subscriber::registry()
        .with(default_filter())
        .with(fmt::layer().with_writer(std::io::stderr))
        .try_init();
    if installed.is_ok() {
        install_panic_hook();
    }
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|location| format!("{}:{}", location.file(), location.line()));
        let message = info.payload_as_str().unwrap_or("non-text panic payload");
        tracing::error!(
            location = location.as_deref().unwrap_or("unknown"),
            message = %redact(message),
            "panic"
        );
        previous(info);
    }));
}
