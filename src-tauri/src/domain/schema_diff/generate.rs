//! The steps that make a target schema match a source schema, in a safe order, with what each
//! one risks.
//!
//! Objects are matched by name (tables, columns, views) or by what they are (constraints and
//! indexes: their kind, columns and normalized expression), so a constraint PostgreSQL named
//! differently in two databases is not dropped and re-added. Renames are not detected: a renamed
//! column is a dropped column and an added one, and the step says it deletes data.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

use super::normalize::{
    name_list, nextval_sequence, normalize_expression, qualified, quote_ident, retarget,
    serial_name, unqualify, volatile_default,
};
use super::{
    Column, Constraint, ConstraintKind, DiffSummary, ForeignKeyDefinition, GenerateOptions, Index,
    MigrationStep, NoteKind, SchemaModel, StepAction, StepNote, Table, View,
};
use crate::domain::catalog::{Identity, ReferentialAction};
use crate::domain::connections::Environment;
use crate::domain::policy::classify::{ImpactFacts, classify_statement};
use crate::domain::policy::facts::inspect;
use crate::domain::policy::risk::RiskLevel;

/// The order steps run in: what depends on an object goes before it is dropped and after it is
/// created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    DropView,
    DropForeignKey,
    DropIndex,
    DropConstraint,
    DropTable,
    CreateTable,
    AlterColumn,
    AddConstraint,
    CreateIndex,
    AddForeignKey,
    CreateView,
}

#[derive(Debug)]
struct Draft {
    phase: Phase,
    summary: String,
    action: StepAction,
    object: String,
    sql: String,
    transactional: bool,
    notes: Vec<StepNote>,
}

fn note(kind: NoteKind, text: impl Into<String>) -> StepNote {
    StepNote {
        kind,
        text: text.into(),
    }
}

/// The migration: steps in the order they run, counts, the whole script and its highest risk.
#[derive(Debug)]
pub struct Plan {
    pub steps: Vec<MigrationStep>,
    pub summary: DiffSummary,
    pub script: String,
    pub risk: RiskLevel,
    /// What the comparison could not decide, for the reader.
    pub notes: Vec<String>,
}

/// Steps that make `target` match `source`, judged for a connection in `environment`.
#[must_use]
pub fn plan(
    source: &SchemaModel,
    target: &SchemaModel,
    options: GenerateOptions,
    environment: Environment,
) -> Plan {
    let context = Context {
        from: &source.schema,
        to: &target.schema,
        options,
        reshaped: RefCell::default(),
        checks_as_text: source.stored_text && target.stored_text,
        matched_by_name: RefCell::default(),
    };
    let mut drafts = Vec::new();
    let mut summary = DiffSummary::default();
    context.tables(source, target, &mut drafts, &mut summary);
    context.views(source, target, &mut drafts, &mut summary);
    // Stable: within a phase, steps keep the order they were found in.
    drafts.sort_by_key(|draft| (!draft.transactional, draft.phase));
    let steps: Vec<MigrationStep> = drafts
        .into_iter()
        .map(|draft| {
            let risk = statement_risk(&draft.sql, environment);
            MigrationStep {
                summary: draft.summary,
                action: draft.action,
                object: draft.object,
                sql: draft.sql,
                transactional: draft.transactional,
                risk,
                notes: draft.notes,
            }
        })
        .collect();
    let risk = steps
        .iter()
        .map(|step| step.risk)
        .max()
        .unwrap_or(RiskLevel::R0);
    let matched = context.matched_by_name.into_inner();
    let notes = if matched.is_empty() {
        Vec::new()
    } else {
        vec![format!(
            "Check constraints compared by name only, since PostgreSQL stores a condition in its \
             own words: {}. Check that their conditions are the same.",
            matched.join(", ")
        )]
    };
    Plan {
        script: script(&steps, &target.schema),
        steps,
        summary,
        risk,
        notes,
    }
}

/// The policy's risk for one generated statement.
fn statement_risk(sql: &str, environment: Environment) -> RiskLevel {
    inspect(sql)
        .statements
        .first()
        .map_or(RiskLevel::R4, |facts| {
            classify_statement(facts, &ImpactFacts::unknown(), environment).risk
        })
}

