//! The query advisor: a PostgreSQL plan (`EXPLAIN (FORMAT JSON, VERBOSE)`, optionally with
//! `ANALYZE, BUFFERS`) read into a tree, and suggestions drawn only from what the plan and the
//! tables' statistics show. Each suggestion carries its evidence: the numbers it rests on.
//! Nothing here runs SQL; suggested statements are text for the user.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::tokenizer::{Token, Tokenizer};

use crate::domain::schema_diff::normalize::{qualified, quote_ident};

/// Nodes kept from a plan; larger plans are cut.
pub const MAX_PLAN_NODES: usize = 2_000;

/// A scan reading fewer rows than this is not worth an index.
const MIN_ROWS_READ: f64 = 10_000.0;
/// A filter keeping at most this share of the rows read is selective enough for an index.
const MAX_KEPT_SHARE: f64 = 0.05;
/// Estimates this far off (either way) suggest stale statistics.
const MISESTIMATE_FACTOR: f64 = 10.0;
/// ...on nodes with at least this many rows, estimated or real.
const MIN_MISESTIMATE_ROWS: f64 = 1_000.0;
/// Heap fetches an index-only scan may make before vacuuming is worth suggesting.
const MIN_HEAP_FETCHES: f64 = 1_000.0;

/// What to explain: one read statement, planned or also run (`analyze`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExplainRequest {
    pub sql: String,
    /// Run the statement (read-only, rolled back) to measure it.
    #[serde(default)]
    pub analyze: bool,
    #[serde(default)]
    #[cfg_attr(test, ts(optional, type = "string"))]
    pub operation_id: Option<crate::domain::database::OperationId>,
}

/// An index on a table the plan reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexFacts {
    pub name: String,
    /// Key columns in order; `None` for an expression.
    pub columns: Vec<Option<String>>,
    pub partial: bool,
}

/// What the database knows about a table the plan reads.
#[derive(Debug, Clone, PartialEq)]
pub struct TableFacts {
    pub schema: String,
    pub name: String,
    /// The planner's row estimate (`reltuples`), when the table was ever analyzed.
    pub rows: Option<f64>,
    /// When statistics were last gathered, if ever.
    pub last_analyzed: Option<String>,
    pub columns: Vec<String>,
    pub indexes: Vec<IndexFacts>,
}

/// One node of the plan, in depth-first order.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct PlanNode {
    pub id: u32,
    pub parent: Option<u32>,
    pub depth: u32,
    pub node_type: String,
    /// "Seq Scan on public.orders o", "Hash Join (inner)".
    pub label: String,
    /// The node's conditions and figures, as lines: "Filter: (o.total > 100)".
    pub details: Vec<String>,
    pub total_cost: f64,
    /// Rows the planner expects, for all loops.
    pub plan_rows: f64,
    /// Rows it produced, for all loops (with `analyze`).
    pub actual_rows: Option<f64>,
    pub loops: Option<f64>,
    /// Time in this node and below, in milliseconds, for all loops.
    pub actual_ms: Option<f64>,
    /// Time in this node alone.
    pub self_ms: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum SuggestionKind {
    MissingIndex,
    StaleStatistics,
    SortSpill,
    HashSpill,
    LossyBitmap,
    HeapFetches,
    FewerWorkers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
}

/// Something the plan suggests, with the numbers it rests on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub kind: SuggestionKind,
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    pub evidence: Vec<String>,
    /// The plan node it is about.
    pub node: Option<u32>,
    /// A statement that would act on it, for the user to review and run elsewhere.
    pub sql: Option<String>,
}

/// A plan and what it suggests.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct QueryPlan {
    /// The statement ran to measure it.
    pub analyzed: bool,
    pub planning_ms: Option<f64>,
    pub execution_ms: Option<f64>,
    pub nodes: Vec<PlanNode>,
    /// More nodes than [`MAX_PLAN_NODES`]; the rest are left out.
    pub truncated: bool,
    pub suggestions: Vec<Suggestion>,
}

/// A plan could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("aidb could not read the plan PostgreSQL returned.")]
pub struct PlanError;

