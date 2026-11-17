//! SQL text from the editor: split into statements, parsed with `sqlparser` (PostgreSQL dialect)
//! and classified. Only reads may run from the editor; everything else is refused with a reason.
//!
//! The classification is one safety layer, not the only one. Statements run with the extended
//! protocol (one command at a time), in a `READ ONLY` transaction that is always rolled back, under
//! the role's privileges. The function list below catches calls that act outside a transaction
//! or outlive its rollback; user-defined functions are not inspected.

use std::ops::ControlFlow;

use serde::Serialize;
use sqlparser::ast::{
    Expr, ObjectName, ObjectNamePart, Query, SetExpr, Statement, TableFactor, Visit, Visitor,
};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::{Parser, ParserError};
use sqlparser::tokenizer::{Location, Token, TokenWithSpan, Tokenizer};

/// Longest SQL text accepted, in bytes.
pub const MAX_SQL_BYTES: usize = 1024 * 1024;

/// Functions that act outside the transaction, outlive its rollback, touch server files or run
/// SQL given as text. Matched by name, ignoring schema and case.
const BLOCKED_FUNCTIONS: &[&str] = &[
    "pg_terminate_backend",
    "pg_cancel_backend",
    "pg_reload_conf",
    "pg_rotate_logfile",
    "pg_switch_wal",
    "pg_promote",
    "pg_create_restore_point",
    "pg_log_backend_memory_contexts",
    "set_config",
    "pg_advisory_lock",
    "pg_advisory_lock_shared",
    "pg_try_advisory_lock",
    "pg_try_advisory_lock_shared",
    "pg_advisory_unlock",
    "pg_advisory_unlock_shared",
    "pg_advisory_unlock_all",
    "lo_import",
    "lo_export",
    "pg_read_file",
    "pg_read_binary_file",
    "pg_ls_dir",
    "pg_stat_file",
    "pg_ls_logdir",
    "pg_ls_waldir",
    "pg_ls_tmpdir",
    "pg_ls_archive_statusdir",
    "pg_file_write",
    "pg_file_rename",
    "pg_file_unlink",
    "pg_notify",
    "dblink",
    "dblink_exec",
    "dblink_connect",
    "dblink_connect_u",
    "dblink_send_query",
    "query_to_xml",
    "query_to_xmlschema",
    "query_to_xml_and_xmlschema",
];

/// A place in the submitted text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SourcePosition {
    /// From 1.
    pub line: u32,
    /// From 1, in characters.
    pub column: u32,
    /// From 0, in UTF-16 code units, as editors in the webview count.
    pub offset: u32,
}

/// A statement's extent: `start` is its first character, `end` just after its last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SourceSpan {
    pub start: SourcePosition,
    pub end: SourcePosition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum StatementKind {
    /// SELECT, VALUES, TABLE or WITH that only reads; EXPLAIN; SHOW.
    Read,
    /// Changes rows: INSERT, UPDATE, DELETE, MERGE, COPY, SELECT INTO, locking reads.
    Write,
    /// Changes structure or runs maintenance: CREATE, ALTER, DROP, TRUNCATE, VACUUM, ...
    Schema,
    /// Transaction control and session state: BEGIN, COMMIT, SET, LISTEN, PREPARE, ...
    Session,
    /// Privileges, procedures, server administration or functions with outside effects.
    Privileged,
    /// Not understood by the parser.
    Unknown,
}

/// How a read is executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadShape {
    /// Returns rows that can be wrapped in a sub-select (SELECT, VALUES, TABLE, WITH).
    Query,
    /// EXPLAIN or SHOW: runs as written; returns text.
    Utility,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct AnalyzedStatement {
    pub span: SourceSpan,
    pub kind: StatementKind,
    /// Why the statement cannot run from the editor; absent when it can.
    pub reason: Option<String>,
    /// Where the parser stopped, for statements it could not read.
    pub problem_at: Option<SourcePosition>,
    /// The statement's text, exactly as submitted.
    #[serde(skip)]
    pub text: String,
    /// Characters before the statement in the submitted text.
    #[serde(skip)]
    pub start_char: usize,
    #[serde(skip)]
    pub shape: Option<ReadShape>,
}

impl AnalyzedStatement {
    #[must_use]
    pub const fn runnable(&self) -> bool {
        self.reason.is_none()
    }
}

/// The statements in a piece of SQL, in order. Empty pieces (`;;`, comments) are skipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SqlAnalysis {
    pub statements: Vec<AnalyzedStatement>,
    /// Set when the text could not even be split (an unterminated string or comment).
    pub problem: Option<SqlProblem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "camelCase")]