/// The steps as one script: the transactional ones in a transaction with the target schema on
/// the search path (names in expressions resolve there), then the others.
fn script(steps: &[MigrationStep], target_schema: &str) -> String {
    if steps.is_empty() {
        return String::new();
    }
    let mut script = String::from("BEGIN;\n");
    script.push_str(&format!(
        "SET LOCAL search_path TO {};\n",
        quote_ident(target_schema)
    ));
    for step in steps.iter().filter(|step| step.transactional) {
        script.push('\n');
        script.push_str(&format!("-- {}\n{};\n", step.summary, step.sql));
    }
    script.push_str("\nCOMMIT;\n");
    let outside: Vec<&MigrationStep> = steps.iter().filter(|step| !step.transactional).collect();
    if !outside.is_empty() {
        script.push_str("\n-- Outside the transaction: these cannot run inside one.\n");
        script.push_str(&format!(
            "SET search_path TO {};\n",
            quote_ident(target_schema)
        ));
        for step in outside {
            script.push('\n');
            script.push_str(&format!("-- {}\n{};\n", step.summary, step.sql));
        }
    }
    script
}

struct Context<'a> {
    /// The source schema: its own names are written for the target.
    from: &'a str,
    to: &'a str,
    options: GenerateOptions,
    /// Existing columns (`table.column`) dropped or changing type: views that read them must
    /// be dropped first and created again after.
    reshaped: RefCell<BTreeSet<String>>,
    /// Whether check conditions on both sides are as PostgreSQL stores them. When one side is
    /// a file, a check whose condition reads differently is matched by its name.
    checks_as_text: bool,
    /// Checks matched that way, as `table.constraint`.
    matched_by_name: RefCell<Vec<String>>,
}

/// " (about 1,200 rows)", when the estimate says there are rows.
fn about(rows: Option<u64>) -> String {
    match rows {
        Some(0) | None => String::new(),
        Some(1) => " (about 1 row)".to_owned(),
        Some(rows) => format!(" (about {} rows)", thousands(rows)),
    }
}

fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

const BRIEF_LOCK: &str =
    "Takes an ACCESS EXCLUSIVE lock on the table for a moment; the table is not rewritten.";

