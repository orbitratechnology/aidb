use super::*;
use crate::domain::advisor::SuggestionKind;
use crate::errors::ErrorCode;
use crate::services::fakes::{sessions, target};

#[test]
fn only_one_query_is_explained() {
    assert_eq!(
        the_query("  SELECT * FROM orders WHERE id = 1;  ").expect("a query"),
        "SELECT * FROM orders WHERE id = 1"
    );
    for refused in [
        "",
        "DELETE FROM orders",
        "SELECT 1; SELECT 2",
        "EXPLAIN SELECT 1",
        "SHOW work_mem",
        "SELECT 'unclosed",
    ] {
        let error = the_query(refused).expect_err(refused);
        assert_eq!(error.code(), ErrorCode::InvalidInput, "{refused}");
    }
}

#[tokio::test]
async fn a_plan_comes_back_with_its_suggestions() {
    let (_adapter, sessions) = sessions();
    let id = crate::domain::connections::ConnectionId::new();
    let request = |sql: &str, analyze| ExplainRequest {
        sql: sql.into(),
        analyze,
        operation_id: None,
    };
    let closed = explain(&sessions, id, &request("SELECT 1", false)).await;
    assert_eq!(closed.expect_err("closed").code(), ErrorCode::Unavailable);

    sessions.open(id, target("db")).await.expect("open");
    let planned = explain(&sessions, id, &request("SELECT * FROM orders", false))
        .await
        .expect("plans");
    assert!(!planned.analyzed);
    assert_eq!(planned.suggestions[0].kind, SuggestionKind::MissingIndex);
    let measured = explain(&sessions, id, &request("SELECT * FROM orders", true))
        .await
        .expect("measures");
    assert!(measured.analyzed);
    assert!(measured.suggestions[0].evidence[0].contains("read 100,000 rows"));
}
