//! The tools the assistant may call. Each has a typed, versioned input that rejects unknown
//! fields, a JSON Schema the model sees, and a risk class decided here. There is no generic
//! "run SQL" tool: a query is accepted only if the SQL classifier finds exactly one read.
//!
//! Only R0 (introspection) and R1 (bounded read) tools exist while the assistant is read-only.

use std::fmt;
use std::sync::LazyLock;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::risk::RiskLevel;
use crate::domain::ai::{ToolCall, ToolInput, ToolSpec};
use crate::domain::catalog::ObjectName;
use crate::domain::sql::{self, StatementKind};

/// Version of the tool inputs and outputs; recorded with each plan.
pub const TOOLS_VERSION: u32 = 1;
/// Most rows of a query result the assistant sees.
pub const MAX_RESULT_ROWS: u32 = 50;
/// Longest SQL a tool accepts, in characters.
pub const MAX_SQL_CHARS: usize = 20_000;
/// Longest stated purpose of a query, in characters.
pub const MAX_PURPOSE_CHARS: usize = 300;
/// Longest search text, in characters.
pub const MAX_SEARCH_CHARS: usize = 100;
/// PostgreSQL truncates identifiers longer than 63 bytes.
const MAX_IDENTIFIER_BYTES: usize = 63;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum ToolName {
    ListSchemas,
    ListTables,
    DescribeTable,
    RunReadQuery,
    ExplainQuery,
}

impl ToolName {
    /// The name the model calls the tool by.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ListSchemas => "list_schemas",
            Self::ListTables => "list_tables",
            Self::DescribeTable => "describe_table",
            Self::RunReadQuery => "run_read_query",
            Self::ExplainQuery => "explain_query",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        TOOLS
            .iter()
            .map(|tool| tool.name)
            .find(|tool| tool.as_str() == name)
    }
}

impl fmt::Display for ToolName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A tool as the registry describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDefinition {
    pub name: ToolName,
    /// When and why to call it; models choose tools by this.
    pub description: &'static str,
    pub input_schema: Value,
    pub risk: RiskLevel,
    /// Whether the result contains values from the database (as opposed to structure only).
    /// Such results need the user's consent before they go to a hosted provider.
    pub returns_data: bool,
}

/// Every tool the read-only assistant has.
pub static TOOLS: LazyLock<Vec<ToolDefinition>> = LazyLock::new(|| {
    let identifier = |description: &str| json!({"type": "string", "minLength": 1, "maxLength": MAX_IDENTIFIER_BYTES, "description": description});
    vec![
        ToolDefinition {
            name: ToolName::ListSchemas,
            description: "Lists the schemas of the database with how many tables and views each \
                holds. Call this first when you do not know where the user's data lives.",
            input_schema: json!({
                "type": "object", "properties": {}, "additionalProperties": false,
            }),
            risk: RiskLevel::R0,
            returns_data: false,
        },
        ToolDefinition {
            name: ToolName::ListTables,
            description: "Lists tables and views, with their kind, estimated row count and \
                comment. Call this to find the tables a question is about; filter by schema or \
                by part of a name.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "schema": identifier("Only this schema; omit for all schemas."),
                    "search": {"type": "string", "maxLength": MAX_SEARCH_CHARS,
                        "description": "Only names containing this text, case-insensitive."},
                },
                "additionalProperties": false,
            }),
            risk: RiskLevel::R0,
            returns_data: false,
        },
        ToolDefinition {
            name: ToolName::DescribeTable,
            description: "Describes one table or view: columns with types and nullability, \
                primary and unique keys, foreign keys in both directions, indexes, and a view's \
                query. Call this before writing SQL against a table, so you use real column names.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "schema": identifier("The table's schema, such as public."),
                    "name": identifier("The table or view name."),
                },
                "required": ["schema", "name"],
                "additionalProperties": false,
            }),
            risk: RiskLevel::R0,
            returns_data: false,
        },
        ToolDefinition {
            name: ToolName::RunReadQuery,
            description: "Runs one read-only SQL statement (SELECT, WITH or VALUES) and returns \
                at most 50 rows. Call this to answer questions about the data. Select only the \
                columns you need, filter, aggregate where possible, and add a LIMIT. Statements \
                that change data or structure are refused.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sql": {"type": "string", "minLength": 1, "maxLength": MAX_SQL_CHARS,
                        "description": "Exactly one PostgreSQL read statement."},
                    "purpose": {"type": "string", "minLength": 1, "maxLength": MAX_PURPOSE_CHARS,
                        "description": "What the query finds out, in a short sentence for the user."},
                },
                "required": ["sql", "purpose"],
                "additionalProperties": false,
            }),
            risk: RiskLevel::R1,
            returns_data: true,
        },
        ToolDefinition {
            name: ToolName::ExplainQuery,
            description: "Shows PostgreSQL's plan for one read statement. With analyze, the \
                statement runs (read-only) and the plan shows real timings and row counts. Call \
                this when the user asks why a query is slow or how to speed it up.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "sql": {"type": "string", "minLength": 1, "maxLength": MAX_SQL_CHARS,
                        "description": "Exactly one PostgreSQL read statement, without EXPLAIN."},
                    "analyze": {"type": "boolean",
                        "description": "Run the statement to measure it. Defaults to false."},
                },
                "required": ["sql"],
                "additionalProperties": false,
            }),
            risk: RiskLevel::R1,
            returns_data: false,
        },
    ]
});

