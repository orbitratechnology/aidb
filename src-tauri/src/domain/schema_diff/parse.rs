//! A schema from a `.sql` file: the `CREATE TABLE`, `CREATE INDEX`, `ALTER TABLE ... ADD`,
//! `CREATE VIEW` and serial and identity statements that describe tables and views, as
//! `pg_dump --schema-only` writes them or a person does. Nothing in the file runs: it is parsed
//! with `sqlparser`, and what it cannot describe is listed as not compared.

use std::collections::{BTreeMap, HashMap};

use sqlparser::ast::{
    AlterColumnOperation, AlterTableOperation, ColumnOption, CreateIndex, CreateTable, GeneratedAs,
    Ident, ObjectName, ObjectNamePart, Statement, TableConstraint,
};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;
use sqlparser::tokenizer::{Token, TokenWithSpan, Tokenizer};

use super::normalize::{canonical_type, serial_type};
use super::{
    Column, Constraint, ConstraintKind, ForeignKeyDefinition, Index, SchemaModel, Table, View,
};
use crate::domain::catalog::{Identity, ReferentialAction};

/// The schema PostgreSQL puts unqualified names in by default.
const DEFAULT_SCHEMA: &str = "public";

/// A statement of the file, with the line it starts on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Piece {
    line: u64,
    text: String,
    tokens: Vec<Token>,
}

/// The file could not be split into statements (an unclosed quote or comment).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("The file cannot be read as SQL: {0}")]
pub struct FileError(String);

fn pieces(text: &str) -> Result<Vec<Piece>, FileError> {
    let tokens = Tokenizer::new(&PostgreSqlDialect {}, text)
        .tokenize_with_location()
        .map_err(|error| FileError(error.message))?;
    let mut pieces = Vec::new();
    for group in tokens.split(|t| t.token == Token::SemiColon) {
        let meaningful: Vec<&TokenWithSpan> = group
            .iter()
            .filter(|t| !matches!(t.token, Token::Whitespace(_) | Token::EOF))
            .collect();
        let Some(first) = meaningful.first() else {
            continue;
        };
        let start = group
            .iter()
            .position(|t| std::ptr::eq(t, *first))
            .unwrap_or(0);
        pieces.push(Piece {
            line: first.span.start.line,
            text: group[start..]
                .iter()
                .map(|t| t.token.to_string())
                .collect::<String>()
                .trim_end()
                .to_owned(),
            tokens: meaningful.iter().map(|t| t.token.clone()).collect(),
        });
    }
    Ok(pieces)
}

/// The schemas a file's objects are in, in order of appearance (unqualified names are in
/// `public`), and how many statements it has.
///
/// # Errors
///
/// When the file cannot be split into statements.
pub fn survey(text: &str) -> Result<(Vec<String>, usize), FileError> {
    let pieces = pieces(text)?;
    let mut schemas: Vec<String> = Vec::new();
    for piece in &pieces {
        let Ok(mut statements) = Parser::parse_sql(&PostgreSqlDialect {}, &piece.text) else {
            continue;
        };
        let Some(statement) = statements.pop() else {
            continue;
        };
        let name = match &statement {
            Statement::CreateTable(table) => Some(&table.name),
            Statement::CreateView(view) => Some(&view.name),
            Statement::CreateIndex(index) => Some(&index.table_name),
            _ => None,
        };
        if let Some((schema, _)) = name.and_then(split_name) {
            let schema = schema.unwrap_or_else(|| DEFAULT_SCHEMA.to_owned());
            if !schemas.contains(&schema) {
                schemas.push(schema);
            }
        }
    }
    Ok((schemas, pieces.len()))
}

/// A name as PostgreSQL resolves it: bare names fold to lowercase.
fn fold(ident: &Ident) -> String {
    match ident.quote_style {
        None => ident.value.to_lowercase(),
        Some(_) => ident.value.clone(),
    }
}

/// `(schema, name)` of a one- or two-part name.
fn split_name(name: &ObjectName) -> Option<(Option<String>, String)> {
    let parts: Vec<String> = name
        .0
        .iter()
        .map(|part| match part {
            ObjectNamePart::Identifier(ident) => Some(fold(ident)),
            ObjectNamePart::Function(_) => None,
        })
        .collect::<Option<_>>()?;
    match parts.as_slice() {
        [name] => Some((None, name.clone())),
        [schema, name] => Some((Some(schema.clone()), name.clone())),
        _ => None,
    }
}

