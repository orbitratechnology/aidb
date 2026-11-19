use sqlx::postgres::types::Oid;

use super::*;
use crate::adapters::postgres::table_data::RelationRow;
use crate::domain::data_change::ColumnValue;
use crate::domain::table_data::{Filter, FilterOperator};

fn column(name: &str, quoted: &str) -> ColumnRow {
    ColumnRow {
        name: name.into(),
        quoted: quoted.into(),
        data_type: "integer".into(),
        plain_type: "integer".into(),
        has_modifier: false,
        base_type: "pg_catalog.int4".into(),
        category: "N".into(),
        type_name: "int4".into(),
        readable: true,
        can_insert: true,
        can_update: true,
        generated: false,
        always_identity: false,
        key_position: None,
    }
}

fn varchar(name: &str, length: u32) -> ColumnRow {
    ColumnRow {
        data_type: format!("character varying({length})"),
        plain_type: "character varying".into(),
        has_modifier: true,
        base_type: "pg_catalog.\"varchar\"".into(),
        category: "S".into(),
        type_name: "varchar".into(),
        ..column(name, name)
    }
}

fn target(columns: Vec<ColumnRow>) -> Target {
    Target {
        relation: RelationRow {
            oid: Oid(1),
            quoted: r#"public."Order Items""#.into(),
            usable: true,
            encoding: "UTF8".into(),
            table: true,
        },
        columns,
    }
}

fn table() -> ObjectName {
    ObjectName {
        schema: "public".into(),
        name: "Order Items".into(),
    }
}

fn equals(column: &str, value: &str) -> Filter {
    Filter {
        column: column.into(),
        operator: FilterOperator::Equals,
        value: Some(value.into()),
    }
}

fn columns() -> Vec<ColumnRow> {
    vec![
        column("id", "id"),
        varchar("Note \"x\"", 10),
        column("qty", "qty"),
    ]
}

#[test]
fn updates_bind_values_then_filter_values_with_server_quoted_names() {
    let change = DataChange::Update {
        table: table(),
        set: vec![
            ColumnValue {
                column: "Note \"x\"".into(),
                value: Some("packed".into()),
            },
            ColumnValue {
                column: "qty".into(),
                value: None,
            },
        ],
        filters: vec![
            equals("id", "7"),
            Filter {
                column: "Note \"x\"".into(),
                operator: FilterOperator::Contains,
                value: Some("50%".into()),
            },
        ],
    };
    // The test helper quotes the odd name as the server would.
    let mut columns = columns();
    columns[1].quoted = r#""Note ""x""""#.into();
    let built = build(&change, &target(columns)).expect("builds");
    assert_eq!(
        built.statement,
        r#"UPDATE public."Order Items" SET "Note ""x""" = CAST($1 AS character varying), qty = CAST($2 AS integer) WHERE id = CAST($3 AS pg_catalog.int4) AND ("Note ""x""")::text ILIKE $4 RETURNING ("Note ""x""")::text, (qty)::text"#
    );
    assert_eq!(
        built.parameters,
        vec![
            Some("packed".into()),
            None,
            Some("7".into()),
            Some("%50\\%%".into())
        ]
    );
    let (count, values) = built.count.expect("counts");
    assert_eq!(
        count,
        r#"SELECT count(*) FROM (SELECT 1 FROM public."Order Items" WHERE id = CAST($1 AS pg_catalog.int4) AND ("Note ""x""")::text ILIKE $2 LIMIT 10001"#
    );
    assert_eq!(values, vec!["7".to_owned(), "%50\\%%".to_owned()]);

    let normalize = built.normalize.expect("normalizes");
    assert_eq!(
        normalize.sql,
        "SELECT (CAST(v.c0 AS character varying(10)))::text, (CAST(v.c1 AS integer))::text, \
         (v.c0 IS NULL OR rtrim(CAST(v.c0 AS character varying)::text) = \
         rtrim(CAST(v.c0 AS character varying(10))::text))::text \
         FROM (VALUES (1, $1::text, $2::text)) AS v(n, c0, c1) ORDER BY v.n"
    );
    assert_eq!(normalize.parameters, vec![Some("packed".into()), None]);
    assert_eq!(normalize.checked, vec![0]);
    assert_eq!(built.value_columns, vec!["Note \"x\"", "qty"]);
}