impl Context<'_> {
    fn table(&self, name: &str) -> String {
        qualified(self.to, name)
    }

    /// A source expression or type, with the source schema's names moved to the target.
    fn text(&self, text: &str) -> String {
        retarget(text, self.from, self.to)
    }

    fn same_type(&self, source: &Column, target: &Column) -> bool {
        unqualify(&source.data_type, self.from) == unqualify(&target.data_type, self.to)
    }

    fn same_expression(&self, source: &str, target: &str) -> bool {
        normalize_expression(source, self.from) == normalize_expression(target, self.to)
    }

    fn column_sql(&self, column: &Column) -> String {
        let mut sql = quote_ident(&column.name);
        let serial = column
            .serial
            .then(|| serial_name(&column.data_type))
            .flatten();
        sql.push(' ');
        match serial {
            Some(name) => sql.push_str(name),
            None => sql.push_str(&self.text(&column.data_type)),
        }
        if let Some(expression) = &column.generated {
            sql.push_str(&format!(
                " GENERATED ALWAYS AS ({}) STORED",
                self.text(expression)
            ));
        }
        match column.identity {
            Some(Identity::Always) => sql.push_str(" GENERATED ALWAYS AS IDENTITY"),
            Some(Identity::ByDefault) => sql.push_str(" GENERATED BY DEFAULT AS IDENTITY"),
            None => {}
        }
        if serial.is_none()
            && let Some(default) = &column.default
        {
            sql.push_str(&format!(" DEFAULT {}", self.text(default)));
        }
        if !column.nullable && serial.is_none() && column.identity.is_none() {
            sql.push_str(" NOT NULL");
        }
        sql
    }

    fn constraint_sql(&self, constraint: &Constraint) -> String {
        let name = constraint
            .name
            .as_ref()
            .map(|name| format!("CONSTRAINT {} ", quote_ident(name)))
            .unwrap_or_default();
        let body = match &constraint.kind {
            ConstraintKind::PrimaryKey(columns) => format!("PRIMARY KEY ({})", name_list(columns)),
            ConstraintKind::Unique(columns) => format!("UNIQUE ({})", name_list(columns)),
            ConstraintKind::Check(expression) => format!("CHECK ({})", self.text(expression)),
            ConstraintKind::ForeignKey(key) => {
                let schema = match key.schema.as_deref() {
                    None => self.to,
                    Some(schema) if schema == self.from => self.to,
                    Some(schema) => schema,
                };
                let mut sql = format!(
                    "FOREIGN KEY ({}) REFERENCES {} ({})",
                    name_list(&key.columns),
                    qualified(schema, &key.table),
                    name_list(&key.referenced)
                );
                if key.on_update != ReferentialAction::NoAction {
                    sql.push_str(&format!(" ON UPDATE {}", action_sql(key.on_update)));
                }
                if key.on_delete != ReferentialAction::NoAction {
                    sql.push_str(&format!(" ON DELETE {}", action_sql(key.on_delete)));
                }
                sql
            }
        };
        format!("{name}{body}")
    }

    /// What a constraint is, for matching across schemas: names left out.
    fn constraint_key(&self, constraint: &Constraint, own: &str) -> String {
        match &constraint.kind {
            ConstraintKind::PrimaryKey(columns) => format!("p:{}", columns.join(",")),
            ConstraintKind::Unique(columns) => format!("u:{}", columns.join(",")),
            ConstraintKind::Check(expression) => {
                format!("c:{}", normalize_expression(expression, own))
            }
            ConstraintKind::ForeignKey(key) => format!(
                "f:{}>{}.{}({}):{}:{}",
                key.columns.join(","),
                key.schema
                    .as_deref()
                    .filter(|schema| *schema != own)
                    .unwrap_or(""),
                key.table,
                key.referenced.join(","),
                action_sql(key.on_update),
                action_sql(key.on_delete)
            ),
        }
    }

    fn index_sql(&self, table: &str, index: &Index) -> String {
        format!(
            "CREATE {}INDEX {}{}ON {} {}",
            if index.unique { "UNIQUE " } else { "" },
            if self.options.concurrent_indexes {
                "CONCURRENTLY "
            } else {
                ""
            },
            index
                .name
                .as_ref()
                .map(|name| format!("{} ", quote_ident(name)))
                .unwrap_or_default(),
            self.table(table),
            self.text(&index.definition)
        )
    }

    fn tables(
        &self,
        source: &SchemaModel,
        target: &SchemaModel,
        drafts: &mut Vec<Draft>,
        summary: &mut DiffSummary,
    ) {
        let existing: HashMap<&str, &Table> =
            target.tables.iter().map(|t| (t.name.as_str(), t)).collect();
        let wanted: HashMap<&str, &Table> =
            source.tables.iter().map(|t| (t.name.as_str(), t)).collect();
        for table in &target.tables {
            if !wanted.contains_key(table.name.as_str()) {
                summary.tables_removed += 1;
                drafts.push(Draft {
                    phase: Phase::DropTable,
                    summary: format!("Drop table {}", table.name),
                    action: StepAction::Remove,
                    object: table.name.clone(),
                    sql: format!("DROP TABLE {}", self.table(&table.name)),
                    transactional: true,
                    notes: vec![
                        note(
                            NoteKind::DataLoss,
                            format!("Deletes the table and all its rows{}.", about(table.rows)),
                        ),
                        note(
                            NoteKind::MayFail,
                            "Fails if a view or another table's foreign key depends on it.",
                        ),
                    ],
                });
            }
        }
        for table in &source.tables {
            match existing.get(table.name.as_str()) {
                None => {
                    summary.tables_added += 1;
                    self.create_table(table, drafts);
                }
                Some(current) => {
                    let before = drafts.len();
                    self.alter_table(table, current, drafts);
                    if drafts.len() > before {
                        summary.tables_changed += 1;
                    }
                }
            }
        }
    }

    fn create_table(&self, table: &Table, drafts: &mut Vec<Draft>) {
        let mut lines: Vec<String> = table
            .columns
            .iter()
            .map(|column| self.column_sql(column))
            .collect();
        // A new table is empty: its own constraints cost nothing to check here. Foreign keys
        // wait until every table exists.
        lines.extend(
            table
                .constraints
                .iter()
                .filter(|c| !matches!(c.kind, ConstraintKind::ForeignKey(_)))
                .map(|c| self.constraint_sql(c)),
        );
        drafts.push(Draft {
            phase: Phase::CreateTable,
            summary: format!("Create table {}", table.name),
            action: StepAction::Add,
            object: table.name.clone(),
            sql: format!(
                "CREATE TABLE {} (\n    {}\n)",
                self.table(&table.name),
                lines.join(",\n    ")
            ),
            transactional: true,
            notes: Vec::new(),
        });
        for index in &table.indexes {
            drafts.push(self.create_index(&table.name, index, None, true));
        }
        for constraint in &table.constraints {
            if matches!(constraint.kind, ConstraintKind::ForeignKey(_)) {
                drafts.push(self.add_constraint(&table.name, constraint, None, true));
            }
        }
    }

    fn alter_table(&self, wanted: &Table, current: &Table, drafts: &mut Vec<Draft>) {
        let name = &wanted.name;
        let rows = current.rows;
        let existing: HashMap<&str, &Column> = current
            .columns
            .iter()
            .map(|c| (c.name.as_str(), c))
            .collect();
        let kept: HashMap<&str, &Column> = wanted
            .columns
            .iter()
            .map(|c| (c.name.as_str(), c))
            .collect();

        // Constraints and indexes first, by what they are.
        let current_keys: Vec<String> = current
            .constraints
            .iter()
            .map(|c| self.constraint_key(c, self.to))
            .collect();
        let wanted_keys: Vec<String> = wanted
            .constraints
            .iter()
            .map(|c| self.constraint_key(c, self.from))
            .collect();
        // A check a file wrote one way and the database stores another: the same by name.
        let same_check = |a: &Constraint, others: &[Constraint]| {
            !self.checks_as_text
                && matches!(a.kind, ConstraintKind::Check(_))
                && a.name.is_some()
                && others
                    .iter()
                    .any(|b| matches!(b.kind, ConstraintKind::Check(_)) && b.name == a.name)
        };
        for (constraint, key) in current.constraints.iter().zip(&current_keys) {
            if !wanted_keys.contains(key) {
                if same_check(constraint, &wanted.constraints) {
                    self.matched_by_name.borrow_mut().push(format!(
                        "{name}.{}",
                        constraint.name.as_deref().unwrap_or_default()
                    ));
                    continue;
                }
                drafts.push(self.drop_constraint(name, constraint));
            }
        }
        let current_indexes: Vec<String> = current
            .indexes
            .iter()
            .map(|i| index_key(i, self.to))
            .collect();
        let wanted_indexes: Vec<String> = wanted
            .indexes
            .iter()
            .map(|i| index_key(i, self.from))
            .collect();
        for (index, key) in current.indexes.iter().zip(&current_indexes) {
            if !wanted_indexes.contains(key) {
                drafts.push(self.drop_index(name, index));
            }
        }

        for column in &wanted.columns {
            match existing.get(column.name.as_str()) {
                None => drafts.push(self.add_column(name, column, rows)),
                Some(present) => self.alter_column(name, column, present, rows, drafts),
            }
        }
        for column in &current.columns {
            if !kept.contains_key(column.name.as_str()) {
                drafts.push(self.drop_column(name, column, rows, true));
            }
        }

        for (constraint, key) in wanted.constraints.iter().zip(&wanted_keys) {
            if !current_keys.contains(key) && !same_check(constraint, &current.constraints) {
                drafts.push(self.add_constraint(name, constraint, rows, false));
            }
        }
        for (index, key) in wanted.indexes.iter().zip(&wanted_indexes) {
            if !current_indexes.contains(key) {
                drafts.push(self.create_index(name, index, rows, false));
            }
        }
    }

    fn alter(&self, table: &str, column: &str, action: &str) -> String {
        format!(
            "ALTER TABLE {} ALTER COLUMN {} {action}",
            self.table(table),
            quote_ident(column)
        )
    }

    fn add_column(&self, table: &str, column: &Column, rows: Option<u64>) -> Draft {
        let mut notes = Vec::new();
        let filled = column.default.is_some()
            || column.identity.is_some()
            || column.generated.is_some()
            || column.serial;
        if !column.nullable && !filled && rows != Some(0) {
            notes.push(note(
                NoteKind::MayFail,
                "Fails if the table has rows: the column cannot be empty and has no default.",
            ));
        }
        let rewrites = column.serial
            || column.identity.is_some()
            || column.generated.is_some()
            || column.default.as_deref().is_some_and(volatile_default);
        notes.push(note(
            NoteKind::Lock,
            if rewrites {
                format!(
                    "Rewrites the table{} to fill the column, under an ACCESS EXCLUSIVE lock that \
                     blocks reads and writes.",
                    about(rows)
                )
            } else {
                BRIEF_LOCK.to_owned()
            },
        ));
        Draft {
            phase: Phase::AlterColumn,
            summary: format!("Add column {} to {table}", column.name),
            action: StepAction::Add,
            object: table.to_owned(),
            sql: format!(
                "ALTER TABLE {} ADD COLUMN {}",
                self.table(table),
                self.column_sql(column)
            ),
            transactional: true,
            notes,
        }
    }

    fn drop_column(&self, table: &str, column: &Column, rows: Option<u64>, loses: bool) -> Draft {
        self.reshaped
            .borrow_mut()
            .insert(format!("{table}.{}", column.name));
        let mut notes = Vec::new();
        if loses {
            notes.push(note(
                NoteKind::DataLoss,
                format!("Deletes the column's value in every row{}.", about(rows)),
            ));
        }
        notes.push(note(
            NoteKind::MayFail,
            "Fails if another table's foreign key, or a view not compared here, uses the column.",
        ));
        notes.push(note(NoteKind::Lock, BRIEF_LOCK));
        Draft {
            phase: Phase::AlterColumn,
            summary: format!("Drop column {} from {table}", column.name),
            action: StepAction::Remove,
            object: table.to_owned(),
            sql: format!(
                "ALTER TABLE {} DROP COLUMN {}",
                self.table(table),
                quote_ident(&column.name)
            ),
            transactional: true,
            notes,
        }
    }

    fn alter_column(
        &self,
        table: &str,
        wanted: &Column,
        current: &Column,
        rows: Option<u64>,
        drafts: &mut Vec<Draft>,
    ) {
        let name = &wanted.name;
        let change = |summary: String, sql: String, notes: Vec<StepNote>| Draft {
            phase: Phase::AlterColumn,
            summary,
            action: StepAction::Change,
            object: table.to_owned(),
            sql,
            transactional: true,
            notes,
        };

        // A generated column's expression cannot be changed in place on every supported version:
        // drop it and add it again (its values are computed, so nothing is lost), or keep the
        // values as a plain column.
        let generated_differs = match (&wanted.generated, &current.generated) {
            (Some(a), Some(b)) => !self.same_expression(a, b),
            (None, None) => false,
            _ => true,
        };
        if generated_differs {
            if wanted.generated.is_none() {
                drafts.push(change(
                    format!("Keep the values of {table}.{name}, no longer generated"),
                    self.alter(table, name, "DROP EXPRESSION"),
                    vec![note(NoteKind::Lock, BRIEF_LOCK)],
                ));
            } else {
                drafts.push(self.drop_column(table, current, rows, current.generated.is_none()));
                drafts.push(self.add_column(table, wanted, rows));
                return;
            }
        }

        if !self.same_type(wanted, current) {
            self.reshaped.borrow_mut().insert(format!("{table}.{name}"));
            let data_type = self.text(&wanted.data_type);
            let widens = widens(&current.data_type, &wanted.data_type);
            let mut notes = Vec::new();
            if !widens {
                notes.push(note(
                    NoteKind::DataLoss,
                    "Values are converted: a narrower type can cut or round them, and a value \
                     that does not convert makes the step fail.",
                ));
            }
            notes.push(note(
                NoteKind::MayFail,
                "Fails if a view not compared here uses the column.",
            ));
            notes.push(note(
                NoteKind::Lock,
                if widens {
                    BRIEF_LOCK.to_owned()
                } else {
                    format!(
                        "Rewrites the table{} and its indexes under an ACCESS EXCLUSIVE lock that \
                         blocks reads and writes.",
                        about(rows)
                    )
                },
            ));
            drafts.push(change(
                format!(
                    "Change the type of {table}.{name} from {} to {}",
                    current.data_type, wanted.data_type
                ),
                self.alter(
                    table,
                    name,
                    &format!("TYPE {data_type} USING {}::{data_type}", quote_ident(name)),
                ),
                notes,
            ));
        }

        if current.identity.is_some() && wanted.identity.is_none() {
            drafts.push(change(
                format!("Stop filling {table}.{name} as an identity"),
                self.alter(table, name, "DROP IDENTITY"),
                vec![note(NoteKind::Lock, BRIEF_LOCK)],
            ));
        }

        let default_differs = match (&wanted.default, &current.default) {
            (Some(a), Some(b)) => !self.same_expression(a, b),
            (None, None) => false,
            _ => true,
        };
        if default_differs {
            match &wanted.default {
                None => drafts.push(change(
                    format!("Remove the default of {table}.{name}"),
                    self.alter(table, name, "DROP DEFAULT"),
                    vec![note(NoteKind::Lock, BRIEF_LOCK)],
                )),
                Some(default) => {
                    if wanted.serial
                        && let Some(sequence) = nextval_sequence(default)
                    {
                        let sequence = self.text(&format!("'{sequence}'::regclass"));
                        let sequence = sequence
                            .strip_prefix('\'')
                            .and_then(|s| s.strip_suffix("'::regclass"))
                            .unwrap_or(&sequence)
                            .to_owned();
                        drafts.push(change(
                            format!("Create the sequence for {table}.{name}"),
                            format!(
                                "CREATE SEQUENCE IF NOT EXISTS {sequence} OWNED BY {}.{}",
                                self.table(table),
                                quote_ident(name)
                            ),
                            vec![note(
                                NoteKind::MayFail,
                                "The sequence starts at 1. If the column has values, move it \
                                 past the highest with setval(), or new rows will clash.",
                            )],
                        ));
                    }
                    drafts.push(change(
                        format!("Set the default of {table}.{name}"),
                        self.alter(table, name, &format!("SET DEFAULT {}", self.text(default))),
                        vec![note(NoteKind::Lock, BRIEF_LOCK)],
                    ));
                }
            }
        }

        if wanted.nullable != current.nullable && wanted.identity.is_none() {
            if wanted.nullable {
                drafts.push(change(
                    format!("Allow NULL in {table}.{name}"),
                    self.alter(table, name, "DROP NOT NULL"),
                    vec![note(NoteKind::Lock, BRIEF_LOCK)],
                ));
            } else {
                drafts.push(change(
                    format!("Require a value in {table}.{name}"),
                    self.alter(table, name, "SET NOT NULL"),
                    vec![
                        note(
                            NoteKind::MayFail,
                            "Fails if any row has no value in the column.",
                        ),
                        note(
                            NoteKind::Lock,
                            format!(
                                "Reads the whole table{} under an ACCESS EXCLUSIVE lock that \
                                 blocks reads and writes.",
                                about(rows)
                            ),
                        ),
                    ],
                ));
            }
        }

        match (wanted.identity, current.identity) {
            (Some(identity), None) => {
                let mut steps = Vec::new();
                if current.nullable {
                    steps.push(change(
                        format!("Require a value in {table}.{name}"),
                        self.alter(table, name, "SET NOT NULL"),
                        vec![
                            note(
                                NoteKind::MayFail,
                                "Fails if any row has no value in the column.",
                            ),
                            note(
                                NoteKind::Lock,
                                format!(
                                    "Reads the whole table{} under an ACCESS EXCLUSIVE lock.",
                                    about(rows)
                                ),
                            ),
                        ],
                    ));
                }
                steps.push(change(
                    format!("Fill {table}.{name} as an identity"),
                    self.alter(
                        table,
                        name,
                        &format!("ADD GENERATED {} AS IDENTITY", identity_sql(identity)),
                    ),
                    vec![
                        note(
                            NoteKind::MayFail,
                            "The identity starts at 1. If the column has values, restart it past \
                             the highest (ALTER COLUMN ... RESTART WITH), or new rows will clash.",
                        ),
                        note(NoteKind::Lock, BRIEF_LOCK),
                    ],
                ));
                drafts.extend(steps);
            }
            (Some(identity), Some(present)) if identity != present => drafts.push(change(
                format!("Change how {table}.{name} is filled as an identity"),
                self.alter(
                    table,
                    name,
                    &format!("SET GENERATED {}", identity_sql(identity)),
                ),
                vec![note(NoteKind::Lock, BRIEF_LOCK)],
            )),
            _ => {}
        }
    }

    fn drop_constraint(&self, table: &str, constraint: &Constraint) -> Draft {
        let name = constraint.name.clone().unwrap_or_default();
        let (phase, what) = match constraint.kind {
            ConstraintKind::ForeignKey(_) => (Phase::DropForeignKey, "foreign key"),
            ConstraintKind::PrimaryKey(_) => (Phase::DropConstraint, "primary key"),
            ConstraintKind::Unique(_) => (Phase::DropConstraint, "unique constraint"),
            ConstraintKind::Check(_) => (Phase::DropConstraint, "check constraint"),
        };
        let mut notes = vec![note(NoteKind::Lock, BRIEF_LOCK)];
        if matches!(
            constraint.kind,
            ConstraintKind::PrimaryKey(_) | ConstraintKind::Unique(_)
        ) {
            notes.insert(
                0,
                note(
                    NoteKind::MayFail,
                    "Fails if another table's foreign key references it. Its index is dropped \
                     with it.",
                ),
            );
        }
        Draft {
            phase,
            summary: format!("Drop {what} {name} from {table}"),
            action: StepAction::Remove,
            object: table.to_owned(),
            sql: format!(
                "ALTER TABLE {} DROP CONSTRAINT {}",
                self.table(table),
                quote_ident(&name)
            ),
            transactional: true,
            notes,
        }
    }

    /// A constraint added to an existing table, or a foreign key to a new one.
    fn add_constraint(
        &self,
        table: &str,
        constraint: &Constraint,
        rows: Option<u64>,
        new_table: bool,
    ) -> Draft {
        let label = constraint
            .name
            .as_ref()
            .map(|name| format!(" {name}"))
            .unwrap_or_default();
        let (phase, summary, notes) = match &constraint.kind {
            ConstraintKind::ForeignKey(ForeignKeyDefinition { table: other, .. }) => (
                Phase::AddForeignKey,
                format!("Add foreign key{label} from {table} to {other}"),
                if new_table {
                    vec![note(
                        NoteKind::Lock,
                        format!(
                            "Takes a SHARE ROW EXCLUSIVE lock on {other} for a moment, which \
                             blocks writes to it."
                        ),
                    )]
                } else {
                    vec![
                        note(
                            NoteKind::MayFail,
                            "Fails if a row has no matching row in the referenced table.",
                        ),
                        note(
                            NoteKind::Lock,
                            format!(
                                "Checks every row{} under a SHARE ROW EXCLUSIVE lock on both tables, \
                             which blocks writes. Adding it NOT VALID and validating later keeps \
                             the lock short.",
                                about(rows)
                            ),
                        ),
                    ]
                },
            ),
            ConstraintKind::PrimaryKey(_) | ConstraintKind::Unique(_) => (
                Phase::AddConstraint,
                format!(
                    "Add {}{label} to {table}",
                    if matches!(constraint.kind, ConstraintKind::PrimaryKey(_)) {
                        "primary key"
                    } else {
                        "unique constraint"
                    }
                ),
                vec![
                    note(
                        NoteKind::MayFail,
                        "Fails if existing rows repeat a value (or have none, for a primary key).",
                    ),
                    note(
                        NoteKind::Lock,
                        format!(
                            "Builds an index over the table{} under an ACCESS EXCLUSIVE lock that \
                             blocks reads and writes.",
                            about(rows)
                        ),
                    ),
                ],
            ),
            ConstraintKind::Check(_) => (
                Phase::AddConstraint,
                format!("Add check constraint{label} to {table}"),
                vec![
                    note(
                        NoteKind::MayFail,
                        "Fails if an existing row breaks the condition.",
                    ),
                    note(
                        NoteKind::Lock,
                        format!(
                            "Reads every row{} under an ACCESS EXCLUSIVE lock. Adding it NOT \
                             VALID and validating later keeps the lock short.",
                            about(rows)
                        ),
                    ),
                ],
            ),
        };
        Draft {
            phase,
            summary,
            action: StepAction::Add,
            object: table.to_owned(),
            sql: format!(
                "ALTER TABLE {} ADD {}",
                self.table(table),
                self.constraint_sql(constraint)
            ),
            transactional: true,
            notes,
        }
    }

    /// Dropping an index is quick, so it always runs in the transaction (`DROP INDEX
    /// CONCURRENTLY` would also be a statement the policy cannot read, and refuses).
    fn drop_index(&self, table: &str, index: &Index) -> Draft {
        let name = index.name.clone().unwrap_or_default();
        Draft {
            phase: Phase::DropIndex,
            summary: format!("Drop index {name} on {table}"),
            action: StepAction::Remove,
            object: table.to_owned(),
            sql: format!("DROP INDEX {}", qualified(self.to, &name)),
            transactional: true,
            notes: vec![note(NoteKind::Lock, BRIEF_LOCK)],
        }
    }

    /// An index on an existing table, or on a new (empty) one.
    fn create_index(
        &self,
        table: &str,
        index: &Index,
        rows: Option<u64>,
        new_table: bool,
    ) -> Draft {
        let concurrent = self.options.concurrent_indexes;
        let mut notes = Vec::new();
        if index.unique && !new_table {
            notes.push(note(
                NoteKind::MayFail,
                "Fails if existing rows repeat a value.",
            ));
        }
        if concurrent {
            notes.push(note(
                NoteKind::NonTransactional,
                "Runs after the transaction and lets writes continue while it builds; it is not \
                 rolled back if another step fails, and if it fails it leaves an invalid index to \
                 drop.",
            ));
        } else if !new_table {
            notes.push(note(
                NoteKind::Lock,
                format!(
                    "Blocks writes to the table{} while the index builds (SHARE lock). Building it \
                     CONCURRENTLY avoids that, outside the transaction.",
                    about(rows)
                ),
            ));
        }
        Draft {
            phase: Phase::CreateIndex,
            summary: format!(
                "Create {}index{} on {table}",
                if index.unique { "unique " } else { "" },
                index
                    .name
                    .as_ref()
                    .map(|name| format!(" {name}"))
                    .unwrap_or_default()
            ),
            action: StepAction::Add,
            object: table.to_owned(),
            sql: self.index_sql(table, index),
            transactional: !concurrent,
            notes,
        }
    }

    fn views(
        &self,
        source: &SchemaModel,
        target: &SchemaModel,
        drafts: &mut Vec<Draft>,
        summary: &mut DiffSummary,
    ) {
        let existing: HashMap<&str, &View> =
            target.views.iter().map(|v| (v.name.as_str(), v)).collect();
        let wanted: HashMap<&str, &View> =
            source.views.iter().map(|v| (v.name.as_str(), v)).collect();
        let compare = source.stored_text && target.stored_text;
        let changed: BTreeSet<&str> = target
            .views
            .iter()
            .filter(|view| {
                wanted.get(view.name.as_str()).is_some_and(|wanted| {
                    compare && !self.same_expression(&wanted.definition, &view.definition)
                })
            })
            .map(|view| view.name.as_str())
            .collect();
        // Rebuilt: changed views (a new query may change its columns, which CREATE OR REPLACE
        // cannot do), views reading a reshaped column, and views reading a rebuilt view.
        let reshaped = self.reshaped.borrow();
        let mut rebuilt: BTreeSet<&str> = changed.clone();
        loop {
            let before = rebuilt.len();
            for view in &target.views {
                if view.reads.iter().any(|column| reshaped.contains(column))
                    || view
                        .depends_on
                        .iter()
                        .any(|name| rebuilt.contains(name.as_str()))
                {
                    rebuilt.insert(view.name.as_str());
                }
            }
            if rebuilt.len() == before {
                break;
            }
        }
        // Dropped in reverse creation order: later views may read earlier ones.
        for view in target.views.iter().rev() {
            let removed = !wanted.contains_key(view.name.as_str());
            if !removed && !rebuilt.contains(view.name.as_str()) {
                continue;
            }
            if removed {
                summary.views_removed += 1;
            }
            drafts.push(Draft {
                phase: Phase::DropView,
                summary: if removed {
                    format!("Drop view {}", view.name)
                } else {
                    format!("Drop view {} to create it again", view.name)
                },
                action: if removed {
                    StepAction::Remove
                } else {
                    StepAction::Change
                },
                object: view.name.clone(),
                sql: format!("DROP VIEW {}", self.table(&view.name)),
                transactional: true,
                notes: vec![note(
                    NoteKind::MayFail,
                    "Fails if a view not compared here depends on it. Privileges granted on it \
                     and its comment go with it.",
                )],
            });
        }
        for view in &source.views {
            let (summary_text, action) = match existing.get(view.name.as_str()) {
                None => {
                    summary.views_added += 1;
                    (format!("Create view {}", view.name), StepAction::Add)
                }
                Some(_) if changed.contains(view.name.as_str()) => {
                    summary.views_changed += 1;
                    (
                        format!("Create view {} with its new query", view.name),
                        StepAction::Change,
                    )
                }
                Some(_) if rebuilt.contains(view.name.as_str()) => (
                    format!("Create view {} again", view.name),
                    StepAction::Change,
                ),
                Some(_) => continue,
            };
            drafts.push(Draft {
                phase: Phase::CreateView,
                summary: summary_text,
                action,
                object: view.name.clone(),
                sql: format!(
                    "CREATE VIEW {} AS\n{}",
                    self.table(&view.name),
                    self.text(view.definition.trim())
                ),
                transactional: true,
                notes: Vec::new(),
            });
        }
    }
}