/// The registry as the provider adapters take it.
#[must_use]
pub fn specs() -> Vec<ToolSpec> {
    TOOLS
        .iter()
        .map(|tool| ToolSpec {
            name: tool.name.as_str().to_owned(),
            description: tool.description.to_owned(),
            input_schema: tool.input_schema.clone(),
        })
        .collect()
}

#[must_use]
pub fn definition(name: ToolName) -> &'static ToolDefinition {
    TOOLS
        .iter()
        .find(|tool| tool.name == name)
        .expect("every tool name has a definition")
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListTablesInput {
    #[serde(default)]
    pub schema: Option<String>,
    #[serde(default)]
    pub search: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DescribeTableInput {
    pub schema: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadQueryInput {
    pub sql: String,
    pub purpose: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplainInput {
    pub sql: String,
    #[serde(default)]
    pub analyze: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoInput {}

/// A checked tool call, ready for the planner to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolInvocation {
    ListSchemas,
    ListTables(ListTablesInput),
    DescribeTable(ObjectName),
    RunReadQuery(ReadQueryInput),
    ExplainQuery(ExplainInput),
}

impl ToolInvocation {
    #[must_use]
    pub const fn tool(&self) -> ToolName {
        match self {
            Self::ListSchemas => ToolName::ListSchemas,
            Self::ListTables(_) => ToolName::ListTables,
            Self::DescribeTable(_) => ToolName::DescribeTable,
            Self::RunReadQuery(_) => ToolName::RunReadQuery,
            Self::ExplainQuery(_) => ToolName::ExplainQuery,
        }
    }

    /// Decided by the tool and its checked arguments, never by the model.
    #[must_use]
    pub fn risk(&self) -> RiskLevel {
        definition(self.tool()).risk
    }

    /// One line for the user: what this step does.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::ListSchemas => "List the schemas".to_owned(),
            Self::ListTables(input) => match (&input.schema, &input.search) {
                (Some(schema), Some(search)) => {
                    format!("List tables in {schema} matching “{search}”")
                }
                (Some(schema), None) => format!("List tables in {schema}"),
                (None, Some(search)) => format!("List tables matching “{search}”"),
                (None, None) => "List tables".to_owned(),
            },
            Self::DescribeTable(object) => format!("Describe {}.{}", object.schema, object.name),
            Self::RunReadQuery(input) => format!("Run a read query: {}", input.purpose),
            Self::ExplainQuery(input) if input.analyze => {
                "Measure a query's plan (runs it read-only)".to_owned()
            }
            Self::ExplainQuery(_) => "Show a query's plan".to_owned(),
        }
    }

    /// The checked arguments, as recorded in the plan.
    #[must_use]
    pub fn input_json(&self) -> Value {
        match self {
            Self::ListSchemas => json!({}),
            Self::ListTables(input) => json!(input),
            Self::DescribeTable(object) => json!({"schema": object.schema, "name": object.name}),
            Self::RunReadQuery(input) => json!(input),
            Self::ExplainQuery(input) => json!(input),
        }
    }
}

/// Why a tool call was not accepted. The message goes back to the model as an error result so
/// it can correct itself; nothing runs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ToolCallError {
    #[error("There is no tool named `{0}`. Use only the tools you were given.")]
    UnknownTool(String),
    #[error("The arguments are not valid JSON. Call the tool again with a JSON object.")]
    MalformedArguments,
    #[error("The arguments do not match the tool's schema: {0}.")]
    InvalidArguments(String),
    #[error("{0}")]
    NotAllowed(String),
}

impl ToolCallError {
    /// A short code for the audit log; the message can quote the model's arguments.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnknownTool(_) => "unknown_tool",
            Self::MalformedArguments => "malformed_arguments",
            Self::InvalidArguments(_) => "invalid_arguments",
            Self::NotAllowed(_) => "not_allowed",
        }
    }
}

/// The tool's own name when `name` is one of the registry's, so audit records never hold text
/// the model made up.
#[must_use]
pub fn known_name(name: &str) -> Option<&'static str> {
    TOOLS
        .iter()
        .map(|tool| tool.name.as_str())
        .find(|known| *known == name)
}

fn arguments<T: DeserializeOwned>(input: &ToolInput) -> Result<T, ToolCallError> {
    let ToolInput::Json(value) = input else {
        return Err(ToolCallError::MalformedArguments);
    };
    serde_json::from_value(value.clone()).map_err(|error| {
        // serde's message names the field and expected type, never values beyond the field name.
        ToolCallError::InvalidArguments(error.to_string())
    })
}

