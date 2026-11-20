use super::*;
use crate::domain::schema_diff::parse::read;

fn column(name: &str, data_type: &str) -> Column {
    Column {
        name: name.into(),
        data_type: data_type.into(),
        nullable: true,
        default: None,
        identity: None,
        generated: None,
        serial: false,
    }
}

fn required(name: &str, data_type: &str) -> Column {
    Column {
        nullable: false,
        ..column(name, data_type)
    }
}

fn key(name: &str, columns: &[&str]) -> Constraint {
    Constraint {
        name: Some(name.into()),
        kind: ConstraintKind::PrimaryKey(columns.iter().map(|c| (*c).to_owned()).collect()),
    }
}

/// The database's side: names qualified with its schema, as read with an empty search path.
fn current() -> SchemaModel {
    SchemaModel {
        schema: "public".into(),
        tables: vec![
            Table {
                name: "customers".into(),
                columns: vec![
                    Column {
                        default: Some("nextval('public.customers_id_seq'::regclass)".into()),
                        serial: true,
                        ..required("id", "integer")
                    },
                    column("name", "character varying(40)"),
                    column("legacy", "text"),
                    Column {
                        default: Some("'ok'::public.mood".into()),
                        ..column("mood", "public.mood")
                    },
                ],
                constraints: vec![
                    key("customers_pkey", &["id"]),
                    Constraint {
                        name: Some("customers_name_check".into()),
                        kind: ConstraintKind::Check("(length((name)::text) > 0)".into()),
                    },
                ],
                indexes: vec![Index {
                    name: Some("customers_legacy_idx".into()),
                    unique: false,
                    definition: "USING btree (legacy)".into(),
                }],
                rows: Some(12_000),
            },
            Table {
                name: "old_log".into(),
                columns: vec![column("line", "text")],
                rows: Some(3),
                ..Table::default()
            },
        ],
        views: vec![View {
            name: "names".into(),
            definition: " SELECT customers.name\n   FROM public.customers".into(),
            depends_on: vec!["customers".into()],
            reads: vec!["customers.name".into()],
        }],
        not_compared: Vec::new(),
        stored_text: true,
    }
}

const WANTED: &str = r"
    CREATE TABLE customers (
        id serial PRIMARY KEY,
        name varchar(80) NOT NULL CHECK (length(name::text) > 0),
        mood mood DEFAULT 'ok',
        email text
    );
    CREATE TABLE orders (
        id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
        customer_id int NOT NULL REFERENCES customers (id),
        placed_at timestamptz NOT NULL DEFAULT now()
    );
    CREATE INDEX orders_customer_idx ON orders (customer_id);
";

fn sqls(plan: &Plan) -> Vec<&str> {
    plan.steps.iter().map(|step| step.sql.as_str()).collect()
}

#[test]
fn a_schema_that_matches_needs_nothing() {
    let model = current();
    let plan = plan(
        &model,
        &model,
        GenerateOptions::default(),
        Environment::Development,
    );
    assert!(plan.steps.is_empty());
    assert!(plan.script.is_empty());
    assert_eq!(plan.risk, RiskLevel::R0);
    assert_eq!(plan.summary, DiffSummary::default());
}

