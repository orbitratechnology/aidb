//! Risk assessment against the open connection: reads what the SQL does, asks the database what
//! it would affect (the planner's estimate, the role's privileges, cascading deletes), and
//! applies the policy. Nothing is executed; estimates come from `EXPLAIN` without `ANALYZE`.

use super::sessions::{Sessions, not_connected};
use crate::adapters::database::{DatabaseAdapter, DatabaseSession};
use crate::adapters::local_store::LocalStore;
use crate::domain::connections::ConnectionId;
use crate::domain::policy::classify::{
    self, ImpactFacts, ImpactSource, PolicyContext, RiskAssessment,
};
use crate::domain::policy::facts::{self, Operation, SqlFacts, StatementFacts, TargetTable};
use crate::domain::sql::MAX_SQL_BYTES;
use crate::errors::{AppError, AppResult};

/// The privilege an operation needs on the tables it writes, by name.
fn needed_privileges(operation: Operation) -> &'static [&'static str] {
    match operation {
        Operation::Insert | Operation::Copy => &["INSERT"],
        Operation::Update => &["UPDATE"],
        Operation::Delete => &["DELETE"],
        Operation::Merge => &["INSERT", "UPDATE", "DELETE"],
        Operation::Truncate => &["TRUNCATE"],
        _ => &[],
    }
}

fn holds(table: &TargetTable, privilege: &str) -> bool {
    match privilege {
        "INSERT" => table.can_insert,
        "UPDATE" => table.can_update,
        "DELETE" => table.can_delete,
        "TRUNCATE" => table.can_truncate,
        _ => false,
    }
}

/// Whether to ask the planner: a data change it can plan, that the policy has not already
/// blocked for what its text says.
fn worth_estimating(statement: &StatementFacts) -> bool {
    matches!(
        statement.operation,
        Operation::Insert | Operation::Update | Operation::Delete | Operation::Merge
    ) && statement.literal_rows.is_none()
        && statement.unreadable.is_none()
        && statement.blocked_function.is_none()
        && statement.filtered != Some(false)
        && !statement.disables_safeguards
}

/// Assesses `sql` for the open connection `id`.
///
/// # Errors
///
/// `invalid_input` for text over [`MAX_SQL_BYTES`]; `not_found` for an unknown profile;
/// `unavailable` when the connection is not open. Database errors while estimating are not
/// errors: the impact counts as unknown, which raises the risk.
pub async fn assess<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    id: ConnectionId,
    sql: &str,
) -> AppResult<RiskAssessment> {
    if sql.len() > MAX_SQL_BYTES {
        return Err(AppError::invalid_input(format!(
            "The SQL is too long ({} MB at most).",
            MAX_SQL_BYTES / (1024 * 1024)
        )));
    }
    let sql_facts = facts::inspect(sql);
    let (impacts, context) = gather(store, sessions, id, &sql_facts, &[]).await?;
    Ok(log(classify::assess(&sql_facts, &impacts, context)))
}

/// Assesses data changes aidb wrote itself and applies together: each statement with the rows
/// it writes, counted beforehand instead of estimated by the planner. See
/// [`classify::assess_change_set`].
///
/// # Errors
///
/// As [`assess`].
pub async fn assess_change_set<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    id: ConnectionId,
    statements: &[(&str, u64)],
) -> AppResult<RiskAssessment> {
    let mut sql_facts = SqlFacts {
        statements: Vec::new(),
        problem: None,
    };
    let mut counted = Vec::new();
    for (sql, rows) in statements {
        let facts = facts::inspect(sql);
        // A count belongs to its statement only when the text is exactly that one write.
        let single = facts.statements.len() == 1
            && matches!(
                facts.statements[0].operation,
                Operation::Insert | Operation::Update | Operation::Delete
            );
        counted.extend(facts.statements.iter().map(|_| single.then_some(*rows)));
        sql_facts.statements.extend(facts.statements);
        if sql_facts.problem.is_none() {
            sql_facts.problem = facts.problem;
        }
    }
    let (impacts, context) = gather(store, sessions, id, &sql_facts, &counted).await?;
    Ok(log(classify::assess_change_set(
        &sql_facts, &impacts, context,
    )))
}

/// Assesses a schema migration aidb generated, its statements in the order they run. See
/// [`classify::assess_migration`].
///
/// # Errors
///
/// As [`assess`].
pub async fn assess_migration<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    id: ConnectionId,
    statements: &[String],
) -> AppResult<RiskAssessment> {
    let mut sql_facts = SqlFacts {
        statements: Vec::new(),
        problem: None,
    };
    for sql in statements {
        let facts = facts::inspect(sql);
        sql_facts.statements.extend(facts.statements);
        if sql_facts.problem.is_none() {
            sql_facts.problem = facts.problem;
        }
    }
    let (impacts, context) = gather(store, sessions, id, &sql_facts, &[]).await?;
    Ok(log(classify::assess_migration(
        &sql_facts, &impacts, context,
    )))
}