fn identifier(value: &str, field: &str) -> Result<String, ToolCallError> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_IDENTIFIER_BYTES {
        return Err(ToolCallError::InvalidArguments(format!(
            "`{field}` must be 1 to {MAX_IDENTIFIER_BYTES} bytes"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(ToolCallError::InvalidArguments(format!(
            "`{field}` must not contain control characters"
        )));
    }
    Ok(value.to_owned())
}

fn bounded_text(value: &str, field: &str, max: usize) -> Result<String, ToolCallError> {
    let value = value.trim();
    if value.chars().count() > max {
        return Err(ToolCallError::InvalidArguments(format!(
            "`{field}` must be at most {max} characters"
        )));
    }
    Ok(value.to_owned())
}

/// Accepts the SQL only if it is exactly one statement the classifier reads as a read.
fn read_statement(sql: &str, allow_explain: bool) -> Result<String, ToolCallError> {
    let sql = sql.trim();
    if sql.is_empty() {
        return Err(ToolCallError::InvalidArguments("`sql` is empty".into()));
    }
    if sql.chars().count() > MAX_SQL_CHARS {
        return Err(ToolCallError::InvalidArguments(format!(
            "`sql` must be at most {MAX_SQL_CHARS} characters"
        )));
    }
    let analysis = sql::analyze(sql);
    if let Some(problem) = analysis.problem {
        return Err(ToolCallError::NotAllowed(format!(
            "The SQL could not be read: {}",
            problem.message
        )));
    }
    let [statement] = analysis.statements.as_slice() else {
        return Err(ToolCallError::NotAllowed(
            "Send exactly one statement per call.".into(),
        ));
    };
    if statement.kind != StatementKind::Read {
        return Err(ToolCallError::NotAllowed(format!(
            "Only read statements are allowed; this one {}. You cannot change data or structure.",
            match statement.kind {
                StatementKind::Write => "changes data",
                StatementKind::Schema => "changes structure",
                StatementKind::Session => "changes the session or transaction",
                StatementKind::Privileged => "needs privileges or acts outside the database",
                StatementKind::Read | StatementKind::Unknown => "was not understood",
            }
        )));
    }
    if let Some(reason) = &statement.reason {
        return Err(ToolCallError::NotAllowed(reason.clone()));
    }
    let upper = statement.text.trim_start().to_ascii_uppercase();
    if !allow_explain && upper.starts_with("EXPLAIN") {
        return Err(ToolCallError::NotAllowed(
            "Send the statement without EXPLAIN; explain_query adds it.".into(),
        ));
    }
    Ok(statement
        .text
        .trim()
        .trim_end_matches(';')
        .trim_end()
        .to_owned())
}

/// Checks a tool call from the model. Unknown tools, malformed or extra arguments and
/// statements that are not a single read are rejected before anything runs.
///
/// # Errors
///
/// A [`ToolCallError`] whose message tells the model what to fix.
pub fn parse(call: &ToolCall) -> Result<ToolInvocation, ToolCallError> {
    let name =
        ToolName::parse(&call.name).ok_or_else(|| ToolCallError::UnknownTool(call.name.clone()))?;
    match name {
        ToolName::ListSchemas => {
            arguments::<NoInput>(&call.input)?;
            Ok(ToolInvocation::ListSchemas)
        }
        ToolName::ListTables => {
            let input: ListTablesInput = arguments(&call.input)?;
            Ok(ToolInvocation::ListTables(ListTablesInput {
                schema: input
                    .schema
                    .as_deref()
                    .map(|schema| identifier(schema, "schema"))
                    .transpose()?,
                search: input
                    .search
                    .as_deref()
                    .map(|search| bounded_text(search, "search", MAX_SEARCH_CHARS))
                    .transpose()?
                    .filter(|search| !search.is_empty()),
            }))
        }
        ToolName::DescribeTable => {
            let input: DescribeTableInput = arguments(&call.input)?;
            Ok(ToolInvocation::DescribeTable(ObjectName {
                schema: identifier(&input.schema, "schema")?,
                name: identifier(&input.name, "name")?,
            }))
        }
        ToolName::RunReadQuery => {
            let input: ReadQueryInput = arguments(&call.input)?;
            let purpose = bounded_text(&input.purpose, "purpose", MAX_PURPOSE_CHARS)?;
            if purpose.is_empty() {
                return Err(ToolCallError::InvalidArguments("`purpose` is empty".into()));
            }
            Ok(ToolInvocation::RunReadQuery(ReadQueryInput {
                sql: read_statement(&input.sql, true)?,
                purpose,
            }))
        }
        ToolName::ExplainQuery => {
            let input: ExplainInput = arguments(&call.input)?;
            Ok(ToolInvocation::ExplainQuery(ExplainInput {
                sql: read_statement(&input.sql, false)?,
                analyze: input.analyze,
            }))
        }
    }
}

#[cfg(test)]
mod tests;
