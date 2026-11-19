//! The audit log as the rest of the core uses it: recording never fails the operation being
//! audited (a failure is logged), and listing is bounded.

use crate::adapters::local_store::LocalStore;
use crate::domain::audit::{AuditEntry, AuditEvent, AuditFilter, MAX_LISTED_EVENTS};
use crate::errors::{AppError, AppResult};

/// Appends `event`. A storage failure is logged, not returned: the operation it describes has
/// already happened or been refused.
pub async fn record(store: &LocalStore, event: &AuditEvent) {
    if let Err(error) = store.audit().record(event).await {
        tracing::warn!(
            operation = event.operation,
            status = event.status.as_str(),
            error = %error,
            "could not record an audit event"
        );
    }
}

/// Events newest first.
///
/// # Errors
///
/// `invalid_input` for a limit of 0 or over [`MAX_LISTED_EVENTS`]; `storage` if the local
/// database cannot be read.
pub async fn list(store: &LocalStore, filter: AuditFilter) -> AppResult<Vec<AuditEntry>> {
    if filter.limit == 0 || filter.limit > MAX_LISTED_EVENTS {
        return Err(AppError::invalid_input(format!(
            "List between 1 and {MAX_LISTED_EVENTS} events at a time."
        )));
    }
    Ok(store.audit().list(&filter).await?)
}
