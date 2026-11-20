//! The tools, run against the open connection through the same services the UI uses: catalog
//! reads come from the cached catalog, queries go through the read-only query path with its
//! statement checks, timeouts and row limits.

use std::fmt::Write as _;

use super::{ToolOutput, ToolRunner};
use crate::adapters::database::DatabaseAdapter;
use crate::adapters::local_store::LocalStore;
use crate::domain::advisor::{ExplainRequest, render};
use crate::domain::agent::context::{describe_object, object_line, render_rows};
use crate::domain::agent::tools::{
    ChangeProposal, ExplainInput, ListTablesInput, MAX_RESULT_ROWS, ToolInvocation,
};
use crate::domain::audit::Actor;
use crate::domain::connections::ConnectionId;
use crate::domain::data_change::PendingChange;
use crate::domain::database::OperationId;
use crate::domain::query::{QueryRequest, QueryRun};
use crate::errors::AppError;
use crate::services::approvals::Approvals;
use crate::services::catalog::{self, CatalogCache};
use crate::services::sessions::Sessions;
use crate::services::{advisor, data_changes, query, table_data};

/// Most tables `list_tables` returns; the model is told when there are more.
pub const MAX_LISTED_TABLES: usize = 200;

/// Runs tools on one connection's session.
pub struct SessionTools<'a, A: DatabaseAdapter> {
    pub store: &'a LocalStore,
    pub sessions: &'a Sessions<A>,
    pub cache: &'a CatalogCache,
    /// Where prepared changes wait for the user.
    pub approvals: &'a Approvals,
    pub connection_id: ConnectionId,
}

impl<A: DatabaseAdapter> std::fmt::Debug for SessionTools<'_, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionTools")
            .field("connection_id", &self.connection_id)
            .finish_non_exhaustive()
    }
}

fn error_text(error: &AppError) -> String {
    error.message().into_owned()
}

impl<A: DatabaseAdapter> SessionTools<'_, A> {
    async fn list_schemas(&self) -> Result<ToolOutput, String> {
        let snapshot = catalog::overview(
            self.store,
            self.sessions,
            self.cache,
            self.connection_id,
            false,
        )
        .await
        .map_err(|e| error_text(&e))?;
        let overview = &snapshot.overview;
        let mut text = format!("Schemas in {}:\n", overview.database);
        for schema in &overview.schemas {
            let count = overview
                .objects
                .iter()
                .filter(|object| object.schema == schema.name)
                .count();
            let _ = write!(text, "- {} ({count} tables and views", schema.name);
            if !schema.accessible {
                text.push_str(", no access");
            }
            text.push(')');
            if let Some(comment) = schema.comment.as_deref().filter(|c| !c.trim().is_empty()) {
                let _ = write!(
                    text,
                    " -- {}",
                    comment.split_whitespace().collect::<Vec<_>>().join(" ")
                );
            }
            text.push('\n');
        }
        Ok(ToolOutput {
            text,
            outcome: format!("{} schemas", overview.schemas.len()),
            incomplete: false,
            proposal: None,
        })
    }

    async fn list_tables(&self, input: &ListTablesInput) -> Result<ToolOutput, String> {
        let snapshot = catalog::overview(
            self.store,
            self.sessions,
            self.cache,
            self.connection_id,
            false,
        )
        .await
        .map_err(|e| error_text(&e))?;
        let overview = &snapshot.overview;
        let search = input.search.as_deref().map(str::to_lowercase);
        let matching: Vec<_> = overview
            .objects
            .iter()
            .filter(|object| {
                input
                    .schema
                    .as_ref()
                    .is_none_or(|schema| &object.schema == schema)
            })
            .filter(|object| {
                search.as_ref().is_none_or(|search| {
                    object.name.to_lowercase().contains(search)
                        || object
                            .comment
                            .as_deref()
                            .is_some_and(|comment| comment.to_lowercase().contains(search))
                })
            })
            .collect();
        let mut text = String::new();
        for object in matching.iter().take(MAX_LISTED_TABLES) {
            text.push_str(&object_line(object));
            text.push('\n');
        }
        if matching.is_empty() {
            text.push_str("No tables or views match.\n");
        }
        let cut = matching.len() > MAX_LISTED_TABLES;
        if cut {
            let _ = writeln!(
                text,
                "[{} more match and are not shown; narrow the search.]",
                matching.len() - MAX_LISTED_TABLES
            );
        }
        if overview.truncated {
            text.push_str(
                "[The database has more objects than aidb lists; some may be missing.]\n",
            );
        }
        Ok(ToolOutput {
            text,
            outcome: format!("{} tables and views", matching.len()),
            // A cut list is still exact about what it shows; the model is told to narrow it.
            incomplete: false,
            proposal: None,
        })
    }

    async fn describe(
        &self,
        object: &crate::domain::catalog::ObjectName,
    ) -> Result<ToolOutput, String> {
        let details = catalog::describe(
            self.store,
            self.sessions,
            self.cache,
            self.connection_id,
            object.clone(),
        )
        .await
        .map_err(|e| error_text(&e))?;
        Ok(ToolOutput {
            text: describe_object(&details),
            outcome: format!("{} columns", details.columns.len()),
            incomplete: false,
            proposal: None,
        })
    }

    async fn query(&self, sql: String, operation: OperationId) -> Result<ToolOutput, String> {
        let request = QueryRequest {
            sql,
            offset: 0,
            operation_id: Some(operation),
        };
        let run = query::run_for_assistant(
            self.store,
            self.sessions,
            self.connection_id,
            request,
            MAX_RESULT_ROWS,
        )
        .await
        .map_err(|e| error_text(&e))?;
        match run {
            QueryRun::Completed(result) => {
                let incomplete = result.has_more || result.byte_limit_reached;
                let mut text = render_rows(&result.columns, &result.rows, incomplete);
                if !result.truncated_cells.is_empty() {
                    let _ = write!(
                        text,
                        "\n[{} long values were cut at {} characters.]",
                        result.truncated_cells.len(),
                        result.cell_char_limit
                    );
                }
                let rows = result.rows.len();
                Ok(ToolOutput {
                    text,
                    outcome: if incomplete {
                        format!("First {rows} rows")
                    } else if rows == 1 {
                        "1 row".to_owned()
                    } else {
                        format!("{rows} rows")
                    },
                    incomplete,
                    proposal: None,
                })
            }
            QueryRun::Refused(refusal) => {
                Err(format!("aidb did not run this: {}", refusal.message))
            }
            QueryRun::Failed(failure) => Err(match failure.sqlstate {
                Some(state) => format!(
                    "The database returned an error ({state}): {}",
                    failure.message
                ),
                None => format!("The query did not finish: {}", failure.message),
            }),
        }
    }

    /// The plan through the query advisor: a tree with its figures and aidb's findings, drawn
    /// from the plan's own numbers, so the model explains from the same evidence the user sees.
    async fn explain(
        &self,
        input: &ExplainInput,
        operation: OperationId,
    ) -> Result<ToolOutput, String> {
        let request = ExplainRequest {
            sql: input.sql.clone(),
            analyze: input.analyze,
            operation_id: Some(operation),
        };
        let plan = advisor::explain(self.sessions, self.connection_id, &request)
            .await
            .map_err(|e| error_text(&e))?;
        let findings = plan.suggestions.len();
        Ok(ToolOutput {
            text: render(&plan),
            outcome: format!(
                "{}{}",
                if input.analyze {
                    "Plan with timings"
                } else {
                    "Plan"
                },
                match findings {
                    0 => String::new(),
                    1 => ", 1 finding".to_owned(),
                    n => format!(", {n} findings"),
                }
            ),
            incomplete: plan.truncated,
            proposal: None,
        })
    }
}

