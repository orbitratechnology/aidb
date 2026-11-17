//! Query history and saved queries.

use sqlx::SqlitePool;

use super::StoreError;
use crate::domain::connections::ConnectionId;
use crate::domain::library::{
    HISTORY_LIMIT, HistoryEntry, HistoryStatus, NewHistoryEntry, SavedQuery, SavedQueryDraft,
};
use crate::errors::ErrorCode;

/// Most rows one listing returns.
pub const LIST_LIMIT: u32 = 200;

type HistoryRow = (
    i64,
    String,
    Option<String>,
    String,
    String,
    Option<i64>,
    Option<i64>,
    Option<String>,
);

/// Repository for `query_history`.
#[derive(Debug, Clone, Copy)]
pub struct QueryHistory<'a> {
    pool: &'a SqlitePool,
}

impl<'a> QueryHistory<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Appends an entry and drops the oldest beyond [`HISTORY_LIMIT`].
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] if the connection profile no longer exists; otherwise a query
    /// error.
    pub async fn record(&self, entry: &NewHistoryEntry) -> Result<(), StoreError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO query_history \
                 (connection_id, sql_text, status, row_count, elapsed_ms, error_code) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(entry.connection_id.to_string())
        .bind(&entry.sql)
        .bind(entry.status.as_str())
        .bind(entry.row_count.and_then(|n| i64::try_from(n).ok()))
        .bind(entry.elapsed_ms.and_then(|n| i64::try_from(n).ok()))
        .bind(entry.error_code.map(ErrorCode::as_str))
        .execute(&mut *transaction)
        .await
        .map_err(|error| match &error {
            sqlx::Error::Database(db) if db.is_foreign_key_violation() => StoreError::NotFound,
            _ => StoreError::Query(error),
        })?;
        sqlx::query(
            "DELETE FROM query_history WHERE id <= \
                 (SELECT id FROM query_history ORDER BY id DESC LIMIT 1 OFFSET ?)",
        )
        .bind(i64::from(HISTORY_LIMIT))
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// The newest entries of one connection, newest first, whose text contains `search`
    /// (ignoring case).
    ///
    /// # Errors
    ///
    /// Fails on a query error or a damaged row.
    pub async fn list(
        &self,
        connection: ConnectionId,
        search: Option<&str>,
        limit: u32,
    ) -> Result<Vec<HistoryEntry>, StoreError> {
        let rows: Vec<HistoryRow> = sqlx::query_as(
            "SELECT id, executed_at, connection_id, sql_text, status, row_count, elapsed_ms, \
                    error_code \
             FROM query_history \
             WHERE connection_id = ? AND (? IS NULL OR instr(lower(sql_text), lower(?)) > 0) \
             ORDER BY id DESC LIMIT ?",
        )
        .bind(connection.to_string())
        .bind(search)
        .bind(search)
        .bind(i64::from(limit.min(LIST_LIMIT)))
        .fetch_all(self.pool)
        .await?;
        rows.into_iter().map(history_entry).collect()
    }

    /// # Errors
    ///
    /// [`StoreError::NotFound`] if there is no such entry.
    pub async fn delete(&self, id: i64) -> Result<(), StoreError> {
        let result = sqlx::query("DELETE FROM query_history WHERE id = ?")
            .bind(id)
            .execute(self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }
}

fn history_entry(row: HistoryRow) -> Result<HistoryEntry, StoreError> {
    let (id, executed_at, connection, sql, status, row_count, elapsed_ms, error_code) = row;
    Ok(HistoryEntry {
        id,
        executed_at,
        connection_id: connection.and_then(|c| c.parse().ok()),
        sql,
        status: HistoryStatus::parse(&status).map_err(StoreError::Corrupt)?,
        row_count: row_count.and_then(|n| u64::try_from(n).ok()),
        elapsed_ms: elapsed_ms.and_then(|n| u64::try_from(n).ok()),
        error_code: error_code.and_then(|code| {
            ErrorCode::ALL
                .into_iter()
                .find(|known| known.as_str() == code)
        }),
    })
}

