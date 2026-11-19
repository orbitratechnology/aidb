//! What a statement does, read from its text: the operation, the objects it changes, and the
//! details that change its risk (a missing WHERE, rows written, destructive or safeguard-
//! disabling clauses). Nothing here judges; [`super::classify`] does.

use std::ops::ControlFlow;

use serde::Serialize;
use sqlparser::ast::{
    AlterColumnOperation, AlterTableOperation, CopyTarget, Expr, FromTable, ObjectName,
    ObjectNamePart, ObjectType, SetExpr, Statement, TableFactor, TableObject, TableWithJoins,
    Visit, Visitor,
};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

use crate::domain::sql::{self, BLOCKED_FUNCTIONS, SourceSpan, SqlProblem, StatementKind};

/// What a statement does, for risk purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Read,
    /// EXPLAIN without ANALYZE: plans, never runs.
    Explain,
    Insert,
    Update,
    Delete,
    Merge,
    Copy,
    /// Row locks or SELECT INTO: writes without being INSERT, UPDATE or DELETE.
    OtherWrite,
    CreateObject,
    AlterObject,
    DropObject,
    Truncate,
    Comment,
    /// VACUUM, ANALYZE, REINDEX, CLUSTER, REFRESH MATERIALIZED VIEW.
    Maintenance,
    /// GRANT and REVOKE.
    PrivilegeChange,
    /// Roles, extensions, server settings, procedures, anonymous code, server functions.
    ServerAdministration,
    /// SET, RESET, LOCK, LISTEN, PREPARE, cursors.
    SessionControl,
    /// BEGIN, COMMIT, ROLLBACK, SAVEPOINT.
    TransactionControl,
    Unknown,
}

/// One statement and what it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementFacts {
    pub span: SourceSpan,
    /// The statement's own text, for estimating its impact.
    pub text: String,
    pub kind: StatementKind,
    pub operation: Operation,
    /// The objects it writes or changes, as written (`orders`, `public."Orders"`).
    pub targets: Vec<String>,
    /// UPDATE or DELETE: whether a WHERE clause refers to any column. `Some(false)` means it
    /// would change every row.
    pub filtered: Option<bool>,
    /// Rows an INSERT writes, when its VALUES list says so.
    pub literal_rows: Option<u64>,
    /// Removes data or structure beyond the statement's own rows: DROP COLUMN, DROP
    /// CONSTRAINT, CASCADE.
    pub destructive: bool,
    /// Turns off a protection: triggers, row-level security, rules, replication role.
    pub disables_safeguards: bool,
    /// Cannot run inside a transaction (CONCURRENTLY, VACUUM).
    pub non_transactional: bool,
    /// Calls a function that acts outside the transaction or on the server.
    pub blocked_function: Option<String>,
    /// Copies to or from a server file or program.
    pub server_file_access: bool,
    /// Why the statement could not be read, when it could not.
    pub unreadable: Option<String>,
}

/// What the database says about a table a statement writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetTable {
    /// As the statement wrote it.
    pub target: String,
    /// `schema.table`, or `None` when no such table exists.
    pub resolved: Option<String>,
    pub can_insert: bool,
    pub can_update: bool,
    pub can_delete: bool,
    pub can_truncate: bool,
    /// Tables whose rows are deleted with this table's (`ON DELETE CASCADE`), as `schema.table`.
    pub cascades_to: Vec<String>,
}

/// The statements of some SQL, or why it could not be split.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlFacts {
    pub statements: Vec<StatementFacts>,
    pub problem: Option<SqlProblem>,
}

/// Splits `sql` and reads each statement.
#[must_use]
pub fn inspect(sql: &str) -> SqlFacts {
    let analysis = sql::analyze(sql);
    let statements = analysis
        .statements
        .into_iter()
        .map(|statement| {
            let mut facts = StatementFacts {
                span: statement.span,
                operation: operation_by_keywords(statement.kind, &statement.text),
                kind: statement.kind,
                targets: Vec::new(),
                filtered: None,
                literal_rows: None,
                destructive: false,
                disables_safeguards: false,
                non_transactional: false,
                blocked_function: None,
                server_file_access: false,
                unreadable: None,
                text: statement.text,
            };
            read_details(&mut facts);
            facts
        })
        .collect();
    SqlFacts {
        statements,
        problem: analysis.problem,
    }
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|word| !word.is_empty())
        .take(6)
        .map(str::to_ascii_uppercase)
        .collect()
}