#[test]
fn a_file_becomes_ordered_steps_with_their_risks() {
    let wanted = read(WANTED, "public").expect("reads");
    let plan = plan(
        &wanted,
        &current(),
        GenerateOptions::default(),
        Environment::Production,
    );
    assert_eq!(
        sqls(&plan),
        [
            "DROP VIEW public.names",
            "DROP INDEX public.customers_legacy_idx",
            "DROP TABLE public.old_log",
            "CREATE TABLE public.orders (\n    id bigint GENERATED ALWAYS AS IDENTITY,\n    \
             customer_id integer NOT NULL,\n    placed_at timestamp with time zone DEFAULT now() \
             NOT NULL,\n    PRIMARY KEY (id)\n)",
            "ALTER TABLE public.customers ALTER COLUMN name TYPE character varying(80) USING \
             name::character varying(80)",
            "ALTER TABLE public.customers ALTER COLUMN name SET NOT NULL",
            "ALTER TABLE public.customers ADD COLUMN email text",
            "ALTER TABLE public.customers DROP COLUMN legacy",
            "CREATE INDEX orders_customer_idx ON public.orders USING btree (customer_id)",
            "ALTER TABLE public.orders ADD FOREIGN KEY (customer_id) REFERENCES public.customers \
             (id)",
        ],
        "the serial key, check and enum default match; the file has no view, so it goes \
         first"
    );
    assert_eq!(
        plan.summary,
        DiffSummary {
            tables_added: 1,
            tables_removed: 1,
            tables_changed: 1,
            views_removed: 1,
            ..DiffSummary::default()
        }
    );

    let step = |sql: &str| {
        plan.steps
            .iter()
            .find(|step| step.sql.starts_with(sql))
            .unwrap_or_else(|| panic!("{sql}"))
    };
    let kinds = |sql: &str| step(sql).notes.iter().map(|n| n.kind).collect::<Vec<_>>();
    assert_eq!(kinds("DROP TABLE"), [NoteKind::DataLoss, NoteKind::MayFail]);
    assert!(step("DROP TABLE").notes[0].text.contains("about 3 rows"));
    // varchar(40) to varchar(80) keeps every value and needs no rewrite.
    assert_eq!(
        kinds("ALTER TABLE public.customers ALTER COLUMN name TYPE"),
        [NoteKind::MayFail, NoteKind::Lock]
    );
    assert!(
        step("ALTER TABLE public.customers ALTER COLUMN name SET NOT NULL").notes[1]
            .text
            .contains("about 12,000 rows")
    );
    assert_eq!(
        kinds("ALTER TABLE public.customers DROP COLUMN"),
        [NoteKind::DataLoss, NoteKind::MayFail, NoteKind::Lock]
    );
    assert_eq!(
        step("ALTER TABLE public.customers DROP COLUMN").action,
        StepAction::Remove
    );
    // New tables are empty: creating them and their index risks nothing.
    assert!(step("CREATE TABLE").notes.is_empty());
    assert!(step("CREATE INDEX").notes.is_empty());

    assert_eq!(step("DROP TABLE").risk, RiskLevel::R4);
    assert!(step("CREATE TABLE").risk >= RiskLevel::R3, "production");
    assert_eq!(plan.risk, RiskLevel::R4);

    assert!(
        plan.script
            .starts_with("BEGIN;\nSET LOCAL search_path TO public;\n")
    );
    assert!(
        plan.script
            .contains("-- Drop table old_log\nDROP TABLE public.old_log;\n")
    );
    assert!(plan.script.trim_end().ends_with("COMMIT;"));
}

#[test]
fn constraints_and_indexes_match_by_what_they_are() {
    let mut renamed = current();
    renamed.tables[0].constraints[0].name = Some("pk_customers".into());
    renamed.tables[0].indexes[0].name = Some("legacy_lookup".into());
    let plan = plan(
        &renamed,
        &current(),
        GenerateOptions::default(),
        Environment::Local,
    );
    assert!(plan.steps.is_empty(), "{:?}", sqls(&plan));

    let mut changed = current();
    changed.tables[0].constraints[1].kind = ConstraintKind::Check("length(name) > 1".into());
    let plan = super::plan(
        &changed,
        &current(),
        GenerateOptions::default(),
        Environment::Local,
    );
    assert_eq!(
        sqls(&plan),
        [
            "ALTER TABLE public.customers DROP CONSTRAINT customers_name_check",
            "ALTER TABLE public.customers ADD CONSTRAINT customers_name_check CHECK (length(name) \
             > 1)",
        ]
    );
}