type SavedRow = (String, String, String, Option<String>, String, String);

/// Repository for `saved_queries`.
#[derive(Debug, Clone, Copy)]
pub struct SavedQueries<'a> {
    pool: &'a SqlitePool,
}

impl<'a> SavedQueries<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Saved queries, most recently changed first, whose name or text contains `search`.
    ///
    /// # Errors
    ///
    /// Fails on a query error.
    pub async fn list(&self, search: Option<&str>) -> Result<Vec<SavedQuery>, StoreError> {
        let rows: Vec<SavedRow> = sqlx::query_as(
            "SELECT id, name, sql_text, connection_id, created_at, updated_at \
             FROM saved_queries \
             WHERE ? IS NULL OR instr(lower(name), lower(?)) > 0 \
                   OR instr(lower(sql_text), lower(?)) > 0 \
             ORDER BY updated_at DESC, name LIMIT ?",
        )
        .bind(search)
        .bind(search)
        .bind(search)
        .bind(i64::from(LIST_LIMIT))
        .fetch_all(self.pool)
        .await?;
        Ok(rows.into_iter().map(saved_query).collect())
    }

    /// Creates a saved query, or replaces the name and text of `draft.id`.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] when updating a query that no longer exists.
    pub async fn save(&self, draft: &SavedQueryDraft) -> Result<SavedQuery, StoreError> {
        let id = match &draft.id {
            Some(id) => {
                let result = sqlx::query(
                    "UPDATE saved_queries SET name = ?, sql_text = ?, \
                         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                     WHERE id = ?",
                )
                .bind(&draft.name)
                .bind(&draft.sql)
                .bind(id)
                .execute(self.pool)
                .await?;
                if result.rows_affected() == 0 {
                    return Err(StoreError::NotFound);
                }
                id.clone()
            }
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                sqlx::query(
                    "INSERT INTO saved_queries (id, connection_id, name, sql_text) \
                     VALUES (?, (SELECT id FROM connection_profiles WHERE id = ?), ?, ?)",
                )
                .bind(&id)
                .bind(draft.connection_id.map(|c| c.to_string()))
                .bind(&draft.name)
                .bind(&draft.sql)
                .execute(self.pool)
                .await?;
                id
            }
        };
        let row: SavedRow = sqlx::query_as(
            "SELECT id, name, sql_text, connection_id, created_at, updated_at \
             FROM saved_queries WHERE id = ?",
        )
        .bind(&id)
        .fetch_one(self.pool)
        .await?;
        Ok(saved_query(row))
    }

    /// # Errors
    ///
    /// [`StoreError::NotFound`] if there is no such query.
    pub async fn delete(&self, id: &str) -> Result<(), StoreError> {
        let result = sqlx::query("DELETE FROM saved_queries WHERE id = ?")
            .bind(id)
            .execute(self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }
}