/// The operation from the statement's kind and first keywords; robust to statements the parser
/// cannot read.
fn operation_by_keywords(kind: StatementKind, text: &str) -> Operation {
    let words = words(text);
    let first = words.first().map_or("", String::as_str);
    let second = words.get(1).map_or("", String::as_str);
    match kind {
        StatementKind::Read if first == "EXPLAIN" => Operation::Explain,
        StatementKind::Read => Operation::Read,
        StatementKind::Write => match first {
            "INSERT" => Operation::Insert,
            "UPDATE" => Operation::Update,
            "DELETE" => Operation::Delete,
            "MERGE" => Operation::Merge,
            "COPY" => Operation::Copy,
            _ => Operation::OtherWrite,
        },
        StatementKind::Schema => match (first, second) {
            (
                "CREATE" | "ALTER" | "DROP",
                "ROLE" | "USER" | "GROUP" | "EXTENSION" | "DATABASE" | "TABLESPACE" | "SYSTEM"
                | "SERVER" | "SUBSCRIPTION" | "PUBLICATION" | "LANGUAGE" | "FOREIGN" | "EVENT"
                | "OWNED",
            )
            | ("SECURITY" | "IMPORT", _) => Operation::ServerAdministration,
            ("CREATE", _) => Operation::CreateObject,
            ("ALTER", _) => Operation::AlterObject,
            ("DROP", _) => Operation::DropObject,
            ("TRUNCATE", _) => Operation::Truncate,
            ("COMMENT", _) => Operation::Comment,
            ("VACUUM" | "ANALYZE" | "REINDEX" | "CLUSTER" | "REFRESH", _) => Operation::Maintenance,
            _ => Operation::Unknown,
        },
        StatementKind::Session => match first {
            "BEGIN" | "START" | "COMMIT" | "END" | "ROLLBACK" | "ABORT" | "SAVEPOINT"
            | "RELEASE" => Operation::TransactionControl,
            _ => Operation::SessionControl,
        },
        StatementKind::Privileged => match first {
            "GRANT" | "REVOKE" => Operation::PrivilegeChange,
            _ => Operation::ServerAdministration,
        },
        StatementKind::Unknown => Operation::Unknown,
    }
}

fn parse(text: &str) -> Result<Statement, String> {
    let mut statements = Parser::new(&PostgreSqlDialect {})
        .try_with_sql(text)
        .and_then(|mut parser| parser.parse_statements())
        .map_err(|error| error.to_string())?;
    if statements.len() == 1 {
        Ok(statements.remove(0))
    } else {
        Err("not exactly one statement".into())
    }
}