#[test]
fn another_schema_is_written_for_the_target() {
    let mut staging = current();
    staging.schema = "staging".into();
    for table in &mut staging.tables {
        for column in &mut table.columns {
            if let Some(default) = &mut column.default {
                *default = default.replace("public.", "staging.");
            }
            column.data_type = column.data_type.replace("public.", "staging.");
        }
    }
    staging.views[0].definition = " SELECT customers.name\n   FROM staging.customers".into();
    // The same schema in two places: nothing to do.
    let same = plan(
        &staging,
        &current(),
        GenerateOptions::default(),
        Environment::Local,
    );
    assert!(same.steps.is_empty(), "{:?}", sqls(&same));

    staging.tables.push(Table {
        name: "Notes".into(),
        columns: vec![Column {
            default: Some("staging.stamp()".into()),
            ..column("at", "staging.moment")
        }],
        ..Table::default()
    });
    staging.views.push(View {
        name: "recent".into(),
        definition: "SELECT * FROM staging.\"Notes\"".into(),
        depends_on: vec!["Notes".into()],
        reads: vec!["Notes.at".into()],
    });
    let plan = plan(
        &staging,
        &current(),
        GenerateOptions::default(),
        Environment::Local,
    );
    assert_eq!(
        sqls(&plan),
        [
            "CREATE TABLE public.\"Notes\" (\n    at public.moment DEFAULT public.stamp()\n)",
            "CREATE VIEW public.recent AS\nSELECT * FROM public.\"Notes\"",
        ]
    );
}

#[test]
fn columns_change_in_an_order_postgresql_accepts() {
    let mut wanted = current();
    let mut target = current();
    // Plain integer becomes an identity: NOT NULL first, then the identity.
    target.tables[1].columns = vec![column("line", "integer")];
    wanted.tables[1].columns = vec![Column {
        identity: Some(Identity::ByDefault),
        nullable: false,
        ..column("line", "integer")
    }];
    // A serial column loses its sequence and becomes a plain bigint with a constant default.
    wanted.tables[0].columns[0] = Column {
        default: Some("0".into()),
        ..required("id", "bigint")
    };
    let plan = plan(
        &wanted,
        &target,
        GenerateOptions::default(),
        Environment::Local,
    );
    assert_eq!(
        sqls(&plan),
        [
            "ALTER TABLE public.customers ALTER COLUMN id TYPE bigint USING id::bigint",
            "ALTER TABLE public.customers ALTER COLUMN id SET DEFAULT 0",
            "ALTER TABLE public.old_log ALTER COLUMN line SET NOT NULL",
            "ALTER TABLE public.old_log ALTER COLUMN line ADD GENERATED BY DEFAULT AS IDENTITY",
        ]
    );
    let retype = &plan.steps[0];
    assert_eq!(retype.notes[0].kind, NoteKind::DataLoss);
    assert!(
        retype.notes[2]
            .text
            .contains("Rewrites the table (about 12,000 rows)")
    );
}

#[test]
fn new_columns_say_when_they_fail_or_rewrite() {
    let mut wanted = current();
    wanted.tables[0].columns.push(required("code", "text"));
    wanted.tables[0].columns.push(Column {
        default: Some("gen_random_uuid()".into()),
        ..required("token", "uuid")
    });
    wanted.tables[0].columns.push(Column {
        default: Some("'x'::text".into()),
        ..required("kind", "text")
    });
    let plan = plan(
        &wanted,
        &current(),
        GenerateOptions::default(),
        Environment::Local,
    );
    let notes = |index: usize| -> Vec<NoteKind> {
        plan.steps[index].notes.iter().map(|n| n.kind).collect()
    };
    assert_eq!(
        plan.steps[0].sql,
        "ALTER TABLE public.customers ADD COLUMN code text NOT NULL"
    );
    assert_eq!(notes(0), [NoteKind::MayFail, NoteKind::Lock]);
    assert!(plan.steps[1].notes[0].text.starts_with("Rewrites"));
    assert_eq!(notes(2), [NoteKind::Lock]);
    assert!(plan.steps[2].notes[0].text.contains("not rewritten"));
}

#[test]
fn concurrent_indexes_run_after_the_transaction() {
    let mut wanted = current();
    wanted.tables[0].indexes[0] = Index {
        name: Some("customers_name_idx".into()),
        unique: true,
        definition: "USING btree (name)".into(),
    };
    let options = GenerateOptions {
        concurrent_indexes: true,
    };
    let plan = plan(&wanted, &current(), options, Environment::Local);
    assert_eq!(
        sqls(&plan),
        [
            "DROP INDEX CONCURRENTLY public.customers_legacy_idx",
            "CREATE UNIQUE INDEX CONCURRENTLY customers_name_idx ON public.customers USING btree \
             (name)",
        ]
    );
    assert!(plan.steps.iter().all(|step| !step.transactional));
    assert!(
        plan.steps[1]
            .notes
            .iter()
            .any(|n| n.kind == NoteKind::NonTransactional)
    );
    let commit = plan.script.find("COMMIT;").expect("commit");
    let create = plan.script.find("CREATE UNIQUE INDEX").expect("create");
    assert!(create > commit, "{}", plan.script);
    assert!(plan.script.contains("SET search_path TO public;"));
}