fn saved_query(row: SavedRow) -> SavedQuery {
    let (id, name, sql, connection, created_at, updated_at) = row;
    SavedQuery {
        id,
        name,
        sql,
        connection_id: connection.and_then(|c| c.parse().ok()),
        created_at,
        updated_at,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::temp_store;
    use super::*;
    use crate::domain::connections::ConnectionDraft;

    async fn profile(store: &super::super::LocalStore) -> ConnectionId {
        store
            .connections()
            .insert(
                &ConnectionDraft::new("Orders", "db", "orders", "reader")
                    .validated()
                    .expect("valid"),
            )
            .await
            .expect("insert")
            .id
    }

    fn entry(connection: ConnectionId, sql: &str) -> NewHistoryEntry {
        NewHistoryEntry {
            connection_id: connection,
            sql: sql.into(),
            status: HistoryStatus::Completed,
            row_count: Some(3),
            elapsed_ms: Some(12),
            error_code: None,
        }
    }

    #[tokio::test]
    async fn history_is_listed_newest_first_and_searchable() {
        let (_dir, store) = temp_store().await;
        let id = profile(&store).await;
        let history = store.query_history();
        history.record(&entry(id, "SELECT 1")).await.expect("first");
        let mut failed = entry(id, "select * FROM Orders");
        failed.status = HistoryStatus::Failed;
        failed.row_count = None;
        failed.error_code = Some(ErrorCode::InvalidInput);
        history.record(&failed).await.expect("second");

        let all = history.list(id, None, 50).await.expect("list");
        let found = history.list(id, Some("orders"), 50).await.expect("search");
        let other = history
            .list(ConnectionId::new(), None, 50)
            .await
            .expect("other");

        assert_eq!(all.len(), 2);
        assert_eq!(all[0].sql, "select * FROM Orders");
        assert_eq!(all[0].status, HistoryStatus::Failed);
        assert_eq!(all[0].error_code, Some(ErrorCode::InvalidInput));
        assert_eq!(all[1].row_count, Some(3));
        assert_eq!(found.len(), 1);
        assert!(other.is_empty());
    }

    #[tokio::test]
    async fn history_keeps_only_the_newest_entries() {
        let (_dir, store) = temp_store().await;
        let id = profile(&store).await;
        let history = store.query_history();
        for n in 0..HISTORY_LIMIT + 5 {
            history
                .record(&entry(id, &format!("SELECT {n}")))
                .await
                .expect("record");
        }

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM query_history")
            .fetch_one(&store.pool)
            .await
            .expect("count");
        let newest = history.list(id, None, 1).await.expect("list");

        assert_eq!(count, i64::from(HISTORY_LIMIT));
        assert_eq!(newest[0].sql, format!("SELECT {}", HISTORY_LIMIT + 4));
    }

    #[tokio::test]
    async fn forgetting_a_connection_removes_its_history() {
        let (_dir, store) = temp_store().await;
        let id = profile(&store).await;
        store
            .query_history()
            .record(&entry(id, "SELECT 1"))
            .await
            .expect("record");

        store.connections().delete(id).await.expect("delete");

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM query_history")
            .fetch_one(&store.pool)
            .await
            .expect("count");
        assert_eq!(count, 0);
        assert!(matches!(
            store.query_history().record(&entry(id, "SELECT 2")).await,
            Err(StoreError::NotFound)
        ));
    }

    #[tokio::test]
    async fn saved_queries_are_created_updated_and_deleted() {
        let (_dir, store) = temp_store().await;
        let id = profile(&store).await;
        let saved = store.saved_queries();
        let draft = SavedQueryDraft {
            id: None,
            name: "Top customers".into(),
            sql: "SELECT 1".into(),
            connection_id: Some(id),
        };

        let created = saved.save(&draft).await.expect("create");
        let updated = saved
            .save(&SavedQueryDraft {
                id: Some(created.id.clone()),
                name: "Best customers".into(),
                sql: "SELECT 2".into(),
                connection_id: None,
            })
            .await
            .expect("update");
        let found = saved.list(Some("best")).await.expect("search");
        let unknown_profile = saved
            .save(&SavedQueryDraft {
                connection_id: Some(ConnectionId::new()),
                ..draft.clone()
            })
            .await
            .expect("saved without a profile");
        saved.delete(&created.id).await.expect("delete");
        let missing = saved.delete(&created.id).await;

        assert_eq!(created.connection_id, Some(id));
        assert_eq!(
            (updated.name.as_str(), updated.sql.as_str()),
            ("Best customers", "SELECT 2")
        );
        assert_eq!(
            updated.connection_id,
            Some(id),
            "an update keeps the origin"
        );
        assert_eq!(found.len(), 1);
        assert_eq!(unknown_profile.connection_id, None);
        assert!(matches!(missing, Err(StoreError::NotFound)));
    }
}