/// The figures the rules need, kept beside each node while reading.
#[derive(Debug, Clone, Default)]
struct Raw {
    node_type: String,
    schema: Option<String>,
    relation: Option<String>,
    alias: Option<String>,
    filter: Option<String>,
    rows_removed: Option<f64>,
    per_loop_rows: Option<f64>,
    per_loop_plan_rows: f64,
    loops: Option<f64>,
    sort_space_type: Option<String>,
    sort_space_kb: Option<f64>,
    sort_method: Option<String>,
    hash_batches: Option<f64>,
    original_batches: Option<f64>,
    peak_memory_kb: Option<f64>,
    lossy_blocks: Option<f64>,
    heap_fetches: Option<f64>,
    workers_planned: Option<f64>,
    workers_launched: Option<f64>,
    parent_relationship: Option<String>,
    parent_is_nested_loop: bool,
}

fn text(node: &Value, key: &str) -> Option<String> {
    node.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn number(node: &Value, key: &str) -> Option<f64> {
    node.get(key).and_then(Value::as_f64)
}

fn count(value: f64) -> String {
    let rounded = value.round();
    if rounded.abs() >= 1e15 {
        return format!("{rounded:e}");
    }
    #[allow(clippy::cast_possible_truncation, reason = "checked to be below 1e15")]
    let whole = rounded as i64;
    let digits = whole.unsigned_abs().to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    if whole < 0 { format!("-{out}") } else { out }
}

fn ms(value: f64) -> String {
    if value >= 100.0 {
        format!("{value:.0} ms")
    } else {
        format!("{value:.2} ms")
    }
}

/// Reads `EXPLAIN (FORMAT JSON ...)` output into nodes. Suggestions come from [`advise`].
///
/// # Errors
///
/// [`PlanError`] when the output is not a plan.
pub fn read_plan(output: &Value) -> Result<QueryPlan, PlanError> {
    let (plan, nodes, _) = read(output)?;
    Ok(QueryPlan { nodes, ..plan })
}

#[allow(
    clippy::type_complexity,
    reason = "private: the plan, its nodes and their figures"
)]
fn read(output: &Value) -> Result<(QueryPlan, Vec<PlanNode>, Vec<Raw>), PlanError> {
    let top = output
        .as_array()
        .and_then(|items| items.first())
        .or_else(|| output.is_object().then_some(output))
        .ok_or(PlanError)?;
    let root = top.get("Plan").ok_or(PlanError)?;
    let mut nodes = Vec::new();
    let mut raws = Vec::new();
    let mut truncated = false;
    let mut stack: Vec<(&Value, Option<u32>, u32, bool)> = vec![(root, None, 0, false)];
    while let Some((node, parent, depth, parent_is_nested_loop)) = stack.pop() {
        if nodes.len() >= MAX_PLAN_NODES {
            truncated = true;
            break;
        }
        let id = u32::try_from(nodes.len()).map_err(|_| PlanError)?;
        let (plan_node, raw) = node_of(node, id, parent, depth, parent_is_nested_loop)?;
        let is_nested_loop = raw.node_type == "Nested Loop";
        nodes.push(plan_node);
        raws.push(raw);
        if let Some(children) = node.get("Plans").and_then(Value::as_array) {
            // Reversed, so the first child comes out of the stack first.
            for child in children.iter().rev() {
                stack.push((child, Some(id), depth + 1, is_nested_loop));
            }
        }
    }
    // Time in each node alone: its time less its children's.
    let mut child_ms = vec![0.0_f64; nodes.len()];
    for node in &nodes {
        if let (Some(parent), Some(time)) = (node.parent, node.actual_ms) {
            child_ms[parent as usize] += time;
        }
    }
    for (node, below) in nodes.iter_mut().zip(child_ms) {
        node.self_ms = node.actual_ms.map(|time| (time - below).max(0.0));
    }
    let analyzed = nodes.first().is_some_and(|node| node.actual_rows.is_some());
    Ok((
        QueryPlan {
            analyzed,
            planning_ms: number(top, "Planning Time"),
            execution_ms: number(top, "Execution Time"),
            nodes: Vec::new(),
            truncated,
            suggestions: Vec::new(),
        },
        nodes,
        raws,
    ))
}