impl<A: DatabaseAdapter> SessionTools<'_, A> {
    /// Plans the changes through the same engine as the grid and holds them for the user. The
    /// model is told what was prepared and that nothing was written; values, counts and the
    /// rows they match stay in the app.
    async fn propose(&self, proposal: &ChangeProposal) -> Result<ToolOutput, String> {
        let preview = data_changes::preview(
            self.store,
            self.sessions,
            self.approvals,
            self.connection_id,
            proposal.changes.clone(),
            Actor::Agent,
        )
        .await
        .map_err(|e| error_text(&e))?;
        let what: Vec<String> = proposal
            .changes
            .iter()
            .map(|change| {
                let table = change.table();
                format!(
                    "{} of {}.{}",
                    change.kind().as_str(),
                    table.schema,
                    table.name
                )
            })
            .collect();
        let text = format!(
            "Prepared {} for the user's review: {}. Nothing has been written. The user reviews \
             it in the app, then applies or declines it; you cannot apply it. Tell the user it \
             is waiting for their review, and do not say that it was applied.",
            if what.len() == 1 {
                "1 change".to_owned()
            } else {
                format!("{} changes", what.len())
            },
            what.join(", ")
        );
        let outcome = format!(
            "Prepared for your review: {}, risk {}",
            if preview.rows == 1 {
                "1 row".to_owned()
            } else {
                format!("{} rows", preview.rows)
            },
            preview.risk.as_str()
        );
        Ok(ToolOutput {
            text,
            outcome,
            incomplete: false,
            proposal: Some(PendingChange {
                connection_id: self.connection_id,
                actor: Actor::Agent,
                preview,
                changes: proposal.changes.clone(),
            }),
        })
    }
}

impl<A: DatabaseAdapter> ToolRunner for SessionTools<'_, A> {
    async fn run(
        &self,
        invocation: &ToolInvocation,
        operation: OperationId,
    ) -> Result<ToolOutput, String> {
        match invocation {
            ToolInvocation::ListSchemas => self.list_schemas().await,
            ToolInvocation::ListTables(input) => self.list_tables(input).await,
            ToolInvocation::DescribeTable(object) => self.describe(object).await,
            ToolInvocation::RunReadQuery(input) => self.query(input.sql.clone(), operation).await,
            ToolInvocation::ExplainQuery(input) => self.explain(input, operation).await,
            ToolInvocation::PreviewDataChange(proposal) => self.propose(proposal).await,
        }
    }

    async fn cancel(&self, operation: OperationId) {
        if let Err(error) = table_data::cancel(self.sessions, self.connection_id, operation).await {
            tracing::debug!(error = %error, "could not cancel the assistant's query");
        }
    }
}