pub struct SqlProblem {
    pub message: String,
    pub position: Option<SourcePosition>,
}

/// Splits and classifies `sql`.
#[must_use]
pub fn analyze(sql: &str) -> SqlAnalysis {
    let index = TextIndex::new(sql);
    let tokens = match Tokenizer::new(&PostgreSqlDialect {}, sql).tokenize_with_location() {
        Ok(tokens) => tokens,
        Err(error) => {
            return SqlAnalysis {
                statements: Vec::new(),
                problem: Some(SqlProblem {
                    message: sentence(&error.message),
                    position: index.position_of(error.location),
                }),
            };
        }
    };

    let mut statements = Vec::new();
    for piece in tokens.split(|t| t.token == Token::SemiColon) {
        let meaningful: Vec<&TokenWithSpan> = piece
            .iter()
            .filter(|t| !matches!(t.token, Token::Whitespace(_) | Token::EOF))
            .collect();
        let (Some(first), Some(last)) = (meaningful.first(), meaningful.last()) else {
            continue;
        };
        let (Some(start), Some(end)) = (
            index.byte_of(first.span.start),
            index.byte_of(last.span.end),
        ) else {
            continue;
        };
        statements.push(classify_piece(&index, sql, start, end));
    }
    SqlAnalysis {
        statements,
        problem: None,
    }
}

fn classify_piece(index: &TextIndex, sql: &str, start: usize, end: usize) -> AnalyzedStatement {
    let text = sql[start..end].to_owned();
    let span = SourceSpan {
        start: index.position_at(start),
        end: index.position_at(end),
    };
    let start_char = sql[..start].chars().count();
    let mut statement = AnalyzedStatement {
        span,
        kind: StatementKind::Unknown,
        reason: None,
        problem_at: None,
        text,
        start_char,
        shape: None,
    };

    let parsed = Parser::new(&PostgreSqlDialect {})
        .try_with_sql(&statement.text)
        .and_then(|mut parser| parser.parse_statements());
    match parsed {
        Ok(mut parsed) if parsed.len() == 1 => {
            let ast = parsed.remove(0);
            let (kind, shape, reason) = classify(&ast, &statement.text);
            statement.kind = kind;
            statement.shape = shape;
            statement.reason = reason;
        }
        Ok(_) => {
            statement.reason = Some("aidb could not tell where this statement ends.".into());
        }
        Err(error) => {
            let (message, location) = parser_problem(&error);
            statement.reason = Some(message);
            statement.problem_at = location.and_then(|location| {
                // The location is within the statement; shift it to the submitted text.
                let inner = TextIndex::new(&statement.text).byte_of(location)?;
                Some(index.position_at(start + inner))
            });
        }
    }
    statement
}

/// Kind, how to run it, and why not when it cannot run.
fn classify(ast: &Statement, text: &str) -> (StatementKind, Option<ReadShape>, Option<String>) {
    match ast {
        Statement::Query(query) => match inspect_query(query) {
            None => (StatementKind::Read, Some(ReadShape::Query), None),
            Some((kind, reason)) => (kind, None, Some(reason)),
        },
        Statement::Explain {
            analyze, statement, ..
        } => {
            if !analyze {
                // EXPLAIN without ANALYZE plans the statement without running it.
                return (StatementKind::Read, Some(ReadShape::Utility), None);
            }
            // The inner statement's own text, so its first keyword is its own, not EXPLAIN.
            match classify(statement, &statement.to_string()) {
                (StatementKind::Read, _, None) => {
                    (StatementKind::Read, Some(ReadShape::Utility), None)
                }
                (kind, _, reason) => (
                    kind,
                    None,
                    Some(format!(
                        "EXPLAIN ANALYZE runs the statement. {}",
                        reason.unwrap_or_default()
                    )),
                ),
            }
        }
        Statement::ShowVariable { .. } => (StatementKind::Read, Some(ReadShape::Utility), None),
        _ => {
            let kind = kind_by_keyword(text);
            (kind, None, Some(refusal(kind)))
        }
    }
}

/// Looks through a query for anything that is not a plain read.
fn inspect_query(query: &Query) -> Option<(StatementKind, String)> {
    let mut finder = Finder::default();
    let _ = query.visit(&mut finder);
    finder.found
}

#[derive(Default)]
struct Finder {
    found: Option<(StatementKind, String)>,
}

impl Finder {
    fn flag(&mut self, kind: StatementKind, reason: impl Into<String>) -> ControlFlow<()> {
        self.found = Some((kind, reason.into()));
        ControlFlow::Break(())
    }

