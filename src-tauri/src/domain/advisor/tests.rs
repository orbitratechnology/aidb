use serde_json::json;

use super::*;

fn orders(indexes: Vec<IndexFacts>) -> TableFacts {
    TableFacts {
        schema: "public".into(),
        name: "orders".into(),
        rows: Some(100_000.0),
        last_analyzed: Some("2026-11-01 10:00:00+00".into()),
        columns: vec![
            "id".into(),
            "customer_id".into(),
            "email".into(),
            "total".into(),
            "placed_at".into(),
        ],
        indexes,
    }
}

fn primary_key() -> IndexFacts {
    IndexFacts {
        name: "orders_pkey".into(),
        columns: vec![Some("id".into())],
        partial: false,
    }
}

/// A measured sequential scan of `public.orders` keeping `kept` rows with `filter`.
fn scan(filter: &str, kept: f64, removed: f64) -> Value {
    json!([{
        "Plan": {
            "Node Type": "Seq Scan",
            "Relation Name": "orders",
            "Schema": "public",
            "Alias": "o",
            "Startup Cost": 0.0,
            "Total Cost": 1943.0,
            "Plan Rows": kept,
            "Plan Width": 40,
            "Actual Startup Time": 0.02,
            "Actual Total Time": 9.5,
            "Actual Rows": kept,
            "Actual Loops": 1,
            "Filter": filter,
            "Rows Removed by Filter": removed,
            "Shared Hit Blocks": 600,
            "Shared Read Blocks": 343
        },
        "Planning Time": 0.12,
        "Execution Time": 9.7
    }])
}

#[test]
fn a_selective_filter_over_a_whole_table_suggests_an_index() {
    let plan = advise(
        &scan("(o.customer_id = 42)", 12.0, 99_988.0),
        &[orders(vec![primary_key()])],
    )
    .expect("reads");
    assert!(plan.analyzed);
    assert_eq!(plan.execution_ms, Some(9.7));
    let node = &plan.nodes[0];
    assert_eq!(node.label, "Seq Scan on public.orders o");
    assert!(
        node.details
            .contains(&"Filter: (o.customer_id = 42)".to_owned())
    );
    assert!(
        node.details
            .contains(&"Rows removed by filter: 99,988".to_owned())
    );
    assert!(
        node.details
            .contains(&"Buffers: 600 hit, 343 read".to_owned())
    );
    assert_eq!(node.self_ms, Some(9.5));

    assert_eq!(plan.suggestions.len(), 1);
    let suggestion = &plan.suggestions[0];
    assert_eq!(suggestion.kind, SuggestionKind::MissingIndex);
    assert_eq!(suggestion.node, Some(0));
    assert_eq!(
        suggestion.sql.as_deref(),
        Some("CREATE INDEX CONCURRENTLY ON public.orders (customer_id);")
    );
    assert!(
        suggestion.evidence[0]
            .contains("Seq Scan on public.orders o read 100,000 rows and kept 12 (0.01%)"),
        "{:?}",
        suggestion.evidence
    );
}

#[test]
fn equality_columns_come_first_then_one_range_column() {
    let plan = advise(
        &scan(
            "((o.placed_at > '2026-01-01'::date) AND (o.customer_id = 42) AND (o.total >= 10))",
            5.0,
            99_995.0,
        ),
        &[orders(vec![])],
    )
    .expect("reads");
    assert_eq!(
        plan.suggestions[0].sql.as_deref(),
        Some("CREATE INDEX CONCURRENTLY ON public.orders (customer_id, placed_at);")
    );
}

#[test]
fn no_index_is_suggested_when_one_exists_cannot_help_or_would_not_pay() {
    let indexed = orders(vec![IndexFacts {
        name: "orders_customer".into(),
        columns: vec![Some("customer_id".into()), Some("placed_at".into())],
        partial: false,
    }]);
    let cases = [
        // An index that starts with the column is there already.
        (scan("(o.customer_id = 42)", 12.0, 99_988.0), indexed),
        // A function of the column: a plain index would not serve it.
        (
            scan("(lower(o.email) = 'a@b.c'::text)", 1.0, 99_999.0),
            orders(vec![]),
        ),
        // Most rows are kept.
        (scan("(o.total > 1)", 90_000.0, 10_000.0), orders(vec![])),
        // A small table.
        (scan("(o.customer_id = 42)", 1.0, 400.0), orders(vec![])),
    ];
    for (output, table) in cases {
        let plan = advise(&output, &[table]).expect("reads");
        assert!(plan.suggestions.is_empty(), "{:?}", plan.suggestions);
    }
}

