use super::*;

#[test]
fn requests_name_their_source_by_kind() {
    let request: CompareRequest = serde_json::from_str(
        r#"{"targetSchema":"public","source":{"kind":"file","fileId":"6f1c2a40-0000-4000-8000-000000000001","schema":"app"},"options":{"concurrentIndexes":true}}"#,
    )
    .expect("parses");
    assert_eq!(request.target_schema, "public");
    assert!(request.options.concurrent_indexes);
    assert!(matches!(request.source, CompareSource::File { ref schema, .. } if schema == "app"));

    let other: CompareRequest = serde_json::from_str(
        r#"{"targetSchema":"public","source":{"kind":"connection","connectionId":"6f1c2a40-0000-4000-8000-000000000002","schema":"public"}}"#,
    )
    .expect("parses");
    assert!(!other.options.concurrent_indexes);
    assert!(
        serde_json::from_str::<CompareRequest>(
            r#"{"targetSchema":"public","source":{"kind":"schema","schema":"a","sql":"DROP TABLE x"}}"#
        )
        .is_err(),
        "unknown fields are refused"
    );
}
