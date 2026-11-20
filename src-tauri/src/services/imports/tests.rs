use super::*;
use crate::adapters::local_store::test_support::temp_store;
use crate::domain::connections::ConnectionDraft;
use crate::domain::csv_import::ColumnMapping;
use crate::services::fakes::{sessions, target};

fn orders() -> ObjectName {
    ObjectName {
        schema: "public".into(),
        name: "orders".into(),
    }
}

#[tokio::test]
async fn files_are_held_read_again_and_forgotten() {
    let (_dir, store) = temp_store().await;
    let profile = store
        .connections()
        .insert(
            &ConnectionDraft::new("Shop", "db", "orders", "writer")
                .validated()
                .expect("valid"),
        )
        .await
        .expect("insert");
    let (_adapter, sessions) = sessions();
    sessions.open(profile.id, target("db")).await.expect("open");
    let imports = Imports::default();
    let approvals = Approvals::default();

    let draft = open(
        &sessions,
        &imports,
        profile.id,
        orders(),
        "orders.csv".into(),
        b"id;note\n1;a\n2;b\n".to_vec(),
    )
    .await
    .expect("opens");
    assert_eq!(draft.options.delimiter, ";");
    assert!(draft.options.has_header);
    assert_eq!(draft.total_rows, 2);

    let again = reread(
        &imports,
        draft.import_id,
        CsvOptions {
            has_header: false,
            ..draft.options.clone()
        },
    )
    .expect("reads again");
    assert_eq!(again.total_rows, 3);
    assert_eq!(again.columns, vec!["Column 1", "Column 2"]);

    // Another connection cannot use it; problems come back before anything is planned.
    let request = CsvImportRequest {
        mapping: vec![ColumnMapping {
            file_column: 0,
            column: "id".into(),
        }],
        upsert_key: None,
    };
    let foreign = preview(
        &store,
        &sessions,
        &approvals,
        &imports,
        ConnectionId::new(),
        draft.import_id,
        request.clone(),
    )
    .await
    .expect_err("another connection");
    assert_eq!(foreign.code(), ErrorCode::NotFound);
    let checked = preview(
        &store,
        &sessions,
        &approvals,
        &imports,
        profile.id,
        draft.import_id,
        request.clone(),
    )
    .await
    .expect("checked");
    // The fake table has no columns.
    assert_eq!(
        checked.problems,
        vec!["The table has no column id.".to_owned()]
    );
    assert!(checked.ready.is_none());

    close(&imports, draft.import_id);
    let gone = reread(&imports, draft.import_id, draft.options.clone()).expect_err("closed");
    assert_eq!(gone.code(), ErrorCode::NotFound);

    // Bad files are refused when opened, and only a few files are held at once.
    let empty = open(
        &sessions,
        &imports,
        profile.id,
        orders(),
        "x.csv".into(),
        Vec::new(),
    )
    .await
    .expect_err("empty");
    assert_eq!(empty.code(), ErrorCode::InvalidInput);
    let mut ids = Vec::new();
    for _ in 0..=MAX_HELD {
        let draft = open(
            &sessions,
            &imports,
            profile.id,
            orders(),
            "x.csv".into(),
            b"a,b\n1,2\n".to_vec(),
        )
        .await
        .expect("opens");
        ids.push(draft.import_id);
    }
    assert!(
        reread(&imports, ids[0], draft.options.clone()).is_err(),
        "the oldest went"
    );
    imports.forget_connection(profile.id);
    assert!(reread(&imports, ids[MAX_HELD], draft.options).is_err());
}