#[test]
fn a_planned_scan_is_judged_from_statistics() {
    let output = json!([{"Plan": {
        "Node Type": "Seq Scan", "Relation Name": "orders", "Schema": "public", "Alias": "orders",
        "Total Cost": 18334.0, "Plan Rows": 10, "Plan Width": 40,
        "Filter": "(orders.customer_id = 42)"
    }}]);
    let mut table = orders(vec![]);
    table.rows = Some(1_000_000.0);
    let plan = advise(&output, &[table]).expect("reads");
    assert!(!plan.analyzed);
    assert_eq!(plan.nodes[0].label, "Seq Scan on public.orders");
    let evidence = &plan.suggestions[0].evidence;
    assert!(
        evidence[0].contains("is expected to read 1,000,000 rows"),
        "{evidence:?}"
    );
    assert!(
        evidence
            .last()
            .is_some_and(|e| e.contains("run it to measure"))
    );
    // Never analyzed: no row estimate, nothing to judge from.
    let mut unknown = orders(vec![]);
    unknown.rows = None;
    let output = json!([{"Plan": {
        "Node Type": "Seq Scan", "Relation Name": "orders", "Schema": "public", "Alias": "orders",
        "Total Cost": 1.0, "Plan Rows": 10, "Filter": "(orders.customer_id = 42)"
    }}]);
    assert!(
        advise(&output, &[unknown])
            .expect("reads")
            .suggestions
            .is_empty()
    );
}

#[test]
fn an_inner_scan_repeated_by_a_nested_loop_counts_every_loop() {
    let output = json!([{"Plan": {
        "Node Type": "Nested Loop", "Join Type": "Inner", "Total Cost": 90000.0,
        "Plan Rows": 5000,
        "Plans": [
            {"Node Type": "Seq Scan", "Parent Relationship": "Outer", "Relation Name": "customers",
             "Schema": "public", "Alias": "c", "Total Cost": 100.0, "Plan Rows": 5000,
             "Parallel Aware": true},
            {"Node Type": "Seq Scan", "Parent Relationship": "Inner", "Relation Name": "orders",
             "Schema": "public", "Alias": "o", "Total Cost": 18.0, "Plan Rows": 1,
             "Filter": "(o.customer_id = c.id)"}
        ]
    }}]);
    let mut table = orders(vec![primary_key()]);
    table.rows = Some(1_000.0);
    let plan = advise(&output, &[table]).expect("reads");
    assert_eq!(plan.nodes.len(), 3);
    assert_eq!(
        plan.nodes[1].label,
        "Parallel Seq Scan on public.customers c"
    );
    assert_eq!(plan.nodes[2].parent, Some(0));
    assert_eq!(plan.nodes[2].depth, 1);
    let suggestion = &plan.suggestions[0];
    assert_eq!(suggestion.node, Some(2));
    assert!(
        suggestion.evidence[0].contains("over 5,000 loops"),
        "{:?}",
        suggestion.evidence
    );
}

#[test]
fn estimates_far_off_suggest_analyzing() {
    let output = json!([{"Plan": {
        "Node Type": "Index Scan", "Index Name": "orders_pkey", "Relation Name": "orders",
        "Schema": "public", "Alias": "o", "Total Cost": 8.0, "Plan Rows": 5,
        "Actual Total Time": 40.0, "Actual Rows": 52000, "Actual Loops": 1,
        "Index Cond": "(o.id > 100)"
    }}]);
    let mut table = orders(vec![primary_key()]);
    table.last_analyzed = None;
    let plan = advise(&output, &[table]).expect("reads");
    assert_eq!(
        plan.nodes[0].label,
        "Index Scan using orders_pkey on public.orders o"
    );
    let suggestion = &plan.suggestions[0];
    assert_eq!(suggestion.kind, SuggestionKind::StaleStatistics);
    assert_eq!(suggestion.sql.as_deref(), Some("ANALYZE public.orders;"));
    assert!(suggestion.evidence[0].contains("estimated 5 rows per loop, got 52,000"));
    assert!(suggestion.evidence[1].contains("never been analyzed"));
}