/// Fills in the details the statement's syntax tree gives, and text-level hints for statements
/// the parser cannot read.
fn read_details(facts: &mut StatementFacts) {
    let upper = facts.text.to_ascii_uppercase();
    facts.non_transactional = contains_word(&upper, "CONCURRENTLY")
        || matches!(
            words(&facts.text).first().map(String::as_str),
            Some("VACUUM")
        );
    facts.disables_safeguards = disables_safeguards(&upper);

    let statement = match parse(&facts.text) {
        Ok(statement) => statement,
        Err(error) => {
            facts.unreadable = Some(error);
            return;
        }
    };

    let mut functions = FunctionFinder::default();
    let _ = statement.visit(&mut functions);
    facts.blocked_function = functions.blocked;

    match &statement {
        Statement::Insert(insert) => {
            if let TableObject::TableName(name) = &insert.table {
                facts.targets.push(name.to_string());
            }
            facts.literal_rows =
                insert
                    .source
                    .as_ref()
                    .and_then(|source| match source.body.as_ref() {
                        SetExpr::Values(values) => u64::try_from(values.rows.len()).ok(),
                        _ => None,
                    });
        }
        Statement::Update(update) => {
            push_relation(&mut facts.targets, &update.table);
            facts.filtered = Some(update.selection.as_ref().is_some_and(refers_to_columns));
        }
        Statement::Delete(delete) => {
            let tables = match &delete.from {
                FromTable::WithFromKeyword(tables) | FromTable::WithoutKeyword(tables) => tables,
            };
            for table in tables {
                push_relation(&mut facts.targets, table);
            }
            for name in &delete.tables {
                facts.targets.push(name.to_string());
            }
            facts.filtered = Some(delete.selection.as_ref().is_some_and(refers_to_columns));
        }
        Statement::Merge(merge) => {
            if let TableFactor::Table { name, .. } = &merge.table {
                facts.targets.push(name.to_string());
            }
        }
        Statement::Copy { target, to, .. } => {
            facts.server_file_access =
                matches!(target, CopyTarget::File { .. } | CopyTarget::Program { .. });
            if !to {
                facts.targets.push(copy_table(&facts.text));
            }
        }
        Statement::Truncate(truncate) => {
            facts
                .targets
                .extend(truncate.table_names.iter().map(|t| t.name.to_string()));
            facts.destructive = matches!(
                truncate.cascade,
                Some(sqlparser::ast::CascadeOption::Cascade)
            );
        }
        Statement::Drop {
            names,
            cascade,
            object_type,
            ..
        } => {
            facts.targets.extend(names.iter().map(ToString::to_string));
            facts.destructive = *cascade || matches!(object_type, ObjectType::Schema);
        }
        Statement::AlterTable(alter) => {
            facts.targets.push(alter.name.to_string());
            for operation in &alter.operations {
                match operation {
                    AlterTableOperation::DropColumn { .. }
                    | AlterTableOperation::DropConstraint { .. }
                    | AlterTableOperation::DropPrimaryKey { .. }
                    | AlterTableOperation::DropForeignKey { .. }
                    | AlterTableOperation::DropPartitions { .. } => facts.destructive = true,
                    AlterTableOperation::AlterColumn {
                        op: AlterColumnOperation::SetDataType { .. },
                        ..
                    } => {}
                    AlterTableOperation::DisableRowLevelSecurity
                    | AlterTableOperation::NoForceRowLevelSecurity
                    | AlterTableOperation::DisableTrigger { .. }
                    | AlterTableOperation::DisableRule { .. } => facts.disables_safeguards = true,
                    _ => {}
                }
            }
        }
        Statement::CreateTable(create) => facts.targets.push(create.name.to_string()),
        Statement::CreateView(view) => facts.targets.push(view.name.to_string()),
        Statement::CreateIndex(index) => {
            facts.targets.push(index.table_name.to_string());
            facts.non_transactional |= index.concurrently;
        }
        Statement::Comment { object_name, .. } => facts.targets.push(object_name.to_string()),
        Statement::Query(query) => {
            // Data-changing CTEs: WITH gone AS (DELETE FROM t ...) SELECT ...
            let mut writes = WriteFinder::default();
            let _ = query.visit(&mut writes);
            if let Some((operation, targets, filtered)) = writes.found {
                facts.operation = operation;
                facts.targets = targets;
                facts.filtered = filtered;
            }
        }
        _ => {}
    }
}

fn contains_word(upper: &str, word: &str) -> bool {
    upper
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|candidate| candidate == word)
}

/// Text-level: clauses that turn protections off, also in statements the parser cannot read.
fn disables_safeguards(upper: &str) -> bool {
    let compact: String = upper.split_whitespace().collect::<Vec<_>>().join(" ");
    [
        "DISABLE TRIGGER",
        "DISABLE ROW LEVEL SECURITY",
        "NO FORCE ROW LEVEL SECURITY",
        "DISABLE RULE",
        "SESSION_REPLICATION_ROLE",
        "ROW_SECURITY",
        "DEFAULT_TRANSACTION_READ_ONLY",
        "TRANSACTION_READ_ONLY",
    ]
    .iter()
    .any(|clause| compact.contains(clause))
}