fn index_key(index: &Index, own: &str) -> String {
    format!(
        "{}:{}",
        index.unique,
        normalize_expression(&index.definition, own)
    )
}

const fn action_sql(action: ReferentialAction) -> &'static str {
    match action {
        ReferentialAction::NoAction => "NO ACTION",
        ReferentialAction::Restrict => "RESTRICT",
        ReferentialAction::Cascade => "CASCADE",
        ReferentialAction::SetNull => "SET NULL",
        ReferentialAction::SetDefault => "SET DEFAULT",
    }
}

const fn identity_sql(identity: Identity) -> &'static str {
    match identity {
        Identity::Always => "ALWAYS",
        Identity::ByDefault => "BY DEFAULT",
    }
}

/// Whether changing `from` to `to` keeps every value as it is and needs no rewrite: a longer
/// or unlimited `varchar`, or `varchar` to `text`.
fn widens(from: &str, to: &str) -> bool {
    let limit = |data_type: &str| -> Option<Option<u32>> {
        if data_type == "text" || data_type == "character varying" {
            return Some(None);
        }
        data_type
            .strip_prefix("character varying(")
            .and_then(|rest| rest.strip_suffix(')'))
            .and_then(|size| size.parse().ok())
            .map(Some)
    };
    let from_varchar = from.starts_with("character varying");
    match (limit(from), limit(to)) {
        (Some(Some(a)), Some(Some(b))) => b >= a,
        (Some(_), Some(None)) => from_varchar || from == to,
        _ => false,
    }
}

#[cfg(test)]
mod tests;