const fn action(action: Option<sqlparser::ast::ReferentialAction>) -> ReferentialAction {
    use sqlparser::ast::ReferentialAction as Parsed;
    match action {
        None | Some(Parsed::NoAction) => ReferentialAction::NoAction,
        Some(Parsed::Restrict) => ReferentialAction::Restrict,
        Some(Parsed::Cascade) => ReferentialAction::Cascade,
        Some(Parsed::SetNull) => ReferentialAction::SetNull,
        Some(Parsed::SetDefault) => ReferentialAction::SetDefault,
    }
}

/// What the file says about the schema, as it is read.
struct Reader<'a> {
    schema: &'a str,
    tables: Vec<Table>,
    views: Vec<View>,
    /// Sequences the file says a column owns: sequence name to (table, column).
    owned: HashMap<String, (String, String)>,
    /// Statements read that do not describe a table or a view, by kind ("function").
    other: BTreeMap<String, usize>,
    /// Statements in other schemas.
    elsewhere: usize,
    /// Statements that could not be read, with their line.
    unread: Vec<String>,
}

/// The objects of `schema` in a file. Statements about other schemas are left out, and what
/// cannot be read or compared is listed in `not_compared`.
///
/// # Errors
///
/// When the file cannot be split into statements.
pub fn read(text: &str, schema: &str) -> Result<SchemaModel, FileError> {
    let mut reader = Reader {
        schema,
        tables: Vec::new(),
        views: Vec::new(),
        owned: HashMap::new(),
        other: BTreeMap::new(),
        elsewhere: 0,
        unread: Vec::new(),
    };
    for piece in pieces(text)? {
        reader.piece(&piece);
    }
    Ok(reader.finish())
}