fn node_of(
    node: &Value,
    id: u32,
    parent: Option<u32>,
    depth: u32,
    parent_is_nested_loop: bool,
) -> Result<(PlanNode, Raw), PlanError> {
    let node_type = text(node, "Node Type").ok_or(PlanError)?;
    let schema = text(node, "Schema");
    let relation = text(node, "Relation Name");
    let alias = text(node, "Alias");
    let loops = number(node, "Actual Loops");
    let per_loop_rows = number(node, "Actual Rows");
    let per_loop_plan_rows = number(node, "Plan Rows").unwrap_or(0.0);
    let loop_count = loops.unwrap_or(1.0).max(1.0);

    // The text format's name: JSON says "Seq Scan" with "Parallel Aware".
    let mut label = if node.get("Parallel Aware").and_then(Value::as_bool) == Some(true)
        && !node_type.starts_with("Parallel")
    {
        format!("Parallel {node_type}")
    } else {
        node_type.clone()
    };
    if let Some(strategy) = text(node, "Strategy").filter(|s| s != "Plain") {
        label = format!("{label} ({})", strategy.to_lowercase());
    }
    if let Some(join) = text(node, "Join Type").filter(|j| j != "Inner") {
        label = format!("{label} ({} join)", join.to_lowercase());
    }
    if let Some(index) = text(node, "Index Name") {
        label = format!("{label} using {index}");
    }
    if let Some(name) = &relation {
        let table = match &schema {
            Some(schema) => qualified(schema, name),
            None => quote_ident(name),
        };
        label = format!("{label} on {table}");
        if let Some(alias) = alias.as_ref().filter(|alias| *alias != name) {
            label = format!("{label} {}", quote_ident(alias));
        }
    } else if let Some(cte) = text(node, "CTE Name") {
        label = format!("{label} on {cte}");
    } else if let Some(function) = text(node, "Function Name") {
        label = format!("{label} on {function}()");
    }
    if let Some(sub) = text(node, "Subplan Name") {
        label = format!("{label} ({sub})");
    }

    let mut details = Vec::new();
    for (key, title) in [
        ("Index Cond", "Index condition"),
        ("Recheck Cond", "Recheck condition"),
        ("Hash Cond", "Hash condition"),
        ("Merge Cond", "Merge condition"),
        ("Join Filter", "Join filter"),
        ("Filter", "Filter"),
    ] {
        if let Some(condition) = text(node, key) {
            details.push(format!("{title}: {condition}"));
        }
    }
    for (key, title) in [
        ("Rows Removed by Filter", "Rows removed by filter"),
        ("Rows Removed by Join Filter", "Rows removed by join filter"),
        ("Rows Removed by Index Recheck", "Rows removed by recheck"),
        ("Heap Fetches", "Heap fetches"),
        ("Lossy Heap Blocks", "Lossy heap blocks"),
    ] {
        if let Some(value) = number(node, key).filter(|value| *value > 0.0) {
            details.push(format!("{title}: {}", count(value * loop_count)));
        }
    }
    if let Some(keys) = node.get("Sort Key").and_then(Value::as_array) {
        let keys: Vec<&str> = keys.iter().filter_map(Value::as_str).collect();
        details.push(format!("Sort key: {}", keys.join(", ")));
    }
    if let Some(method) = text(node, "Sort Method") {
        let space = match (
            number(node, "Sort Space Used"),
            text(node, "Sort Space Type"),
        ) {
            (Some(used), Some(kind)) => format!(", {kind}: {} kB", count(used)),
            _ => String::new(),
        };
        details.push(format!("Sort method: {method}{space}"));
    }
    if let Some(batches) = number(node, "Hash Batches") {
        let memory = number(node, "Peak Memory Usage")
            .map(|kb| format!(", memory: {} kB", count(kb)))
            .unwrap_or_default();
        details.push(format!("Hash batches: {}{memory}", count(batches)));
    }
    if let (Some(planned), Some(launched)) = (
        number(node, "Workers Planned"),
        number(node, "Workers Launched"),
    ) {
        details.push(format!(
            "Workers: {} planned, {} launched",
            count(planned),
            count(launched)
        ));
    }
    let hit = number(node, "Shared Hit Blocks");
    let read = number(node, "Shared Read Blocks");
    if hit.is_some() || read.is_some() {
        details.push(format!(
            "Buffers: {} hit, {} read",
            count(hit.unwrap_or(0.0)),
            count(read.unwrap_or(0.0))
        ));
    }

    let raw = Raw {
        node_type: node_type.clone(),
        schema,
        relation: relation.clone(),
        alias,
        filter: text(node, "Filter"),
        rows_removed: number(node, "Rows Removed by Filter"),
        per_loop_rows,
        per_loop_plan_rows,
        loops,
        sort_space_type: text(node, "Sort Space Type"),
        sort_space_kb: number(node, "Sort Space Used"),
        sort_method: text(node, "Sort Method"),
        hash_batches: number(node, "Hash Batches"),
        original_batches: number(node, "Original Hash Batches"),
        peak_memory_kb: number(node, "Peak Memory Usage"),
        lossy_blocks: number(node, "Lossy Heap Blocks"),
        heap_fetches: number(node, "Heap Fetches"),
        workers_planned: number(node, "Workers Planned"),
        workers_launched: number(node, "Workers Launched"),
        parent_relationship: text(node, "Parent Relationship"),
        parent_is_nested_loop,
    };
    Ok((
        PlanNode {
            id,
            parent,
            depth,
            node_type,
            label,
            details,
            total_cost: number(node, "Total Cost").unwrap_or(0.0),
            plan_rows: per_loop_plan_rows * loop_count,
            actual_rows: per_loop_rows.map(|rows| rows * loop_count),
            loops,
            actual_ms: number(node, "Actual Total Time").map(|time| time * loop_count),
            self_ms: None,
        },
        raw,
    ))
}