    fn check_function(&mut self, name: &ObjectName) -> ControlFlow<()> {
        let Some(ObjectNamePart::Identifier(last)) = name.0.last() else {
            return ControlFlow::Continue(());
        };
        let lower = last.value.to_lowercase();
        if BLOCKED_FUNCTIONS.contains(&lower.as_str()) {
            return self.flag(
                StatementKind::Privileged,
                format!(
                    "{lower}() acts outside the transaction or on the server itself, so aidb does \
                     not run it from the editor."
                ),
            );
        }
        ControlFlow::Continue(())
    }
}

impl Visitor for Finder {
    type Break = ();

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<()> {
        if !query.locks.is_empty() {
            return self.flag(
                StatementKind::Write,
                "FOR UPDATE and FOR SHARE lock rows, which a read-only run cannot do.",
            );
        }
        match query.body.as_ref() {
            SetExpr::Insert(_) | SetExpr::Update(_) | SetExpr::Delete(_) | SetExpr::Merge(_) => {
                self.flag(StatementKind::Write, WRITE_REFUSAL)
            }
            SetExpr::Select(select) if select.into.is_some() => self.flag(
                StatementKind::Write,
                "SELECT INTO creates a table. Use CREATE TABLE AS through a change instead.",
            ),
            _ => ControlFlow::Continue(()),
        }
    }

    fn pre_visit_statement(&mut self, statement: &Statement) -> ControlFlow<()> {
        // Data-changing statements nested in a query (WITH x AS (DELETE ...) SELECT ...).
        if matches!(statement, Statement::Query(_)) {
            ControlFlow::Continue(())
        } else {
            self.flag(StatementKind::Write, WRITE_REFUSAL)
        }
    }

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
        if let Expr::Function(function) = expr {
            return self.check_function(&function.name);
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
            | TableFactor::Function { name, .. } => self.check_function(name),
            _ => ControlFlow::Continue(()),
        }
    }
}

const WRITE_REFUSAL: &str = "This statement changes data. The SQL editor runs reads only; \
                             changes need a preview and your approval.";

fn refusal(kind: StatementKind) -> String {
    match kind {
        StatementKind::Write => WRITE_REFUSAL.to_owned(),
        StatementKind::Schema => "This statement changes the database's structure or runs \
                                  maintenance. The SQL editor runs reads only."
            .to_owned(),
        StatementKind::Session => "Transaction and session commands are not run from the editor: \
                                   each statement runs alone, in a read-only transaction."
            .to_owned(),
        StatementKind::Privileged => "This statement changes privileges, runs a procedure or \
                                      administers the server. The SQL editor runs reads only."
            .to_owned(),
        StatementKind::Read | StatementKind::Unknown => {
            "aidb runs queries (SELECT, VALUES, TABLE, WITH), EXPLAIN and SHOW from the editor."
                .to_owned()
        }
    }
}

/// A category for statements that cannot run, from their first keyword.
fn kind_by_keyword(text: &str) -> StatementKind {
    let first = text
        .split(|c: char| !c.is_ascii_alphabetic())
        .find(|word| !word.is_empty())
        .unwrap_or_default()
        .to_ascii_uppercase();
    match first.as_str() {
        "INSERT" | "UPDATE" | "DELETE" | "MERGE" | "COPY" | "UPSERT" => StatementKind::Write,
        "CREATE" | "ALTER" | "DROP" | "TRUNCATE" | "COMMENT" | "REINDEX" | "CLUSTER" | "VACUUM"
        | "ANALYZE" | "REFRESH" | "SECURITY" | "IMPORT" => StatementKind::Schema,
        "BEGIN" | "START" | "COMMIT" | "END" | "ROLLBACK" | "ABORT" | "SAVEPOINT" | "RELEASE"
        | "SET" | "RESET" | "LISTEN" | "NOTIFY" | "UNLISTEN" | "DISCARD" | "PREPARE"
        | "EXECUTE" | "DEALLOCATE" | "LOCK" | "DECLARE" | "FETCH" | "MOVE" | "CLOSE" => {
            StatementKind::Session
        }
        "GRANT" | "REVOKE" | "DO" | "CALL" | "REASSIGN" | "CHECKPOINT" | "LOAD" => {
            StatementKind::Privileged
        }
        _ => StatementKind::Unknown,
    }
}

