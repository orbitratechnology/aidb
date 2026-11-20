use serde_json::json;

use super::*;

fn call(name: &str, input: Value) -> ToolCall {
    ToolCall {
        id: "c1".into(),
        name: name.into(),
        input: ToolInput::Json(input),
    }
}

#[test]
fn the_registry_holds_read_tools_and_one_that_prepares_changes() {
    let names: Vec<&str> = TOOLS.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "list_schemas",
            "list_tables",
            "describe_table",
            "run_read_query",
            "explain_query",
            "preview_data_change"
        ]
    );
    for tool in TOOLS.iter() {
        assert!(tool.risk <= RiskLevel::R1, "{}", tool.name);
        assert_eq!(tool.input_schema["type"], "object", "{}", tool.name);
        assert_eq!(
            tool.input_schema["additionalProperties"], false,
            "{}",
            tool.name
        );
        assert_eq!(ToolName::parse(tool.name.as_str()), Some(tool.name));
        // Every required property is declared.
        for required in tool.input_schema["required"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let field = required.as_str().expect("name");
            assert!(
                tool.input_schema["properties"].get(field).is_some(),
                "{field}"
            );
        }
    }
    assert_eq!(specs(true).len(), TOOLS.len());
    // Read-only connections are not offered the tool that prepares changes.
    let read_only: Vec<String> = specs(false).into_iter().map(|spec| spec.name).collect();
    assert_eq!(read_only.len(), TOOLS.len() - 1);
    assert!(!read_only.iter().any(|name| name == "preview_data_change"));
    assert!(definition(ToolName::RunReadQuery).returns_data);
    assert!(!definition(ToolName::DescribeTable).returns_data);
}

#[test]
fn schema_properties_match_the_rust_inputs() {
    // A call using every declared property parses; the schema and the types cannot drift apart.
    let full = [
        ("list_schemas", json!({})),
        ("list_tables", json!({"schema": "public", "search": "ord"})),
        (
            "describe_table",
            json!({"schema": "public", "name": "orders"}),
        ),
        (
            "run_read_query",
            json!({"sql": "SELECT 1", "purpose": "Check"}),
        ),
        ("explain_query", json!({"sql": "SELECT 1", "analyze": true})),
        (
            "preview_data_change",
            json!({"purpose": "Ship order 7", "changes": [update_change()]}),
        ),
    ];
    for (name, input) in full {
        let tool = definition(ToolName::parse(name).expect("known"));
        let declared: Vec<&String> = tool.input_schema["properties"]
            .as_object()
            .expect("properties")
            .keys()
            .collect();
        let used: Vec<&String> = input.as_object().expect("object").keys().collect();
        assert_eq!(declared.len(), used.len(), "{name}");
        assert!(parse(&call(name, input)).is_ok(), "{name}");
    }
}

fn update_change() -> Value {
    json!({
        "kind": "update", "schema": "public", "table": "orders",
        "set": [{"column": "status", "value": "shipped"}],
        "filters": [{"column": "id", "operator": "equals", "value": "7"}],
    })
}

#[test]
fn the_change_schema_declares_every_field_of_a_change() {
    let schema = &definition(ToolName::PreviewDataChange).input_schema;
    let item = &schema["properties"]["changes"]["items"];
    let mut fields: Vec<&String> = item["properties"]
        .as_object()
        .expect("object")
        .keys()
        .collect();
    fields.sort();
    assert_eq!(
        fields,
        [
            "columns", "filters", "kind", "rows", "schema", "set", "table"
        ]
    );
    assert_eq!(item["additionalProperties"], false);
    assert_eq!(
        schema["properties"]["changes"]["maxItems"],
        MAX_PROPOSED_CHANGES
    );
}