/// Reads a plan and what it suggests, given the tables it reads.
///
/// # Errors
///
/// [`PlanError`] when the output is not a plan.
pub fn advise(output: &Value, tables: &[TableFacts]) -> Result<QueryPlan, PlanError> {
    let (mut plan, nodes, raws) = read(output)?;
    let mut suggestions = Vec::new();
    for (node, raw) in nodes.iter().zip(&raws) {
        let table = table_of(raw, tables);
        missing_index(node, raw, table, &nodes, &mut suggestions);
        stale_statistics(node, raw, table, &mut suggestions);
        spills(node, raw, &mut suggestions);
        heap_fetches(node, raw, table, &mut suggestions);
    }
    // One suggestion per index and per table to analyze: the first (outermost) one.
    let mut seen: Vec<(SuggestionKind, Option<String>)> = Vec::new();
    suggestions.retain(|suggestion: &Suggestion| {
        let key = (suggestion.kind, suggestion.sql.clone());
        if suggestion.sql.is_some() && seen.contains(&key) {
            return false;
        }
        seen.push(key);
        true
    });
    suggestions.sort_by_key(|suggestion| suggestion.severity != Severity::Warning);
    plan.nodes = nodes;
    plan.suggestions = suggestions;
    Ok(plan)
}

fn table_of<'a>(raw: &Raw, tables: &'a [TableFacts]) -> Option<&'a TableFacts> {
    let name = raw.relation.as_ref()?;
    tables.iter().find(|table| {
        &table.name == name
            && raw
                .schema
                .as_ref()
                .is_none_or(|schema| &table.schema == schema)
    })
}

fn table_name(table: &TableFacts) -> String {
    qualified(&table.schema, &table.name)
}

