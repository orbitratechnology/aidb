//! The tools, run against the open connection through the same services the UI uses: catalog
//! reads come from the cached catalog, queries go through the read-only query path with its
//! statement checks, timeouts and row limits.

use std::fmt::Write as _;

use super::{ToolOutput, ToolRunner};
use crate::adapters::database::DatabaseAdapter;
use crate::adapters::local_store::LocalStore;
use crate::domain::agent::context::{describe_object, object_line, render_rows};
use crate::domain::agent::tools::{ExplainInput, ListTablesInput, MAX_RESULT_ROWS, ToolInvocation};
use crate::domain::connections::ConnectionId;
use crate::domain::database::OperationId;
use crate::domain::query::{QueryRequest, QueryRun};
use crate::errors::AppError;
use crate::services::catalog::{self, CatalogCache};
use crate::services::sessions::Sessions;
use crate::services::{query, table_data};

/// Most tables `list_tables` returns; the model is told when there are more.
pub const MAX_LISTED_TABLES: usize = 200;

/// Runs tools on one connection's session.
pub struct SessionTools<'a, A: DatabaseAdapter> {
    pub store: &'a LocalStore,
    pub sessions: &'a Sessions<A>,
    pub cache: &'a CatalogCache,
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

    async fn explain(
        &self,
        input: &ExplainInput,
        operation: OperationId,
    ) -> Result<ToolOutput, String> {
        let options = if input.analyze {
            "ANALYZE, FORMAT TEXT"
        } else {
            "FORMAT TEXT"
        };
        let mut output = self
            .query(format!("EXPLAIN ({options})\n{}", input.sql), operation)
            .await?;
        // A plan's lines are rows of one text column; the header is noise for the model.
        output.text = output
            .text
            .split_once('\n')
            .map_or(output.text.clone(), |(_, rest)| rest.to_owned());
        output.outcome = if input.analyze {
            "Plan with timings".into()
        } else {
            "Plan".into()
        };
        Ok(output)
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
        }
    }

    async fn cancel(&self, operation: OperationId) {
        if let Err(error) = table_data::cancel(self.sessions, self.connection_id, operation).await {
            tracing::debug!(error = %error, "could not cancel the assistant's query");
        }
    }
}