#[test]
fn proposals_become_checked_change_sets() {
    let proposal = parse(&call(
        "preview_data_change",
        json!({"purpose": " Ship order 7 ", "changes": [
            update_change(),
            {"kind": "insert", "schema": "public", "table": "notes",
             "columns": ["body"], "rows": [["Shipped"], [null]]},
            {"kind": "delete", "schema": "public", "table": "carts",
             "filters": [{"column": "order_id", "operator": "equals", "value": "7"}]},
        ]}),
    ))
    .expect("proposal");
    assert_eq!(proposal.risk(), RiskLevel::R1, "it writes nothing itself");
    assert_eq!(
        proposal.summary(),
        "Prepare a change for your review: Ship order 7"
    );
    let ToolInvocation::PreviewDataChange(proposal) = proposal else {
        panic!("not a proposal");
    };
    assert_eq!(proposal.changes.len(), 3);
    assert_eq!(
        proposal.changes[0],
        DataChange::Update {
            table: ObjectName {
                schema: "public".into(),
                name: "orders".into(),
            },
            set: vec![ColumnValue {
                column: "status".into(),
                value: Some("shipped".into()),
            }],
            filters: vec![Filter {
                column: "id".into(),
                operator: FilterOperator::Equals,
                value: Some("7".into()),
            }],
        }
    );
}

#[test]
fn proposals_are_held_to_the_grid_rules() {
    let propose = |change: Value| {
        parse(&call(
            "preview_data_change",
            json!({"purpose": "Change", "changes": [change]}),
        ))
    };
    // An update or delete without filters would change every row.
    let unfiltered = propose(json!({"kind": "delete", "schema": "public", "table": "orders"}));
    assert!(
        matches!(&unfiltered, Err(ToolCallError::NotAllowed(m)) if m.contains("at least one filter")),
        "{unfiltered:?}"
    );
    let empty = propose(
        json!({"kind": "update", "schema": "public", "table": "orders",
        "set": [{"column": "status", "value": "x"}], "filters": []}),
    );
    assert!(matches!(empty, Err(ToolCallError::NotAllowed(_))));
    // Fields that do not belong to the kind, unknown fields and kinds.
    for bad in [
        json!({"kind": "insert", "schema": "public", "table": "t", "columns": ["a"],
               "rows": [["1"]], "set": []}),
        json!({"kind": "delete", "schema": "public", "table": "t", "set": [],
               "filters": [{"column": "id", "operator": "equals", "value": "1"}]}),
        json!({"kind": "update", "schema": "public", "table": "t"}),
        json!({"kind": "truncate", "schema": "public", "table": "t"}),
        json!({"kind": "delete", "schema": "public", "table": "t", "sql": "DELETE FROM t",
               "filters": [{"column": "id", "operator": "equals", "value": "1"}]}),
    ] {
        assert!(
            matches!(
                propose(bad.clone()),
                Err(ToolCallError::InvalidArguments(_))
            ),
            "{bad}"
        );
    }
    let many = parse(&call(
        "preview_data_change",
        json!({"purpose": "Many", "changes": vec![update_change(); MAX_PROPOSED_CHANGES + 1]}),
    ));
    assert!(matches!(many, Err(ToolCallError::InvalidArguments(_))));
    let no_purpose = parse(&call(
        "preview_data_change",
        json!({"purpose": " ", "changes": [update_change()]}),
    ));
    assert!(matches!(
        no_purpose,
        Err(ToolCallError::InvalidArguments(_))
    ));
}

#[test]
fn calls_parse_into_typed_invocations() {
    assert_eq!(
        parse(&call(
            "describe_table",
            json!({"schema": " public ", "name": "orders"})
        )),
        Ok(ToolInvocation::DescribeTable(ObjectName {
            schema: "public".into(),
            name: "orders".into(),
        }))
    );
    let query = parse(&call(
        "run_read_query",
        json!({"sql": "  SELECT id FROM orders LIMIT 5;  ", "purpose": " Latest orders "}),
    ))
    .expect("read");
    assert_eq!(
        query,
        ToolInvocation::RunReadQuery(ReadQueryInput {
            sql: "SELECT id FROM orders LIMIT 5".into(),
            purpose: "Latest orders".into(),
        })
    );
    assert_eq!(query.risk(), RiskLevel::R1);
    assert_eq!(query.summary(), "Run a read query: Latest orders");
    assert_eq!(
        parse(&call("list_tables", json!({"search": "  "}))),
        Ok(ToolInvocation::ListTables(ListTablesInput::default()))
    );
    assert_eq!(ToolInvocation::ListSchemas.risk(), RiskLevel::R0);
}

