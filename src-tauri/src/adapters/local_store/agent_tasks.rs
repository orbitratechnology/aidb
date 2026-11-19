//! Assistant tasks: the question, the plan with every step, and the answer. They hold whatever
//! the assistant read, so they stay on this device and can be cleared in Settings › Storage.

use sqlx::SqlitePool;

use super::StoreError;
use crate::domain::agent::plan::{TaskId, TaskPlan};
use crate::domain::agent::task::{MAX_ANSWER_CHARS, TaskStatus, TaskSummary};

/// Longest title kept, in characters (the table allows 500).
const TITLE_CHARS: usize = 200;

type TaskRow = (String, Option<String>, String, String, String, String);
/// [`TaskRow`] with the plan and the answer.
type FullTaskRow = (
    String,
    Option<String>,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
);

/// A stored task: its summary, plan and answer.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredTask {
    pub summary: TaskSummary,
    /// `None` when the stored plan is from another version or unreadable.
    pub plan: Option<TaskPlan>,
    pub answer: Option<String>,
}

/// Repository for `agent_tasks`.
#[derive(Debug, Clone, Copy)]
pub struct AgentTasks<'a> {
    pool: &'a SqlitePool,
}

fn title(intent: &str) -> String {
    let single: String = intent.split_whitespace().collect::<Vec<_>>().join(" ");
    match single.char_indices().nth(TITLE_CHARS) {
        Some((at, _)) => format!("{}…", &single[..at]),
        None => single,
    }
}

fn summary(row: TaskRow) -> Result<TaskSummary, StoreError> {
    let (id, connection_id, title, status, created_at, updated_at) = row;
    let corrupt =
        |field: &str, error: String| StoreError::Corrupt(format!("task {field}: {error}"));
    Ok(TaskSummary {
        id: id
            .parse()
            .map_err(|e: uuid::Error| corrupt("id", e.to_string()))?,
        // A profile id that no longer parses is treated as a deleted connection.
        connection_id: connection_id.and_then(|id| id.parse().ok()),
        title,
        status: TaskStatus::parse(&status).map_err(|e| corrupt("status", e))?,
        created_at,
        updated_at,
    })
}