#[test]
fn views_are_dropped_and_created_again_when_their_query_changes() {
    let mut wanted = current();
    wanted.views[0].definition =
        " SELECT upper(customers.name) AS name\n   FROM public.customers".into();
    wanted.views.insert(
        0,
        View {
            name: "first".into(),
            definition: "SELECT 1".into(),
            depends_on: Vec::new(),
            reads: Vec::new(),
        },
    );
    let mut target = current();
    target.views.push(View {
        name: "gone".into(),
        definition: "SELECT 2".into(),
        depends_on: Vec::new(),
        reads: Vec::new(),
    });
    let plan = plan(
        &wanted,
        &target,
        GenerateOptions::default(),
        Environment::Local,
    );
    assert_eq!(
        sqls(&plan),
        [
            "DROP VIEW public.gone",
            "DROP VIEW public.names",
            "CREATE VIEW public.first AS\nSELECT 1",
            "CREATE VIEW public.names AS\nSELECT upper(customers.name) AS name\n   FROM \
             public.customers",
        ]
    );
    assert_eq!(
        plan.summary,
        DiffSummary {
            views_added: 1,
            views_removed: 1,
            views_changed: 1,
            ..DiffSummary::default()
        }
    );
}

#[test]
fn widening_varchar_keeps_values() {
    assert!(widens("character varying(10)", "character varying(20)"));
    assert!(widens("character varying(10)", "text"));
    assert!(widens("character varying(10)", "character varying"));
    assert!(!widens("character varying(20)", "character varying(10)"));
    assert!(!widens("text", "character varying(10)"));
    assert!(!widens("integer", "bigint"));
}

#[test]
fn views_reading_a_reshaped_table_are_rebuilt_around_the_change() {
    let mut wanted = current();
    wanted.tables[0].columns[1].data_type = "text".into();
    let mut target = current();
    // A view on the view: rebuilt too, after the one it reads.
    target.views.push(View {
        name: "short_names".into(),
        definition: " SELECT names.name\n   FROM public.names".into(),
        depends_on: vec!["names".into()],
        reads: vec!["names.name".into()],
    });
    wanted.views.push(target.views[1].clone());
    let plan = plan(
        &wanted,
        &target,
        GenerateOptions::default(),
        Environment::Local,
    );
    assert_eq!(
        sqls(&plan),
        [
            "DROP VIEW public.short_names",
            "DROP VIEW public.names",
            "ALTER TABLE public.customers ALTER COLUMN name TYPE text USING name::text",
            "CREATE VIEW public.names AS\nSELECT customers.name\n   FROM public.customers",
            "CREATE VIEW public.short_names AS\nSELECT names.name\n   FROM public.names",
        ]
    );
    assert_eq!(plan.summary.views_changed, 0, "rebuilt, not changed");
    assert_eq!(plan.steps[3].summary, "Create view names again");
}

#[test]
fn a_files_checks_fall_back_to_their_names() {
    // PostgreSQL stored the file's `length(name) > 0` its own way.
    let file = read(
        "CREATE TABLE customers (id serial PRIMARY KEY, name varchar(40) CHECK (name <> ''), \
         mood public.mood DEFAULT 'ok', legacy text);",
        "public",
    )
    .expect("reads");
    let mut target = current();
    target.tables.truncate(1);
    target.views.clear();
    target.tables[0].indexes.clear();
    let plan = plan(
        &file,
        &target,
        GenerateOptions::default(),
        Environment::Local,
    );
    assert!(plan.steps.is_empty(), "{:?}", sqls(&plan));
    assert!(
        plan.notes[0].contains("customers.customers_name_check"),
        "{:?}",
        plan.notes
    );
}