/// Columns of the scanned table a filter compares: those compared with `=`, then those
/// compared with `<`, `>`, `<=` or `>=`. A column inside a function call or cast is not
/// counted: a plain index would not serve it.
fn filter_columns(
    filter: &str,
    alias: Option<&str>,
    table: &TableFacts,
) -> (Vec<String>, Vec<String>) {
    let tokens: Vec<Token> = Tokenizer::new(&PostgreSqlDialect {}, filter)
        .tokenize()
        .unwrap_or_default()
        .into_iter()
        .filter(|token| !matches!(token, Token::Whitespace(_)))
        .collect();
    let column_at = |index: usize| -> Option<(String, usize)> {
        // `alias.column` or a bare `column`; returns the column and the index after it.
        match (
            tokens.get(index),
            tokens.get(index + 1),
            tokens.get(index + 2),
        ) {
            (Some(Token::Word(qualifier)), Some(Token::Period), Some(Token::Word(column)))
                if alias.is_some_and(|alias| qualifier.value == alias)
                    || qualifier.value == table.name =>
            {
                Some((column.value.clone(), index + 3))
            }
            (Some(Token::Word(column)), next, _) if next != Some(&Token::Period) => {
                let before = index.checked_sub(1).and_then(|i| tokens.get(i));
                (before != Some(&Token::Period)).then(|| (column.value.clone(), index + 1))
            }
            _ => None,
        }
    };
    let mut equal = Vec::new();
    let mut range = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let Some((column, after)) = column_at(index) else {
            index += 1;
            continue;
        };
        if table.columns.contains(&column) {
            // Not wrapped in a call: the token before is not `name(` (`AND (` is grouping).
            let wrapped = index > 1
                && tokens.get(index - 1) == Some(&Token::LParen)
                && matches!(
                    tokens.get(index - 2),
                    Some(Token::Word(word)) if !["and", "or", "not"]
                        .contains(&word.value.to_lowercase().as_str())
                );
            let list = match tokens.get(after) {
                Some(Token::Eq) if !wrapped => Some(&mut equal),
                Some(Token::Lt | Token::Gt | Token::LtEq | Token::GtEq) if !wrapped => {
                    Some(&mut range)
                }
                _ => None,
            };
            if let Some(list) = list
                && !list.contains(&column)
            {
                list.push(column);
            }
        }
        index = after;
    }
    range.retain(|column| !equal.contains(column));
    (equal, range)
}

fn missing_index(
    node: &PlanNode,
    raw: &Raw,
    table: Option<&TableFacts>,
    nodes: &[PlanNode],
    suggestions: &mut Vec<Suggestion>,
) {
    if raw.node_type != "Seq Scan" && raw.node_type != "Parallel Seq Scan" {
        return;
    }
    let (Some(table), Some(filter)) = (table, &raw.filter) else {
        return;
    };
    let loops = raw
        .loops
        .unwrap_or_else(|| {
            // Planned only: an inner scan of a nested loop runs once per outer row.
            if raw.parent_is_nested_loop && raw.parent_relationship.as_deref() == Some("Inner") {
                node.parent
                    .and_then(|parent| {
                        nodes
                            .iter()
                            .find(|n| n.parent == Some(parent) && n.id != node.id)
                            .map(|outer| outer.plan_rows)
                    })
                    .unwrap_or(1.0)
            } else {
                1.0
            }
        })
        .max(1.0);
    // Rows read and kept, from the run when there is one, else from the estimates.
    let (read, kept, measured) = match (raw.per_loop_rows, raw.rows_removed) {
        (Some(rows), Some(removed)) => ((rows + removed) * loops, rows * loops, true),
        (Some(rows), None) => (rows * loops, rows * loops, true),
        _ => match table.rows {
            Some(rows) => (rows * loops, raw.per_loop_plan_rows * loops, false),
            None => return,
        },
    };
    if read < MIN_ROWS_READ || kept > read * MAX_KEPT_SHARE {
        return;
    }
    let (equal, range) = filter_columns(filter, raw.alias.as_deref(), table);
    let mut columns: Vec<String> = equal.iter().take(3).cloned().collect();
    if let Some(column) = range.first() {
        columns.push(column.clone());
    }
    let Some(leading) = columns.first() else {
        return;
    };
    let indexed = table.indexes.iter().any(|index| {
        !index.partial && index.columns.first().cloned().flatten().as_ref() == Some(leading)
    });
    if indexed {
        return;
    }
    let share = if read > 0.0 { kept / read * 100.0 } else { 0.0 };
    let mut evidence = vec![
        format!(
            "{} {} {} rows and {} {} ({share:.2}%){}.",
            node.label,
            if measured {
                "read"
            } else {
                "is expected to read"
            },
            count(read),
            if measured { "kept" } else { "keep" },
            count(kept),
            if loops > 1.0 {
                format!(", over {} loops", count(loops))
            } else {
                String::new()
            }
        ),
        format!("Filter: {filter}"),
        format!(
            "No index on {} starts with {}.",
            table_name(table),
            quote_ident(leading)
        ),
    ];
    if !measured {
        evidence.push("Estimated from statistics; run it to measure.".to_owned());
    }
    let list = columns
        .iter()
        .map(|column| quote_ident(column))
        .collect::<Vec<_>>()
        .join(", ");
    suggestions.push(Suggestion {
        kind: SuggestionKind::MissingIndex,
        severity: Severity::Warning,
        title: format!("An index on {} ({list}) may help", table_name(table)),
        detail: "The query reads the whole table to keep a few rows. An index on the filtered \
                 columns lets PostgreSQL find them directly. Building one locks writes to the \
                 table unless it is built concurrently, and every index slows writes a little."
            .to_owned(),
        evidence,
        node: Some(node.id),
        sql: Some(format!(
            "CREATE INDEX CONCURRENTLY ON {} ({list});",
            table_name(table)
        )),
    });
}