#[test]
fn bad_calls_are_rejected_before_anything_runs() {
    let cases = [
        (call("drop_table", json!({})), "unknown"),
        (call("run_sql", json!({"sql": "SELECT 1"})), "unknown"),
        (
            ToolCall {
                id: "c".into(),
                name: "list_tables".into(),
                input: ToolInput::Malformed("{\"schema\":".into()),
            },
            "malformed",
        ),
        (call("list_schemas", json!({"extra": true})), "invalid"),
        (
            call("describe_table", json!({"schema": "public"})),
            "invalid",
        ),
        (
            call("describe_table", json!({"schema": "public", "name": 7})),
            "invalid",
        ),
        (
            call(
                "describe_table",
                json!({"schema": "public", "name": "x".repeat(64)}),
            ),
            "invalid",
        ),
        (
            call("list_tables", json!({"schema": "pub\nlic"})),
            "invalid",
        ),
        (
            call("run_read_query", json!({"sql": "SELECT 1"})),
            "invalid",
        ),
        (
            call("run_read_query", json!({"sql": "SELECT 1", "purpose": " "})),
            "invalid",
        ),
        (call("explain_query", json!({"sql": ""})), "invalid"),
    ];
    for (call, expected) in cases {
        let error = parse(&call).expect_err(expected);
        let kind = match error {
            ToolCallError::UnknownTool(_) => "unknown",
            ToolCallError::MalformedArguments => "malformed",
            ToolCallError::InvalidArguments(_) => "invalid",
            ToolCallError::NotAllowed(_) => "not allowed",
        };
        assert_eq!(kind, expected, "{call:?}");
    }
}

#[test]
fn only_a_single_read_statement_is_accepted() {
    for sql in [
        "DELETE FROM orders",
        "UPDATE orders SET total = 0",
        "DROP TABLE orders",
        "SELECT 1; DROP TABLE orders",
        "SELECT 1; SELECT 2",
        "WITH gone AS (DELETE FROM orders RETURNING id) SELECT * FROM gone",
        "SELECT pg_terminate_backend(42)",
        "SET statement_timeout = 0",
        "COMMIT",
        "SELECT * FROM orders FOR UPDATE",
        "SELECT 'unterminated",
    ] {
        let error =
            parse(&call("run_read_query", json!({"sql": sql, "purpose": "x"}))).expect_err(sql);
        assert!(
            matches!(error, ToolCallError::NotAllowed(_)),
            "{sql}: {error:?}"
        );
    }
    // EXPLAIN is a read for run_read_query, but explain_query adds it itself.
    assert!(
        parse(&call(
            "run_read_query",
            json!({"sql": "EXPLAIN SELECT 1", "purpose": "plan"})
        ))
        .is_ok()
    );
    assert!(matches!(
        parse(&call("explain_query", json!({"sql": "EXPLAIN SELECT 1"}))),
        Err(ToolCallError::NotAllowed(_))
    ));
    let explain = parse(&call("explain_query", json!({"sql": "SELECT 1"}))).expect("explain");
    assert_eq!(
        explain,
        ToolInvocation::ExplainQuery(ExplainInput {
            sql: "SELECT 1".into(),
            analyze: false,
        })
    );
}

#[test]
fn error_messages_tell_the_model_what_to_fix() {
    let error = parse(&call(
        "run_read_query",
        json!({"sql": "DELETE FROM t", "purpose": "x"}),
    ))
    .expect_err("write");
    assert!(error.to_string().contains("changes data"), "{error}");
    assert!(
        parse(&call("nope", json!({})))
            .expect_err("unknown")
            .to_string()
            .contains("Use only the tools")
    );
}