/// The parser's message without its location suffix, and the location if it gave one.
fn parser_problem(error: &ParserError) -> (String, Option<Location>) {
    let raw = match error {
        ParserError::TokenizerError(message) | ParserError::ParserError(message) => message.clone(),
        ParserError::RecursionLimitExceeded => {
            return ("This statement is nested too deeply to read.".into(), None);
        }
    };
    let (message, location) = match raw.rsplit_once(" at Line: ") {
        Some((message, rest)) => {
            let location = rest.split_once(", Column: ").and_then(|(line, column)| {
                Some(Location {
                    line: line.trim().parse().ok()?,
                    column: column.trim().parse().ok()?,
                })
            });
            (message.to_owned(), location)
        }
        None => (raw, None),
    };
    (sentence(&message), location)
}

fn sentence(message: &str) -> String {
    let mut text = message.trim().to_owned();
    if let Some(first) = text.get(..1) {
        let upper = first.to_uppercase();
        text.replace_range(..1, &upper);
    }
    if !text.ends_with('.') {
        text.push('.');
    }
    text
}

/// Converts between byte offsets, sqlparser locations (line and character column, from 1) and
/// [`SourcePosition`]s.
#[derive(Debug)]
pub struct TextIndex<'a> {
    text: &'a str,
    /// Byte offset where each line starts.
    lines: Vec<usize>,
}

impl<'a> TextIndex<'a> {
    #[must_use]
    pub fn new(text: &'a str) -> Self {
        let mut lines = vec![0];
        lines.extend(text.match_indices('\n').map(|(at, _)| at + 1));
        Self { text, lines }
    }

    /// Byte offset of a parser location, if it lies in the text.
    #[must_use]
    pub fn byte_of(&self, location: Location) -> Option<usize> {
        let line = usize::try_from(location.line).ok()?.checked_sub(1)?;
        let column = usize::try_from(location.column).ok()?.checked_sub(1)?;
        let start = *self.lines.get(line)?;
        let rest = &self.text[start..];
        if column == 0 {
            return Some(start);
        }
        rest.char_indices()
            .nth(column)
            .map(|(at, _)| start + at)
            .or_else(|| (rest.chars().count() == column).then_some(start + rest.len()))
    }

    fn position_of(&self, location: Location) -> Option<SourcePosition> {
        self.byte_of(location).map(|byte| self.position_at(byte))
    }

    /// The position of a byte offset (on a character boundary).
    #[must_use]
    pub fn position_at(&self, byte: usize) -> SourcePosition {
        let byte = byte.min(self.text.len());
        let line = self.lines.partition_point(|&start| start <= byte) - 1;
        let line_start = self.lines[line];
        let column = self.text[line_start..byte].chars().count() + 1;
        let offset = self.text[..byte].encode_utf16().count();
        SourcePosition {
            line: u32::try_from(line + 1).unwrap_or(u32::MAX),
            column: u32::try_from(column).unwrap_or(u32::MAX),
            offset: u32::try_from(offset).unwrap_or(u32::MAX),
        }
    }