/// The table of `COPY table FROM ...`, as written.
fn copy_table(text: &str) -> String {
    text.split_whitespace()
        .nth(1)
        .map(|name| name.split('(').next().unwrap_or(name).to_owned())
        .unwrap_or_default()
}

fn push_relation(targets: &mut Vec<String>, table: &TableWithJoins) {
    if let TableFactor::Table { name, .. } = &table.relation {
        targets.push(name.to_string());
    }
}

/// Whether an expression refers to any column (`WHERE 1 = 1` and `WHERE true` do not).
fn refers_to_columns(expr: &Expr) -> bool {
    #[derive(Default)]
    struct Columns(bool);
    impl Visitor for Columns {
        type Break = ();
        fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
            if matches!(expr, Expr::Identifier(_) | Expr::CompoundIdentifier(_)) {
                self.0 = true;
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        }
    }
    let mut columns = Columns::default();
    let _ = expr.visit(&mut columns);
    columns.0
}

#[derive(Default)]
struct FunctionFinder {
    blocked: Option<String>,
}

impl Visitor for FunctionFinder {
    type Break = ();

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
        if let Expr::Function(function) = expr {
            return self.check(&function.name);
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_table_factor(&mut self, factor: &TableFactor) -> ControlFlow<()> {
        match factor {
            TableFactor::Table {
                name,
                args: Some(_),
                ..
            }
            | TableFactor::Function { name, .. } => self.check(name),
            _ => ControlFlow::Continue(()),
        }
    }
}

impl FunctionFinder {
    fn check(&mut self, name: &ObjectName) -> ControlFlow<()> {
        if let Some(ObjectNamePart::Identifier(last)) = name.0.last() {
            let lower = last.value.to_lowercase();
            if BLOCKED_FUNCTIONS.contains(&lower.as_str()) {
                self.blocked = Some(lower);
                return ControlFlow::Break(());
            }
        }
        ControlFlow::Continue(())
    }
}

/// The first data-changing statement inside a query.
#[derive(Default)]
struct WriteFinder {
    found: Option<(Operation, Vec<String>, Option<bool>)>,
}

impl Visitor for WriteFinder {
    type Break = ();

    fn pre_visit_statement(&mut self, statement: &Statement) -> ControlFlow<()> {
        let mut targets = Vec::new();
        let found = match statement {
            Statement::Insert(insert) => {
                if let TableObject::TableName(name) = &insert.table {
                    targets.push(name.to_string());
                }
                Some((Operation::Insert, None))
            }
            Statement::Update(update) => {
                push_relation(&mut targets, &update.table);
                Some((
                    Operation::Update,
                    Some(update.selection.as_ref().is_some_and(refers_to_columns)),
                ))
            }
            Statement::Delete(delete) => {
                let (FromTable::WithFromKeyword(tables) | FromTable::WithoutKeyword(tables)) =
                    &delete.from;
                for table in tables {
                    push_relation(&mut targets, table);
                }
                Some((
                    Operation::Delete,
                    Some(delete.selection.as_ref().is_some_and(refers_to_columns)),
                ))
            }
            _ => None,
        };
        if let Some((operation, filtered)) = found {
            self.found = Some((operation, targets, filtered));
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_query(&mut self, query: &sqlparser::ast::Query) -> ControlFlow<()> {
        let operation = match query.body.as_ref() {
            SetExpr::Insert(statement)
            | SetExpr::Update(statement)
            | SetExpr::Delete(statement) => {
                return self.pre_visit_statement(statement);
            }
            _ if !query.locks.is_empty() => Operation::OtherWrite,
            SetExpr::Select(select) if select.into.is_some() => Operation::OtherWrite,
            _ => return ControlFlow::Continue(()),
        };
        self.found = Some((operation, Vec::new(), None));
        ControlFlow::Break(())
    }
}