impl Reader<'_> {
    /// Whether a name is in the schema read, and its object name.
    fn own(&mut self, name: &ObjectName) -> Option<String> {
        let (schema, name) = split_name(name)?;
        if schema.as_deref().unwrap_or(DEFAULT_SCHEMA) == self.schema {
            Some(name)
        } else {
            self.elsewhere += 1;
            None
        }
    }

    fn table_mut(&mut self, name: &str) -> Option<&mut Table> {
        self.tables.iter_mut().find(|table| table.name == name)
    }

    fn other(&mut self, kind: &str) {
        *self.other.entry(kind.to_owned()).or_default() += 1;
    }

    fn piece(&mut self, piece: &Piece) {
        // Forms pg_dump writes that sqlparser does not read.
        if self.sequence_owner(&piece.tokens) || self.identity(&piece.tokens) {
            return;
        }
        let statement = match Parser::parse_sql(&PostgreSqlDialect {}, &piece.text) {
            Ok(mut statements) if statements.len() == 1 => statements.remove(0),
            _ => {
                let words: Vec<String> = piece
                    .tokens
                    .iter()
                    .take(3)
                    .map(ToString::to_string)
                    .collect();
                self.unread
                    .push(format!("line {} ({} …)", piece.line, words.join(" ")));
                return;
            }
        };
        match statement {
            Statement::CreateTable(table) => self.create_table(&table),
            Statement::CreateIndex(index) => self.create_index(&index),
            Statement::AlterTable(alter) => {
                let Some(table) = self.own(&alter.name) else {
                    return;
                };
                for operation in alter.operations {
                    self.alter_table(&table, operation);
                }
            }
            Statement::CreateView(view) => {
                let Some(name) = self.own(&view.name) else {
                    return;
                };
                if view.materialized {
                    self.other("materialized view");
                    return;
                }
                self.views.retain(|existing| existing.name != name);
                self.views.push(View {
                    name,
                    definition: view.query.to_string(),
                    depends_on: Vec::new(),
                    reads: Vec::new(),
                });
            }
            // Settings, ownership, comments, privileges and schemas: nothing to compare.
            Statement::Set(_)
            | Statement::Query(_)
            | Statement::Comment { .. }
            | Statement::Grant { .. }
            | Statement::Revoke { .. }
            | Statement::CreateSchema { .. }
            | Statement::CreateSequence { .. }
            | Statement::StartTransaction { .. }
            | Statement::Commit { .. } => {}
            other => {
                let kind = statement_kind(&other.to_string());
                self.other(&kind);
            }
        }
    }

    /// `ALTER SEQUENCE s OWNED BY t.c`: the column `c` is a serial column.
    fn sequence_owner(&mut self, tokens: &[Token]) -> bool {
        let words = words(tokens);
        if !(words.len() > 4 && words[0] == "alter" && words[1] == "sequence") {
            return false;
        }
        let Some(by) = words.iter().position(|w| w == "owned") else {
            return true;
        };
        // The names after the keywords, read from the tokens to keep their case and quoting.
        let names = dotted_names(tokens);
        if let (Some(sequence), Some(owner)) = (names.first(), names.get(1))
            && words.get(by + 1).is_some_and(|w| w == "by")
        {
            let sequence_name = match sequence.as_slice() {
                [schema, name] if schema == self.schema => Some(name.clone()),
                [name] => Some(name.clone()),
                _ => None,
            };
            let column = match owner.as_slice() {
                [schema, table, column] if schema == self.schema => {
                    Some((table.clone(), column.clone()))
                }
                [table, column] => Some((table.clone(), column.clone())),
                _ => None,
            };
            if let (Some(sequence), Some(column)) = (sequence_name, column) {
                self.owned.insert(sequence, column);
            }
        }
        true
    }

    /// `ALTER TABLE t ALTER COLUMN c ADD GENERATED ... AS IDENTITY (options)`.
    fn identity(&mut self, tokens: &[Token]) -> bool {
        let words = words(tokens);
        let Some(add) = words
            .windows(2)
            .position(|w| w[0] == "add" && w[1] == "generated")
        else {
            return false;
        };
        if !(words.first().is_some_and(|w| w == "alter")
            && words.get(1).is_some_and(|w| w == "table")
            && words[add..].iter().any(|w| w == "identity"))
        {
            return false;
        }
        let identity = if words.get(add + 2).is_some_and(|w| w == "always") {
            Identity::Always
        } else {
            Identity::ByDefault
        };
        let names = dotted_names(tokens);
        let table = match names.first().map(Vec::as_slice) {
            Some([schema, table]) if schema == self.schema => Some(table.clone()),
            Some([table]) if self.schema == DEFAULT_SCHEMA => Some(table.clone()),
            _ => None,
        };
        let column = names.get(1).and_then(|name| name.last().cloned());
        if let (Some(table), Some(column)) = (table, column)
            && let Some(table) = self.table_mut(&table)
            && let Some(column) = table.columns.iter_mut().find(|c| c.name == column)
        {
            column.identity = Some(identity);
            column.nullable = false;
        }
        true
    }

    fn create_table(&mut self, create: &CreateTable) {
        let Some(name) = self.own(&create.name) else {
            return;
        };
        if create.temporary {
            return;
        }
        if create.partition_by.is_some()
            || create.inherits.is_some()
            || create.like.is_some()
            || create.query.is_some()
        {
            self.other(&format!(
                "table {name} (partitioned, inherited, LIKE or AS)"
            ));
            return;
        }
        let mut table = Table {
            name: name.clone(),
            ..Table::default()
        };
        for definition in &create.columns {
            let type_text = definition.data_type.to_string();
            let serial = serial_type(&type_text);
            let column_name = fold(&definition.name);
            let mut column = Column {
                name: column_name.clone(),
                data_type: canonical_type(&type_text),
                nullable: !serial,
                default: serial.then(|| {
                    format!(
                        "nextval('{}'::regclass)",
                        serial_sequence(&name, &column_name)
                    )
                }),
                identity: None,
                generated: None,
                serial,
            };
            for option in &definition.options {
                match &option.option {
                    ColumnOption::Null => column.nullable = true,
                    ColumnOption::NotNull => column.nullable = false,
                    ColumnOption::Default(expression) => {
                        column.default = Some(expression.to_string());
                    }
                    ColumnOption::PrimaryKey(key) => {
                        column.nullable = false;
                        table.constraints.push(Constraint {
                            name: key.name.as_ref().map(fold),
                            kind: ConstraintKind::PrimaryKey(vec![column_name.clone()]),
                        });
                    }
                    ColumnOption::Unique(unique) => table.constraints.push(Constraint {
                        name: unique.name.as_ref().map(fold),
                        kind: ConstraintKind::Unique(vec![column_name.clone()]),
                    }),
                    ColumnOption::ForeignKey(key) => {
                        if let Some(kind) = foreign_key(
                            vec![column_name.clone()],
                            &key.foreign_table,
                            &key.referred_columns,
                            key.on_update,
                            key.on_delete,
                        ) {
                            table.constraints.push(Constraint {
                                name: key.name.as_ref().map(fold),
                                kind,
                            });
                        }
                    }
                    ColumnOption::Check(check) => table.constraints.push(Constraint {
                        // PostgreSQL's name for it, so a database's copy is found by name.
                        name: Some(
                            check
                                .name
                                .as_ref()
                                .map_or_else(|| format!("{name}_{column_name}_check"), fold),
                        ),
                        kind: ConstraintKind::Check(check.expr.to_string()),
                    }),
                    ColumnOption::Generated {
                        generated_as,
                        generation_expr,
                        ..
                    } => match (generated_as, generation_expr) {
                        (GeneratedAs::ExpStored | GeneratedAs::Always, Some(expression)) => {
                            column.generated = Some(expression.to_string());
                        }
                        (GeneratedAs::Always, None) => {
                            column.identity = Some(Identity::Always);
                            column.nullable = false;
                        }
                        (GeneratedAs::ByDefault, _) => {
                            column.identity = Some(Identity::ByDefault);
                            column.nullable = false;
                        }
                        (GeneratedAs::ExpStored, None) => {}
                    },
                    _ => {}
                }
            }
            table.columns.push(column);
        }
        for constraint in &create.constraints {
            add_table_constraint(&mut table, constraint);
        }
        self.tables.retain(|existing| existing.name != name);
        self.tables.push(table);
    }

    fn create_index(&mut self, create: &CreateIndex) {
        let Some(table) = self.own(&create.table_name) else {
            return;
        };
        let name = create
            .name
            .as_ref()
            .and_then(split_name)
            .map(|(_, name)| name);
        let mut definition = format!(
            "USING {} ({})",
            create.using.as_ref().map_or_else(
                || "btree".to_owned(),
                |using| using.to_string().to_lowercase()
            ),
            create
                .columns
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
        if !create.include.is_empty() {
            definition.push_str(&format!(
                " INCLUDE ({})",
                create
                    .include
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if create.nulls_distinct == Some(false) {
            definition.push_str(" NULLS NOT DISTINCT");
        }
        if let Some(predicate) = &create.predicate {
            definition.push_str(&format!(" WHERE ({predicate})"));
        }
        let index = Index {
            name,
            unique: create.unique,
            definition,
        };
        match self.table_mut(&table) {
            Some(table) => table.indexes.push(index),
            None => self.other("index on a table not in the file"),
        }
    }

    fn alter_table(&mut self, table: &str, operation: AlterTableOperation) {
        let owned = self.owned.clone();
        let Some(target) = self.table_mut(table) else {
            return;
        };
        match operation {
            AlterTableOperation::AddConstraint { constraint, .. } => {
                add_table_constraint(target, &constraint);
            }
            AlterTableOperation::AlterColumn { column_name, op } => {
                let name = fold(&column_name);
                let table_name = target.name.clone();
                let Some(column) = target.columns.iter_mut().find(|c| c.name == name) else {
                    return;
                };
                match op {
                    AlterColumnOperation::SetDefault { value } => {
                        let default = value.to_string();
                        column.serial = super::normalize::nextval_sequence(&default)
                            .and_then(|sequence| {
                                let sequence =
                                    sequence.rsplit('.').next().unwrap_or_default().to_owned();
                                owned.get(&sequence).cloned()
                            })
                            .is_some_and(|(t, c)| t == table_name && c == name);
                        column.default = Some(default);
                    }
                    AlterColumnOperation::DropDefault => column.default = None,
                    AlterColumnOperation::SetNotNull => column.nullable = false,
                    AlterColumnOperation::DropNotNull => column.nullable = true,
                    _ => {}
                }
            }
            // Ownership, row security, storage settings: not compared.
            _ => {}
        }
    }

    fn finish(mut self) -> SchemaModel {
        // A serial column declared before its `OWNED BY` (pg_dump's order): mark it now.
        for table in &mut self.tables {
            for column in &mut table.columns {
                if let Some(sequence) = column
                    .default
                    .as_deref()
                    .and_then(super::normalize::nextval_sequence)
                {
                    let sequence = sequence.rsplit('.').next().unwrap_or_default();
                    if self
                        .owned
                        .get(sequence)
                        .is_some_and(|(t, c)| *t == table.name && *c == column.name)
                    {
                        column.serial = true;
                    }
                }
            }
            // Key columns cannot be NULL.
            let keys: Vec<String> = table
                .constraints
                .iter()
                .filter_map(|c| match &c.kind {
                    ConstraintKind::PrimaryKey(columns) => Some(columns.clone()),
                    _ => None,
                })
                .flatten()
                .collect();
            for column in &mut table.columns {
                if keys.contains(&column.name) {
                    column.nullable = false;
                }
            }
        }
        // `REFERENCES t` without columns means t's primary key.
        let keys: HashMap<String, Vec<String>> = self
            .tables
            .iter()
            .filter_map(|table| {
                table.constraints.iter().find_map(|c| match &c.kind {
                    ConstraintKind::PrimaryKey(columns) => {
                        Some((table.name.clone(), columns.clone()))
                    }
                    _ => None,
                })
            })
            .collect();
        for table in &mut self.tables {
            for constraint in &mut table.constraints {
                if let ConstraintKind::ForeignKey(key) = &mut constraint.kind
                    && key.referenced.is_empty()
                    && key.schema.as_deref().unwrap_or(self.schema) == self.schema
                    && let Some(columns) = keys.get(&key.table)
                {
                    key.referenced = columns.clone();
                }
            }
        }
        let mut not_compared: Vec<String> = self
            .other
            .iter()
            .map(|(kind, count)| {
                if *count == 1 {
                    format!("1 {kind}")
                } else {
                    format!("{count} {kind} statements")
                }
            })
            .collect();
        if self.elsewhere > 0 {
            not_compared.push(format!(
                "{} {} about other schemas",
                self.elsewhere,
                if self.elsewhere == 1 {
                    "statement"
                } else {
                    "statements"
                }
            ));
        }
        if !self.unread.is_empty() {
            let shown: Vec<&String> = self.unread.iter().take(5).collect();
            not_compared.push(format!(
                "{} {} aidb could not read: {}{}",
                self.unread.len(),
                if self.unread.len() == 1 {
                    "statement"
                } else {
                    "statements"
                },
                shown
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                if self.unread.len() > shown.len() {
                    ", …"
                } else {
                    ""
                }
            ));
        }
        SchemaModel {
            schema: self.schema.to_owned(),
            tables: self.tables,
            views: self.views,
            not_compared,
            stored_text: false,
        }
    }
}

/// The name PostgreSQL gives a serial column's sequence.
fn serial_sequence(table: &str, column: &str) -> String {
    format!("{table}_{column}_seq")
}

fn foreign_key(
    columns: Vec<String>,
    table: &ObjectName,
    referenced: &[Ident],
    on_update: Option<sqlparser::ast::ReferentialAction>,
    on_delete: Option<sqlparser::ast::ReferentialAction>,
) -> Option<ConstraintKind> {
    let (schema, table) = split_name(table)?;
    Some(ConstraintKind::ForeignKey(ForeignKeyDefinition {
        columns,
        schema,
        table,
        // An empty list means the referenced table's primary key; it is written out when the
        // key is known only by the database, so it is kept empty here.
        referenced: referenced.iter().map(fold).collect(),
        on_update: action(on_update),
        on_delete: action(on_delete),
    }))
}

fn index_columns(columns: &[sqlparser::ast::IndexColumn]) -> Vec<String> {
    columns
        .iter()
        .map(|column| match &column.column.expr {
            sqlparser::ast::Expr::Identifier(ident) => fold(ident),
            other => other.to_string(),
        })
        .collect()
}

fn add_table_constraint(table: &mut Table, constraint: &TableConstraint) {
    let (name, kind) = match constraint {
        TableConstraint::PrimaryKey(key) => (
            key.name.as_ref().map(fold),
            ConstraintKind::PrimaryKey(index_columns(&key.columns)),
        ),
        TableConstraint::Unique(unique) => (
            unique.name.as_ref().map(fold),
            ConstraintKind::Unique(index_columns(&unique.columns)),
        ),
        TableConstraint::Check(check) => {
            let expression = check.expr.to_string();
            let name = check
                .name
                .as_ref()
                .map_or_else(|| check_name(table, &expression), fold);
            (Some(name), ConstraintKind::Check(expression))
        }
        TableConstraint::ForeignKey(key) => {
            let Some(kind) = foreign_key(
                key.columns.iter().map(fold).collect(),
                &key.foreign_table,
                &key.referred_columns,
                key.on_update,
                key.on_delete,
            ) else {
                return;
            };
            (key.name.as_ref().map(fold), kind)
        }
        _ => return,
    };
    if let ConstraintKind::PrimaryKey(columns) = &kind {
        for column in &mut table.columns {
            if columns.contains(&column.name) {
                column.nullable = false;
            }
        }
    }
    table.constraints.push(Constraint { name, kind });
}

/// The name PostgreSQL gives an unnamed table check: the table and the first of its columns the
/// condition names.
fn check_name(table: &Table, expression: &str) -> String {
    let first = Tokenizer::new(&PostgreSqlDialect {}, expression)
        .tokenize()
        .unwrap_or_default()
        .into_iter()
        .find_map(|token| match token {
            Token::Word(word) => {
                let name = match word.quote_style {
                    None => word.value.to_lowercase(),
                    Some(_) => word.value,
                };
                table.columns.iter().any(|c| c.name == name).then_some(name)
            }
            _ => None,
        });
    match first {
        Some(column) => format!("{}_{column}_check", table.name),
        None => format!("{}_check", table.name),
    }
}

/// The statement's kind for the not-compared list: "function", "type", "trigger".
fn statement_kind(text: &str) -> String {
    let words: Vec<String> = text
        .split_whitespace()
        .take(4)
        .map(str::to_lowercase)
        .filter(|w| {
            !matches!(
                w.as_str(),
                "create" | "or" | "replace" | "alter" | "drop" | "unique"
            )
        })
        .collect();
    words.first().cloned().unwrap_or_else(|| "other".to_owned())
}

/// The unquoted words of a statement, lowercased, for matching its shape.
fn words(tokens: &[Token]) -> Vec<String> {
    tokens
        .iter()
        .filter_map(|token| match token {
            Token::Word(word) if word.quote_style.is_none() => Some(word.value.to_lowercase()),
            _ => None,
        })
        .collect()
}

/// Dotted names in a statement (`a.b.c`), with keywords left out: each name is the run of
/// identifiers joined by periods.
fn dotted_names(tokens: &[Token]) -> Vec<Vec<String>> {
    const KEYWORDS: &[&str] = &[
        "alter",
        "sequence",
        "owned",
        "by",
        "table",
        "only",
        "column",
        "add",
        "generated",
        "always",
        "default",
        "as",
        "identity",
        "if",
        "exists",
        "none",
    ];
    let mut names: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut after_period = false;
    for token in tokens {
        match token {
            Token::Word(word) => {
                let keyword = word.quote_style.is_none()
                    && KEYWORDS.contains(&word.value.to_lowercase().as_str());
                if keyword && !after_period {
                    if !current.is_empty() {
                        names.push(std::mem::take(&mut current));
                    }
                    continue;
                }
                if !after_period && !current.is_empty() {
                    names.push(std::mem::take(&mut current));
                }
                current.push(match word.quote_style {
                    None => word.value.to_lowercase(),
                    Some(_) => word.value.clone(),
                });
                after_period = false;
            }
            Token::Period => after_period = true,
            _ => {
                if !current.is_empty() {
                    names.push(std::mem::take(&mut current));
                }
                after_period = false;
                // Sequence options after `(`: no more names.
                if *token == Token::LParen {
                    break;
                }
            }
        }
    }
    if !current.is_empty() {
        names.push(current);
    }
    names
}

#[cfg(test)]
mod tests;