fn stale_statistics(
    node: &PlanNode,
    raw: &Raw,
    table: Option<&TableFacts>,
    suggestions: &mut Vec<Suggestion>,
) {
    let (Some(actual), Some(table)) = (raw.per_loop_rows, table) else {
        return;
    };
    let estimated = raw.per_loop_plan_rows;
    let larger = actual.max(estimated);
    let smaller = actual.min(estimated).max(1.0);
    if larger < MIN_MISESTIMATE_ROWS || larger / smaller < MISESTIMATE_FACTOR {
        return;
    }
    let factor = larger / smaller;
    let mut evidence = vec![format!(
        "{}: estimated {} rows per loop, got {} ({factor:.0}× {}).",
        node.label,
        count(estimated),
        count(actual),
        if actual > estimated { "more" } else { "fewer" }
    )];
    evidence.push(match &table.last_analyzed {
        Some(at) => format!("{} was last analyzed at {at}.", table_name(table)),
        None => format!("{} has never been analyzed.", table_name(table)),
    });
    suggestions.push(Suggestion {
        kind: SuggestionKind::StaleStatistics,
        severity: Severity::Warning,
        title: format!(
            "The planner's estimate for {} is far off",
            table_name(table)
        ),
        detail: "Plans are chosen from estimated row counts. When they are this far off, fresh \
                 statistics often lead to a better plan; if they do not, the filter's columns \
                 may be correlated (extended statistics can help)."
            .to_owned(),
        evidence,
        node: Some(node.id),
        sql: Some(format!("ANALYZE {};", table_name(table))),
    });
}

fn spills(node: &PlanNode, raw: &Raw, suggestions: &mut Vec<Suggestion>) {
    if raw.sort_space_type.as_deref() == Some("Disk") {
        let used = raw.sort_space_kb.unwrap_or(0.0);
        suggestions.push(Suggestion {
            kind: SuggestionKind::SortSpill,
            severity: Severity::Warning,
            title: "A sort spilled to disk".to_owned(),
            detail: format!(
                "The rows did not fit in work_mem, so they were sorted in temporary files. More \
                 work_mem for this query (about {} MB), fewer rows to sort, or an index that \
                 returns them in order avoids it.",
                count((used / 1024.0 * 2.0).ceil().max(1.0))
            ),
            evidence: vec![format!(
                "{}: {}, {} kB on disk.",
                node.label,
                raw.sort_method.as_deref().unwrap_or("external sort"),
                count(used)
            )],
            node: Some(node.id),
            sql: None,
        });
    }
    if let Some(batches) = raw.hash_batches.filter(|batches| *batches > 1.0) {
        let original = raw.original_batches.unwrap_or(batches);
        suggestions.push(Suggestion {
            kind: SuggestionKind::HashSpill,
            severity: Severity::Warning,
            title: "A hash table did not fit in memory".to_owned(),
            detail: "It was split into batches written to temporary files. More work_mem for \
                     this query, or fewer rows on the hashed side, keeps it in memory."
                .to_owned(),
            evidence: vec![format!(
                "{}: {} batches{}{}.",
                node.label,
                count(batches),
                if original < batches {
                    format!(" (planned {})", count(original))
                } else {
                    String::new()
                },
                raw.peak_memory_kb
                    .map(|kb| format!(", peak memory {} kB", count(kb)))
                    .unwrap_or_default()
            )],
            node: Some(node.id),
            sql: None,
        });
    }
    if let Some(lossy) = raw.lossy_blocks.filter(|blocks| *blocks > 0.0) {
        suggestions.push(Suggestion {
            kind: SuggestionKind::LossyBitmap,
            severity: Severity::Info,
            title: "A bitmap became lossy".to_owned(),
            detail: "The bitmap of matching rows outgrew work_mem, so whole pages were checked \
                     again row by row. More work_mem for this query avoids it."
                .to_owned(),
            evidence: vec![format!(
                "{}: {} lossy heap blocks.",
                node.label,
                count(lossy * raw.loops.unwrap_or(1.0))
            )],
            node: Some(node.id),
            sql: None,
        });
    }
    if let (Some(planned), Some(launched)) = (raw.workers_planned, raw.workers_launched)
        && launched < planned
    {
        suggestions.push(Suggestion {
            kind: SuggestionKind::FewerWorkers,
            severity: Severity::Info,
            title: "Fewer parallel workers ran than planned".to_owned(),
            detail: "The server had no free background workers when the query started \
                     (max_parallel_workers or max_worker_processes), so it ran with fewer."
                .to_owned(),
            evidence: vec![format!(
                "{}: {} workers planned, {} launched.",
                node.label,
                count(planned),
                count(launched)
            )],
            node: Some(node.id),
            sql: None,
        });
    }
}