impl<'a> AgentTasks<'a> {
    pub(super) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    /// Records a task that has started. Its connection may have been deleted meanwhile; the
    /// task is then kept without it.
    ///
    /// # Errors
    ///
    /// A query error.
    pub async fn start(&self, plan: &TaskPlan) -> Result<(), StoreError> {
        let json = plan
            .to_json()
            .map_err(|e| StoreError::Corrupt(e.to_string()))?;
        sqlx::query(
            "INSERT INTO agent_tasks (id, connection_id, title, status, plan_json) \
             VALUES (?, (SELECT id FROM connection_profiles WHERE id = ?), ?, ?, ?)",
        )
        .bind(plan.task_id.to_string())
        .bind(plan.target.connection_id.to_string())
        .bind(title(&plan.intent))
        .bind(TaskStatus::Executing.as_str())
        .bind(json)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Records the task's status, plan and answer.
    ///
    /// # Errors
    ///
    /// [`StoreError::NotFound`] if the task is gone (cleared meanwhile).
    pub async fn update(
        &self,
        plan: &TaskPlan,
        status: TaskStatus,
        answer: Option<&str>,
    ) -> Result<(), StoreError> {
        let json = plan
            .to_json()
            .map_err(|e| StoreError::Corrupt(e.to_string()))?;
        let answer: Option<String> =
            answer.map(|answer| answer.chars().take(MAX_ANSWER_CHARS).collect());
        let result = sqlx::query(
            "UPDATE agent_tasks SET status = ?, plan_json = ?, summary = ?, \
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(json)
        .bind(answer)
        .bind(plan.task_id.to_string())
        .execute(self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    /// # Errors
    ///
    /// A query error, or a damaged row.
    pub async fn get(&self, id: TaskId) -> Result<Option<StoredTask>, StoreError> {
        let row: Option<FullTaskRow> = sqlx::query_as(
            "SELECT id, connection_id, title, status, created_at, updated_at, plan_json, summary \
                 FROM agent_tasks WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(self.pool)
        .await?;
        row.map(
            |(id, connection, title, status, created, updated, plan, answer)| {
                Ok(StoredTask {
                    summary: summary((id, connection, title, status, created, updated))?,
                    plan: plan.and_then(|json| TaskPlan::from_json(&json).ok()),
                    answer,
                })
            },
        )
        .transpose()
    }

    /// The most recent tasks first; tasks of later phases' statuses are skipped.
    ///
    /// # Errors
    ///
    /// A query error.
    pub async fn list(&self, limit: u32) -> Result<Vec<TaskSummary>, StoreError> {
        let rows: Vec<TaskRow> = sqlx::query_as(
            "SELECT id, connection_id, title, status, created_at, updated_at FROM agent_tasks \
             ORDER BY created_at DESC, id LIMIT ?",
        )
        .bind(i64::from(limit))
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| summary(row).ok())
            .collect())
    }

    /// Marks tasks left running by a previous run of the app (it quit mid-task) as cancelled.
    /// Returns how many there were.
    ///
    /// # Errors
    ///
    /// A query error.
    pub async fn close_abandoned(&self) -> Result<u64, StoreError> {
        let result = sqlx::query(
            "UPDATE agent_tasks SET status = 'cancelled', \
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
             WHERE status IN ('executing', 'verifying')",
        )
        .execute(self.pool)
        .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::temp_store;
    use super::*;
    use crate::domain::agent::plan::PlanTarget;
    use crate::domain::agent::risk::READ_ONLY_AGENT_MAX_RISK;
    use crate::domain::agent::tools::ToolInvocation;
    use crate::domain::connections::{ConnectionDraft, ConnectionId, Environment};

    fn plan(connection_id: ConnectionId, intent: &str) -> TaskPlan {
        TaskPlan::new(
            PlanTarget {
                connection_id,
                database: "shop".into(),
                environment: Environment::Development,
                read_only: true,
            },
            intent,
        )
        .expect("plan")
    }

    #[tokio::test]
    async fn tasks_are_recorded_and_finished() {
        let (_dir, store) = temp_store().await;
        let profile = store
            .connections()
            .insert(
                &ConnectionDraft::new("Shop", "db", "shop", "reader")
                    .validated()
                    .expect("valid"),
            )
            .await
            .expect("profile");
        let mut task = plan(profile.id, "How   many\\norders?");
        store.agent_tasks().start(&task).await.expect("start");
        task.add_step(&ToolInvocation::ListSchemas, READ_ONLY_AGENT_MAX_RISK)
            .expect("step");
        store
            .agent_tasks()
            .update(&task, TaskStatus::Succeeded, Some("There are 12 orders."))
            .await
            .expect("update");

        let stored = store
            .agent_tasks()
            .get(task.task_id)
            .await
            .expect("get")
            .expect("exists");
        assert_eq!(stored.summary.title, "How many\\norders?");
        assert_eq!(stored.summary.status, TaskStatus::Succeeded);
        assert_eq!(stored.summary.connection_id, Some(profile.id));
        assert_eq!(stored.plan, Some(task.clone()));
        assert_eq!(stored.answer.as_deref(), Some("There are 12 orders."));
        assert_eq!(store.agent_tasks().list(10).await.expect("list").len(), 1);
    }

    #[tokio::test]
    async fn tasks_survive_a_deleted_connection_and_abandoned_ones_are_closed() {
        let (_dir, store) = temp_store().await;
        // The connection does not exist (deleted meanwhile): the task is kept without it.
        let task = plan(ConnectionId::new(), &"long question ".repeat(40));
        store.agent_tasks().start(&task).await.expect("start");
        let listed = store.agent_tasks().list(10).await.expect("list");
        assert_eq!(listed[0].connection_id, None);
        assert!(listed[0].title.ends_with('…'));
        assert_eq!(listed[0].title.chars().count(), TITLE_CHARS + 1);

        assert_eq!(
            store.agent_tasks().close_abandoned().await.expect("close"),
            1
        );
        let stored = store
            .agent_tasks()
            .get(task.task_id)
            .await
            .expect("get")
            .expect("exists");
        assert_eq!(stored.summary.status, TaskStatus::Cancelled);
        assert!(matches!(
            store
                .agent_tasks()
                .update(
                    &plan(ConnectionId::new(), "other"),
                    TaskStatus::Failed,
                    None
                )
                .await,
            Err(StoreError::NotFound)
        ));
    }
}
