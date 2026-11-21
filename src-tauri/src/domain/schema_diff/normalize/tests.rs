use super::*;

#[test]
fn names_are_quoted_only_when_postgresql_would_change_them() {
    assert_eq!(quote_ident("orders"), "orders");
    assert_eq!(quote_ident("order_items2"), "order_items2");
    assert_eq!(quote_ident("Orders"), "\"Orders\"");
    assert_eq!(quote_ident("order"), "\"order\"");
    assert_eq!(quote_ident("2fa"), "\"2fa\"");
    assert_eq!(quote_ident("a\"b"), "\"a\"\"b\"");
    assert_eq!(quote_ident("café"), "\"café\"");
    assert_eq!(qualified("public", "user"), "public.\"user\"");
}

#[test]
fn types_take_the_form_format_type_prints() {
    let cases = [
        ("INT", "integer"),
        ("int4", "integer"),
        ("serial", "integer"),
        ("BIGSERIAL", "bigint"),
        ("varchar(40)", "character varying(40)"),
        ("CHARACTER VARYING(40)", "character varying(40)"),
        ("varchar", "character varying"),
        ("char", "character(1)"),
        ("char(3)", "character(3)"),
        ("decimal(10, 2)", "numeric(10,2)"),
        ("numeric(5)", "numeric(5,0)"),
        ("NUMERIC", "numeric"),
        ("timestamptz", "timestamp with time zone"),
        ("TIMESTAMP", "timestamp without time zone"),
        ("timestamp(3) with time zone", "timestamp(3) with time zone"),
        (
            "TIMESTAMP(3) WITHOUT TIME ZONE",
            "timestamp(3) without time zone",
        ),
        ("time", "time without time zone"),
        ("float8", "double precision"),
        ("float(10)", "real"),
        ("bool", "boolean"),
        ("TEXT[]", "text[]"),
        ("int[][]", "integer[][]"),
        ("integer ARRAY", "integer[]"),
        ("jsonb", "jsonb"),
        ("public.mood", "public.mood"),
        ("\"Mood\"", "\"Mood\""),
    ];
    for (written, canonical) in cases {
        assert_eq!(canonical_type(written), canonical, "{written}");
    }
    assert!(serial_type("BigSerial"));
    assert!(!serial_type("integer"));
    assert_eq!(serial_name("bigint"), Some("bigserial"));
}

#[test]
fn expressions_compare_equal_across_spellings() {
    let same = |a: &str, b: &str| {
        assert_eq!(
            normalize_expression(a, "public"),
            normalize_expression(b, "public"),
            "{a} vs {b}"
        );
    };
    same("(price >= (0)::numeric)", "price >= 0");
    same("'misc'::text", "'misc'");
    same("'x'::character varying", "'x'");
    same("now()", "NOW()");
    same(
        "nextval('public.items_id_seq'::regclass)",
        "nextval('items_id_seq'::regclass)",
    );
    same("public.lower(name)", "lower(name)");
    same(
        "'2026-01-01 00:00:00'::timestamp without time zone",
        "'2026-01-01 00:00:00'",
    );
    same("ARRAY[]::text[]", "ARRAY[]");
    same(
        "USING btree (placed DESC) WHERE ((note IS NOT NULL))",
        "USING btree (placed DESC) WHERE (note IS NOT NULL)",
    );
    same("f((a + b))", "f(a + b)");
    // A call keeps its parentheses; grouping that changes the meaning stays.
    assert_ne!(
        normalize_expression("(a + b) * c", "public"),
        normalize_expression("a + b * c", "public")
    );
    assert_ne!(
        normalize_expression("'Misc'", "public"),
        normalize_expression("'misc'", "public")
    );
    // Another schema's qualifier is kept.
    assert_ne!(
        normalize_expression("audit.stamp()", "public"),
        normalize_expression("stamp()", "public")
    );
}

#[test]
fn names_move_from_one_schema_to_another() {
    assert_eq!(
        retarget(
            "nextval('staging.items_id_seq'::regclass)",
            "staging",
            "public"
        ),
        "nextval('public.items_id_seq'::regclass)"
    );
    assert_eq!(
        retarget("staging.f(x) + other.g(y)", "staging", "Prod"),
        "\"Prod\".f(x) + other.g(y)"
    );
    assert_eq!(retarget("price > 0", "a", "a"), "price > 0");
    assert_eq!(unqualify("public.mood", "public"), "mood");
    assert_eq!(unqualify("audit.mood", "public"), "audit.mood");
    // Strings that merely contain the schema name are left alone.
    assert_eq!(
        retarget("'staging.note'", "staging", "public"),
        "'staging.note'"
    );
}

#[test]
fn defaults_say_which_rewrite_and_which_sequence() {
    assert_eq!(
        nextval_sequence("nextval('public.items_id_seq'::regclass)").as_deref(),
        Some("public.items_id_seq")
    );
    assert_eq!(nextval_sequence("now()"), None);
    assert!(volatile_default("gen_random_uuid()"));
    assert!(volatile_default("NEXTVAL('s'::regclass)"));
    assert!(!volatile_default("now()"));
    assert!(!volatile_default("'random'::text"));
}