fn log(assessment: RiskAssessment) -> RiskAssessment {
    tracing::debug!(
        risk = ?assessment.risk,
        approval = ?assessment.required_approval,
        statements = assessment.statements.len(),
        "assessed"
    );
    assessment
}

/// What the database says about each statement (rows written, privileges, cascades) and where
/// it would run. `counted[i]`, when given, is statement `i`'s counted rows.
async fn gather<A: DatabaseAdapter>(
    store: &LocalStore,
    sessions: &Sessions<A>,
    id: ConnectionId,
    sql_facts: &SqlFacts,
    counted: &[Option<u64>],
) -> AppResult<(Vec<ImpactFacts>, PolicyContext)> {
    let profile = store
        .connections()
        .get(id)
        .await?
        .ok_or_else(|| AppError::not_found("The connection no longer exists."))?;
    let session = sessions.get(id).ok_or_else(not_connected)?;
    let context = PolicyContext {
        environment: profile.settings.environment,
        writes_allowed: !profile.settings.read_only && !session.server_info().in_recovery,
    };

    // Every table written, once; the database resolves them with the session's search path.
    let mut targets: Vec<String> = Vec::new();
    for statement in &sql_facts.statements {
        if needed_privileges(statement.operation).is_empty() {
            continue;
        }
        for target in &statement.targets {
            if !targets.contains(target) {
                targets.push(target.clone());
            }
        }
    }
    let tables = if targets.is_empty() {
        Vec::new()
    } else {
        session
            .write_targets(&targets)
            .await
            .unwrap_or_else(|error| {
                tracing::debug!(error = %error, "could not look up the write targets");
                Vec::new()
            })
    };

    let mut impacts = Vec::with_capacity(sql_facts.statements.len());
    for (index, statement) in sql_facts.statements.iter().enumerate() {
        let mut impact = ImpactFacts::unknown();
        if let Some(rows) = counted.get(index).copied().flatten() {
            impact.rows = Some(rows);
            impact.source = ImpactSource::Counted;
        } else if let Some(rows) = statement.literal_rows {
            impact.rows = Some(rows);
            impact.source = ImpactSource::Statement;
        } else if worth_estimating(statement) {
            match session.estimate_written_rows(&statement.text).await {
                Ok(rows) => {
                    impact.rows = Some(rows);
                    impact.source = ImpactSource::Planner;
                }
                Err(error) => {
                    tracing::debug!(error = %error, "the planner could not estimate a write");
                }
            }
        }
        for target in &statement.targets {
            let Some(table) = tables.iter().find(|table| &table.target == target) else {
                continue;
            };
            let Some(resolved) = &table.resolved else {
                continue;
            };
            for privilege in needed_privileges(statement.operation) {
                if !holds(table, privilege) {
                    impact
                        .missing_privileges
                        .push(format!("{privilege} on {resolved}"));
                }
            }
            if statement.operation == Operation::Delete {
                for child in &table.cascades_to {
                    if !impact.cascades_to.contains(child) {
                        impact.cascades_to.push(child.clone());
                    }
                }
            }
        }
        impacts.push(impact);
    }

    Ok((impacts, context))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, PoisonError};

    use super::*;
    use crate::adapters::local_store::test_support::temp_store;
    use crate::domain::connections::{ConnectionDraft, Environment};
    use crate::domain::policy::classify::ReasonCode;
    use crate::domain::policy::risk::{ApprovalLevel, RiskLevel};
    use crate::errors::ErrorCode;
    use crate::services::fakes::{FakeAdapter, sessions, target};

    async fn setup(
        draft: ConnectionDraft,
    ) -> (
        tempfile::TempDir,
        LocalStore,
        Arc<FakeAdapter>,
        Sessions<Arc<FakeAdapter>>,
        ConnectionId,
    ) {
        let (dir, store) = temp_store().await;
        let profile = store
            .connections()
            .insert(&draft.validated().expect("valid"))
            .await
            .expect("insert");
        let (adapter, sessions) = sessions();
        sessions.open(profile.id, target("db")).await.expect("open");
        (dir, store, adapter, sessions, profile.id)
    }

    fn writable() -> ConnectionDraft {
        let mut draft = ConnectionDraft::new("Shop", "db", "shop", "writer");
        draft.read_only = false;
        draft
    }

    #[tokio::test]
    async fn planner_estimates_decide_between_small_and_bulk_writes() {
        let (_dir, store, adapter, sessions, id) = setup(writable()).await;
        *adapter
            .catalog
            .write_estimate
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(3);
        let small = assess(
            &store,
            &sessions,
            id,
            "UPDATE orders SET status = 'x' WHERE id < 4",
        )
        .await
        .expect("assessment");
        assert_eq!(small.risk, RiskLevel::R2);
        assert_eq!(small.required_approval, ApprovalLevel::Apply);
        assert_eq!(small.statements[0].impact.and_then(|i| i.rows), Some(3));

        *adapter
            .catalog
            .write_estimate
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(5_000);
        let bulk = assess(
            &store,
            &sessions,
            id,
            "UPDATE orders SET status = 'x' WHERE id > 4",
        )
        .await
        .expect("assessment");
        assert_eq!(bulk.risk, RiskLevel::R3);

        // No estimate at all counts as unknown, which is not small.
        *adapter
            .catalog
            .write_estimate
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
        let unknown = assess(&store, &sessions, id, "DELETE FROM orders WHERE id = 1")
            .await
            .expect("assessment");
        assert_eq!(unknown.risk, RiskLevel::R3);
        assert_eq!(
            unknown.statements[0].reasons[0].code,
            ReasonCode::UnknownImpact
        );
    }

    #[tokio::test]
    async fn privileges_and_cascades_come_from_the_database() {
        let (_dir, store, adapter, sessions, id) = setup(writable()).await;
        *adapter
            .catalog
            .write_estimate
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(1);
        adapter
            .catalog
            .targets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(TargetTable {
                target: "orders".into(),
                resolved: Some("public.orders".into()),
                can_insert: true,
                can_update: false,
                can_delete: true,
                can_truncate: false,
                cascades_to: vec!["public.order_items".into()],
            });
        let update = assess(
            &store,
            &sessions,
            id,
            "UPDATE orders SET status = 'x' WHERE id = 1",
        )
        .await
        .expect("assessment");
        assert_eq!(update.required_approval, ApprovalLevel::Blocked);
        assert_eq!(
            update.blockers,
            vec!["The database role lacks UPDATE on public.orders.".to_owned()]
        );

        let delete = assess(&store, &sessions, id, "DELETE FROM orders WHERE id = 1")
            .await
            .expect("assessment");
        assert_eq!(delete.risk, RiskLevel::R3);
        assert!(
            delete.statements[0]
                .reasons
                .iter()
                .any(|reason| reason.code == ReasonCode::CascadingDelete
                    && reason.message.contains("public.order_items"))
        );
    }

    #[tokio::test]
    async fn the_profile_sets_environment_and_write_permission() {
        let mut production = writable();
        production.environment = Environment::Production;
        let (_dir, store, adapter, sessions, id) = setup(production).await;
        *adapter
            .catalog
            .write_estimate
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(1);
        let assessment = assess(
            &store,
            &sessions,
            id,
            "UPDATE orders SET status = 'x' WHERE id = 1",
        )
        .await
        .expect("assessment");
        assert_eq!(assessment.risk, RiskLevel::R3);
        assert_eq!(assessment.required_approval, ApprovalLevel::Elevated);

        // New connections are read-only: the same change is blocked there.
        let (_dir, store, adapter, sessions, id) =
            setup(ConnectionDraft::new("Shop", "db", "shop", "reader")).await;
        *adapter
            .catalog
            .write_estimate
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(1);
        let read_only = assess(
            &store,
            &sessions,
            id,
            "UPDATE orders SET status = 'x' WHERE id = 1",
        )
        .await
        .expect("assessment");
        assert_eq!(read_only.required_approval, ApprovalLevel::Blocked);
        assert_eq!(
            assess(&store, &sessions, id, "SELECT 1")
                .await
                .expect("assessment")
                .required_approval,
            ApprovalLevel::None
        );
    }

    #[tokio::test]
    async fn needs_an_open_connection_and_bounded_text() {
        let (_dir, store, _adapter, sessions, _id) = setup(writable()).await;
        let error = assess(&store, &sessions, ConnectionId::new(), "SELECT 1")
            .await
            .expect_err("unknown");
        assert_eq!(error.code(), ErrorCode::NotFound);
        let (_dir, store, _adapter, sessions, id) = setup(writable()).await;
        let long = "x".repeat(MAX_SQL_BYTES + 1);
        let error = assess(&store, &sessions, id, &long)
            .await
            .expect_err("too long");
        assert_eq!(error.code(), ErrorCode::InvalidInput);
    }
}
