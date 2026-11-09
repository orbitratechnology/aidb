//! Reading pages of rows, within the Safety settings' row and size limits.
//!
//! Row values pass through to the UI and are never logged, cached or written to disk.

use std::time::Instant;

use super::sessions::{Sessions, not_connected};
use super::settings;
use crate::adapters::database::{DatabaseAdapter, DatabaseSession};
use crate::adapters::local_store::LocalStore;
use crate::domain::connections::ConnectionId;
use crate::domain::database::OperationId;
use crate::domain::settings::SafetySettings;
use crate::domain::table_data::{PageLimits, PageRequest, TablePage};
use crate::errors::AppResult;

const BYTES_PER_MEGABYTE: usize = 1024 * 1024;

/// The page limits the Safety settings allow.
#[must_use]
pub fn page_limits(safety: &SafetySettings) -> PageLimits {
    PageLimits {
        max_rows: safety.max_rows_per_page,
        max_bytes: usize::try_from(safety.max_result_megabytes)
            .unwrap_or(usize::MAX)
            .saturating_mul(BYTES_PER_MEGABYTE),
    }
}

/// One page of a table or view of the open connection.
///
/// # Errors
///
/// `invalid_input` for a malformed request, an unknown column or a value its column cannot
/// hold; `not_found` if the object is gone; `permission_denied` if the role cannot read it;
/// `unavailable` when the connection is not open; `cancelled` or `timeout` as the database
/// reports.
pub async fn read_page<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    id: ConnectionId,
    request: PageRequest,
) -> AppResult<TablePage> {
    request.validate()?;
    let limits = page_limits(&settings::load(store).await?.safety);
    let session = sessions.get(id).ok_or_else(not_connected)?;
    let started = Instant::now();
    let page = session.read_page(&request, limits).await?;
    // Counts only: names and values stay out of the log.
    tracing::info!(
        connection = %id,
        rows = page.rows.len(),
        has_more = page.has_more,
        byte_limit_reached = page.byte_limit_reached,
        elapsed_ms = started.elapsed().as_millis(),
        "page read"
    );
    Ok(page)
}

/// Asks the database to stop a running operation. `false` if it already finished.
///
/// # Errors
///
/// `unavailable` when the connection is not open, or if the cancel request cannot be sent.
pub async fn cancel<A: DatabaseAdapter>(
    sessions: &Sessions<A>,
    id: ConnectionId,
    operation: OperationId,
) -> AppResult<bool> {
    let session = sessions.get(id).ok_or_else(not_connected)?;
    Ok(session.cancel(operation).await?)
}

#[cfg(test)]
mod tests {
    use std::sync::PoisonError;

    use super::*;
    use crate::adapters::local_store::test_support::temp_store;
    use crate::domain::settings::SettingsPatch;
    use crate::errors::ErrorCode;
    use crate::services::fakes::{sessions, target};

    fn request(json: &str) -> PageRequest {
        serde_json::from_str(json).expect("parses")
    }

    #[tokio::test]
    async fn pages_are_read_within_the_safety_limits() {
        let (_dir, store) = temp_store().await;
        settings::update(
            &store,
            SettingsPatch {
                appearance: None,
                safety: Some(SafetySettings {
                    max_rows_per_page: 50,
                    max_result_megabytes: 2,
                    ..SafetySettings::default()
                }),
            },
        )
        .await
        .expect("save");
        let (adapter, sessions) = sessions();
        let id = ConnectionId::new();
        sessions.open(id, target("db")).await.expect("open");

        let page = read_page(
            &store,
            &sessions,
            id,
            request(r#"{"object": {"schema": "public", "name": "orders"}, "offset": 100}"#),
        )
        .await
        .expect("page");

        assert_eq!(page.offset, 100);
        let (_, limits) = adapter
            .catalog
            .last_page
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .expect("read");
        assert_eq!(
            limits,
            PageLimits {
                max_rows: 50,
                max_bytes: 2 * 1024 * 1024
            }
        );
    }

    #[tokio::test]
    async fn bad_requests_and_closed_connections_are_refused() {
        let (_dir, store) = temp_store().await;
        let (adapter, sessions) = sessions();
        let id = ConnectionId::new();

        let closed = read_page(
            &store,
            &sessions,
            id,
            request(r#"{"object": {"schema": "public", "name": "orders"}}"#),
        )
        .await
        .expect_err("not open");
        assert_eq!(closed.code(), ErrorCode::Unavailable);

        sessions.open(id, target("db")).await.expect("open");
        let malformed = read_page(
            &store,
            &sessions,
            id,
            request(
                r#"{"object": {"schema": "public", "name": "orders"},
                    "filters": [{"column": "id", "operator": "equals"}]}"#,
            ),
        )
        .await
        .expect_err("no value");
        let missing = read_page(
            &store,
            &sessions,
            id,
            request(r#"{"object": {"schema": "public", "name": "ghost"}}"#),
        )
        .await
        .expect_err("gone");

        assert_eq!(malformed.code(), ErrorCode::InvalidInput);
        assert_eq!(missing.code(), ErrorCode::NotFound);
        let (last, _) = adapter
            .catalog
            .last_page
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .expect("read");
        assert_eq!(
            last.object.name, "ghost",
            "the malformed one never reached it"
        );
    }

    #[tokio::test]
    async fn cancelling_needs_an_open_connection() {
        let (_, sessions) = sessions();
        let id = ConnectionId::new();
        let error = cancel(&sessions, id, OperationId::new())
            .await
            .expect_err("closed");
        assert_eq!(error.code(), ErrorCode::Unavailable);

        sessions.open(id, target("db")).await.expect("open");
        assert!(
            !cancel(&sessions, id, OperationId::new())
                .await
                .expect("cancel")
        );
    }
}