#[test]
fn spills_and_lossy_bitmaps_suggest_more_memory() {
    let output: Value = serde_json::from_str(
        r#"[{"Plan": {
        "Node Type": "Sort", "Total Cost": 50000.0, "Plan Rows": 1000000,
        "Actual Total Time": 900.0, "Actual Rows": 1000000, "Actual Loops": 1,
        "Sort Key": ["o.total DESC"], "Sort Method": "external merge",
        "Sort Space Used": 51200, "Sort Space Type": "Disk",
        "Plans": [{
            "Node Type": "Hash Join", "Parent Relationship": "Outer", "Total Cost": 30000.0,
            "Plan Rows": 1000000, "Actual Total Time": 500.0, "Actual Rows": 1000000,
            "Actual Loops": 1, "Hash Cond": "(o.customer_id = c.id)",
            "Plans": [
                {"Node Type": "Bitmap Heap Scan", "Parent Relationship": "Outer",
                 "Relation Name": "orders", "Schema": "public", "Alias": "o",
                 "Total Cost": 100.0, "Plan Rows": 1000000, "Actual Total Time": 200.0,
                 "Actual Rows": 1000000, "Actual Loops": 1, "Lossy Heap Blocks": 4000,
                 "Recheck Cond": "(o.total > 0)"},
                {"Node Type": "Hash", "Parent Relationship": "Inner", "Total Cost": 50.0,
                 "Plan Rows": 20000, "Actual Total Time": 30.0, "Actual Rows": 20000,
                 "Actual Loops": 1, "Hash Batches": 4, "Original Hash Batches": 1,
                 "Peak Memory Usage": 4097,
                 "Plans": [{"Node Type": "Seq Scan", "Parent Relationship": "Outer",
                    "Relation Name": "customers", "Schema": "public", "Alias": "c",
                    "Total Cost": 40.0, "Plan Rows": 20000, "Actual Total Time": 10.0,
                    "Actual Rows": 20000, "Actual Loops": 1}]}
            ]
        }]
    }}]"#,
    )
    .expect("fixture");
    let plan = advise(&output, &[orders(vec![primary_key()])]).expect("reads");
    let kinds: Vec<SuggestionKind> = plan.suggestions.iter().map(|s| s.kind).collect();
    assert_eq!(
        kinds,
        [
            SuggestionKind::SortSpill,
            SuggestionKind::HashSpill,
            SuggestionKind::LossyBitmap
        ]
    );
    assert!(plan.suggestions[0].detail.contains("about 100 MB"));
    assert!(plan.suggestions[1].evidence[0].contains("4 batches (planned 1)"));
    // Time in each node alone.
    assert_eq!(plan.nodes[0].self_ms, Some(400.0));
    assert_eq!(plan.nodes[1].self_ms, Some(270.0));
    assert!(
        plan.nodes[0]
            .details
            .contains(&"Sort key: o.total DESC".to_owned())
    );
}

#[test]
fn index_only_scans_fetching_from_the_table_suggest_vacuum() {
    let output = json!([{"Plan": {
        "Node Type": "Index Only Scan", "Index Name": "orders_pkey", "Relation Name": "orders",
        "Schema": "public", "Alias": "o", "Total Cost": 100.0, "Plan Rows": 5000,
        "Actual Total Time": 4.0, "Actual Rows": 5000, "Actual Loops": 1, "Heap Fetches": 4800
    }}]);
    let plan = advise(&output, &[orders(vec![primary_key()])]).expect("reads");
    assert_eq!(plan.suggestions[0].kind, SuggestionKind::HeapFetches);
    assert_eq!(
        plan.suggestions[0].sql.as_deref(),
        Some("VACUUM public.orders;")
    );
}

#[test]
fn the_assistant_gets_the_tree_and_the_findings() {
    let plan = advise(
        &scan("(o.customer_id = 42)", 12.0, 99_988.0),
        &[orders(vec![primary_key()])],
    )
    .expect("reads");
    let text = render(&plan);
    assert!(
        text.starts_with(
            "-> Seq Scan on public.orders o (cost 1943.00, rows 12; actual rows 12, 9.50 ms)\n"
        ),
        "{text}"
    );
    assert!(text.contains("     Filter: (o.customer_id = 42)\n"));
    assert!(text.contains("Execution: 9.70 ms"));
    assert!(text.contains("aidb's findings"));
    assert!(text.contains("Suggested: CREATE INDEX CONCURRENTLY ON public.orders (customer_id);"));
}

#[test]
fn other_output_is_not_a_plan() {
    assert!(read_plan(&json!({"rows": 1})).is_err());
    assert!(read_plan(&json!([{"Plan": {"Total Cost": 1}}])).is_err());
    assert!(read_plan(&json!("QUERY PLAN")).is_err());
    let one =
        read_plan(&json!([{"Plan": {"Node Type": "Result", "Total Cost": 0.01, "Plan Rows": 1}}]))
            .expect("reads");
    assert_eq!(one.nodes[0].label, "Result");
    assert!(one.suggestions.is_empty());
}