fn heap_fetches(
    node: &PlanNode,
    raw: &Raw,
    table: Option<&TableFacts>,
    suggestions: &mut Vec<Suggestion>,
) {
    let (Some(fetches), Some(table)) = (raw.heap_fetches, table) else {
        return;
    };
    let fetches = fetches * raw.loops.unwrap_or(1.0);
    let rows = node.actual_rows.unwrap_or(0.0);
    if fetches < MIN_HEAP_FETCHES || fetches < rows * 0.2 {
        return;
    }
    suggestions.push(Suggestion {
        kind: SuggestionKind::HeapFetches,
        severity: Severity::Info,
        title: format!("{} needs vacuuming for index-only scans", table_name(table)),
        detail: "An index-only scan had to visit the table for rows not yet marked visible to \
                 everyone. Vacuuming updates that map, so the scan can skip the table."
            .to_owned(),
        evidence: vec![format!(
            "{}: {} heap fetches for {} rows.",
            node.label,
            count(fetches),
            count(rows)
        )],
        node: Some(node.id),
        sql: Some(format!("VACUUM {};", table_name(table))),
    });
}

/// The plan as indented text with its suggestions, for the assistant.
#[must_use]
pub fn render(plan: &QueryPlan) -> String {
    let mut out = String::new();
    for node in &plan.nodes {
        let indent = "  ".repeat(node.depth as usize);
        out.push_str(&format!(
            "{indent}-> {} (cost {:.2}, rows {}",
            node.label,
            node.total_cost,
            count(node.plan_rows)
        ));
        if let Some(actual) = node.actual_rows {
            out.push_str(&format!("; actual rows {}", count(actual)));
        }
        if let Some(loops) = node.loops.filter(|loops| *loops > 1.0) {
            out.push_str(&format!(", loops {}", count(loops)));
        }
        if let Some(time) = node.actual_ms {
            out.push_str(&format!(", {}", ms(time)));
        }
        out.push_str(")\n");
        for detail in &node.details {
            out.push_str(&format!("{indent}     {detail}\n"));
        }
    }
    if plan.truncated {
        out.push_str(&format!("(cut after {MAX_PLAN_NODES} nodes)\n"));
    }
    if let Some(time) = plan.planning_ms {
        out.push_str(&format!("Planning: {}\n", ms(time)));
    }
    if let Some(time) = plan.execution_ms {
        out.push_str(&format!("Execution: {}\n", ms(time)));
    }
    if plan.suggestions.is_empty() {
        out.push_str("aidb found nothing to suggest in this plan.\n");
    } else {
        out.push_str("aidb's findings, from the plan's own numbers:\n");
        for suggestion in &plan.suggestions {
            out.push_str(&format!("- {}\n", suggestion.title));
            for evidence in &suggestion.evidence {
                out.push_str(&format!("  {evidence}\n"));
            }
            if let Some(sql) = &suggestion.sql {
                out.push_str(&format!("  Suggested: {sql}\n"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