#[test]
fn inserts_cast_every_value_and_return_what_was_stored() {
    let change = DataChange::Insert {
        table: table(),
        columns: vec!["id".into(), "qty".into()],
        rows: vec![
            vec![Some("1".into()), Some("2".into())],
            vec![Some("3".into()), None],
        ],
    };
    let built = build(&change, &target(columns())).expect("builds");
    assert_eq!(
        built.statement,
        r#"INSERT INTO public."Order Items" (id, qty) VALUES (CAST($1 AS integer), CAST($2 AS integer)), (CAST($3 AS integer), CAST($4 AS integer)) RETURNING (id)::text, (qty)::text"#
    );
    assert_eq!(built.parameters.len(), 4);
    assert_eq!(built.insert_rows, 2);
    assert!(built.count.is_none());
    let normalize = built.normalize.expect("normalizes");
    assert!(
        normalize
            .sql
            .contains("(VALUES (1, $1::text, $2::text), (2, $3::text, $4::text))")
    );
    assert!(normalize.checked.is_empty());
}

#[test]
fn deletes_need_only_the_filter() {
    let change = DataChange::Delete {
        table: table(),
        filters: vec![Filter {
            column: "qty".into(),
            operator: FilterOperator::IsNull,
            value: None,
        }],
    };
    let built = build(&change, &target(columns())).expect("builds");
    assert_eq!(
        built.statement,
        r#"DELETE FROM public."Order Items" WHERE qty IS NULL"#
    );
    assert!(built.parameters.is_empty());
    assert!(built.normalize.is_none());
}

#[test]
fn columns_the_database_fills_or_the_role_cannot_write_are_refused() {
    let update = |name: &str| DataChange::Update {
        table: table(),
        set: vec![ColumnValue {
            column: name.into(),
            value: Some("1".into()),
        }],
        filters: vec![equals("id", "1")],
    };
    let mut generated = column("total", "total");
    generated.generated = true;
    let mut identity = column("serial", "serial");
    identity.always_identity = true;
    let mut locked = column("price", "price");
    locked.can_update = false;
    let mut hidden = column("secret", "secret");
    hidden.readable = false;
    let target = target(vec![
        column("id", "id"),
        generated,
        identity,
        locked,
        hidden,
    ]);
    let cases = [
        ("total", DbErrorKind::InvalidRequest, "generated column"),
        ("serial", DbErrorKind::InvalidRequest, "identity column"),
        ("price", DbErrorKind::PermissionDenied, "cannot update"),
        ("secret", DbErrorKind::PermissionDenied, "cannot read"),
        ("missing", DbErrorKind::InvalidRequest, "has no column"),
    ];
    for (name, kind, text) in cases {
        let error = build(&update(name), &target).expect_err(name);
        assert_eq!(error.kind(), kind, "{name}");
        assert!(
            error.message().contains(text),
            "{name}: {}",
            error.message()
        );
    }
}

#[test]
fn views_are_not_written() {
    let mut view = target(columns());
    view.relation.table = false;
    let change = DataChange::Delete {
        table: table(),
        filters: vec![equals("id", "1")],
    };
    let error = build(&change, &view).expect_err("a view");
    assert!(error.message().contains("not a table"));
}

#[test]
fn only_string_and_bit_modifiers_are_checked() {
    let mut numeric = column("price", "price");
    numeric.data_type = "numeric(10,2)".into();
    numeric.plain_type = "numeric".into();
    numeric.has_modifier = true;
    assert_eq!(modifier_check(&numeric, "v.c0"), None);
    let mut bits = column("flags", "flags");
    bits.data_type = "bit(3)".into();
    bits.plain_type = "\"bit\"".into();
    bits.has_modifier = true;
    bits.category = "V".into();
    assert_eq!(
        modifier_check(&bits, "v.c0").as_deref(),
        Some("(v.c0 IS NULL OR CAST(v.c0 AS \"bit\")::text = CAST(v.c0 AS bit(3))::text)::text")
    );
    assert_eq!(modifier_check(&column("id", "id"), "v.c0"), None);
}