    /// The position of the `char_offset`-th character (from 0).
    #[must_use]
    pub fn position_of_char(&self, char_offset: usize) -> SourcePosition {
        let byte = self
            .text
            .char_indices()
            .nth(char_offset)
            .map_or(self.text.len(), |(at, _)| at);
        self.position_at(byte)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(sql: &str) -> Vec<StatementKind> {
        analyze(sql).statements.iter().map(|s| s.kind).collect()
    }

    fn only(sql: &str) -> AnalyzedStatement {
        let mut analysis = analyze(sql);
        assert_eq!(analysis.statements.len(), 1, "{sql}");
        analysis.statements.remove(0)
    }

    #[test]
    fn reads_run_and_say_how() {
        for sql in [
            "SELECT 1",
            "select * from orders where id = 4 order by id limit 10",
            "WITH recent AS (SELECT * FROM orders) SELECT count(*) FROM recent",
            "VALUES (1, 'a'), (2, 'b')",
            "SELECT a FROM t UNION ALL SELECT b FROM u",
        ] {
            let statement = only(sql);
            assert_eq!(statement.kind, StatementKind::Read, "{sql}");
            assert_eq!(statement.shape, Some(ReadShape::Query), "{sql}");
            assert!(statement.runnable(), "{sql}");
        }
        for sql in [
            "EXPLAIN SELECT 1",
            "EXPLAIN DELETE FROM t",
            "SHOW search_path",
        ] {
            let statement = only(sql);
            assert_eq!(statement.shape, Some(ReadShape::Utility), "{sql}");
            assert!(statement.runnable(), "{sql}");
        }
    }

    #[test]
    fn changes_are_refused_with_a_reason() {
        let cases = [
            ("INSERT INTO t VALUES (1)", StatementKind::Write),
            ("update t set a = 1", StatementKind::Write),
            ("DELETE FROM t", StatementKind::Write),
            (
                "WITH gone AS (DELETE FROM t RETURNING *) SELECT * FROM gone",
                StatementKind::Write,
            ),
            ("SELECT * INTO copy FROM t", StatementKind::Write),
            ("SELECT * FROM t FOR UPDATE", StatementKind::Write),
            ("EXPLAIN ANALYZE DELETE FROM t", StatementKind::Write),
            ("COPY t FROM '/tmp/x'", StatementKind::Write),
            ("DROP TABLE t", StatementKind::Schema),
            ("CREATE TABLE t (a int)", StatementKind::Schema),
            ("TRUNCATE t", StatementKind::Schema),
            ("VACUUM t", StatementKind::Schema),
            ("BEGIN", StatementKind::Session),
            ("COMMIT", StatementKind::Session),
            ("SET statement_timeout = 0", StatementKind::Session),
            ("GRANT SELECT ON t TO bob", StatementKind::Privileged),
            ("SELECT pg_terminate_backend(42)", StatementKind::Privileged),
            (
                "select PG_CATALOG.set_config('a', 'b', false)",
                StatementKind::Privileged,
            ),
            ("SELECT * FROM pg_ls_dir('.')", StatementKind::Privileged),
            (
                "SELECT query_to_xml('DELETE FROM t', true, true, '')",
                StatementKind::Privileged,
            ),
        ];
        for (sql, kind) in cases {
            let statement = only(sql);
            assert_eq!(statement.kind, kind, "{sql}");
            assert!(!statement.runnable(), "{sql} must not run");
            assert!(statement.reason.is_some_and(|r| r.ends_with('.')), "{sql}");
        }
    }

    #[test]
    fn statements_are_split_outside_strings_and_comments() {
        let sql = "SELECT ';' AS a; -- one; two\nSELECT $$;$$;; /* ; */ DELETE FROM t";

        let analysis = analyze(sql);

        assert_eq!(
            kinds(sql),
            [
                StatementKind::Read,
                StatementKind::Read,
                StatementKind::Write
            ]
        );
        let texts: Vec<&str> = analysis
            .statements
            .iter()
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(texts, ["SELECT ';' AS a", "SELECT $$;$$", "DELETE FROM t"]);
        let second = &analysis.statements[1];
        assert_eq!((second.span.start.line, second.span.start.column), (2, 1));
        assert_eq!(second.span.end.column, 13);
    }

    #[test]
    fn positions_count_characters_and_utf16_units() {
        let sql = "SELECT 'é😀';\n  SELECT 2";

        let analysis = analyze(sql);

        let second = &analysis.statements[1];
        assert_eq!((second.span.start.line, second.span.start.column), (2, 3));
        // "SELECT 'é😀';\n  " is 15 characters but 16 UTF-16 units (the emoji takes two).
        assert_eq!(second.span.start.offset, 16);
        assert_eq!(second.start_char, 15);
        assert_eq!(
            &sql[..]
                .chars()
                .skip(second.start_char)
                .take(6)
                .collect::<String>(),
            "SELECT"
        );
    }

    #[test]
    fn parse_errors_point_into_the_submitted_text() {
        let sql = "SELECT 1;\nSELECT FROM WHERE";

        let analysis = analyze(sql);

        let broken = &analysis.statements[1];
        assert_eq!(broken.kind, StatementKind::Unknown);
        assert!(!broken.runnable());
        let at = broken.problem_at.expect("a location");
        assert_eq!(at.line, 2, "{broken:?}");
        assert!(
            !broken
                .reason
                .as_deref()
                .unwrap_or_default()
                .contains("Line:")
        );
    }

    #[test]
    fn unterminated_text_is_a_problem_for_the_whole_input() {
        let analysis = analyze("SELECT 'oops");
        assert!(analysis.statements.is_empty());
        let problem = analysis.problem.expect("problem");
        assert!(problem.message.ends_with('.'));
        assert!(problem.position.is_some());
    }

    #[test]
    fn statements_the_parser_does_not_know_are_refused() {
        // `TABLE t` is valid PostgreSQL that this parser version does not read.
        let statement = only("TABLE orders");
        assert_eq!(statement.kind, StatementKind::Unknown);
        assert!(!statement.runnable());
    }

    #[test]
    fn empty_input_has_no_statements() {
        assert!(analyze("  ;; -- nothing\n").statements.is_empty());
    }

    #[test]
    fn char_offsets_map_to_positions() {
        let index = TextIndex::new("ab\ncdé\nf");
        assert_eq!(
            index.position_of_char(5),
            SourcePosition {
                line: 2,
                column: 3,
                offset: 5
            }
        );
        assert_eq!(index.position_of_char(99).line, 3);
    }
}
