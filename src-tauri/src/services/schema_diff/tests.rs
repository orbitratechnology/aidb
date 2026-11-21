use super::*;
use crate::adapters::local_store::test_support::temp_store;
use crate::domain::connections::ConnectionDraft;
use crate::domain::schema_diff::{CompareSource, GenerateOptions, StepAction};
use crate::errors::ErrorCode;
use crate::services::fakes::{sessions, target};

const FILE: &str = "
    SET client_encoding = 'UTF8';
    CREATE TABLE public.customers (id integer NOT NULL, email text);
    CREATE TABLE public.orders (id bigint PRIMARY KEY);
    CREATE FUNCTION public.f() RETURNS int LANGUAGE sql AS 'select 1';
";

#[tokio::test]
async fn a_file_is_compared_with_the_open_connection_and_the_result_is_held() {
    let (_dir, store) = temp_store().await;
    let profile = store
        .connections()
        .insert(
            &ConnectionDraft::new("Shop", "db", "orders", "reader")
                .validated()
                .expect("valid"),
        )
        .await
        .expect("insert");
    let (_adapter, sessions) = sessions();
    let credentials = Credentials::session_only("tests");
    let diffs = SchemaDiffs::default();
    let request = |source| CompareRequest {
        target_schema: "public".into(),
        source,
        options: GenerateOptions::default(),
    };

    let file = open_file(&diffs, "shop.sql".into(), FILE.as_bytes().to_vec()).expect("reads");
    assert_eq!(file.schemas, ["public"]);
    assert_eq!(file.statements, 4);
    let source = CompareSource::File {
        file_id: file.file_id,
        schema: "public".into(),
    };

    let closed = compare(
        &store,
        &credentials,
        &sessions,
        &diffs,
        profile.id,
        request(source.clone()),
        None,
    )
    .await
    .expect_err("not connected");
    assert_eq!(closed.code(), ErrorCode::Unavailable);

    sessions.open(profile.id, target("db")).await.expect("open");
    let diff = compare(
        &store,
        &credentials,
        &sessions,
        &diffs,
        profile.id,
        request(source),
        None,
    )
    .await
    .expect("compares");
    assert_eq!(diff.source, "shop.sql (public)");
    assert_eq!(diff.target, "public on Shop");
    let summaries: Vec<(&str, StepAction)> = diff
        .steps
        .iter()
        .map(|step| (step.summary.as_str(), step.action))
        .collect();
    assert_eq!(
        summaries,
        [
            ("Create table orders", StepAction::Add),
            ("Add column email to customers", StepAction::Add),
        ]
    );
    assert!(diff.notes[0].contains("1 function"), "{:?}", diff.notes);
    assert!(diff.script.contains("CREATE TABLE public.orders"));

    let (name, script) = super::script(&diffs, diff.diff_id).expect("held");
    assert_eq!(name, "migrate-public.sql");
    assert_eq!(script, diff.script);

    // The same schema on both sides, or one that is not there, is refused.
    let itself = compare(
        &store,
        &credentials,
        &sessions,
        &diffs,
        profile.id,
        request(CompareSource::Schema {
            schema: "public".into(),
        }),
        None,
    )
    .await
    .expect_err("same schema");
    assert_eq!(itself.code(), ErrorCode::InvalidInput);
    let missing = compare(
        &store,
        &credentials,
        &sessions,
        &diffs,
        profile.id,
        request(CompareSource::Schema {
            schema: "nowhere".into(),
        }),
        None,
    )
    .await
    .expect_err("no schema");
    assert_eq!(missing.code(), ErrorCode::NotFound);

    // A connection change drops what was compared on it; a closed file is gone.
    diffs.forget_connection(profile.id);
    assert!(super::script(&diffs, diff.diff_id).is_err());
    close_file(&diffs, file.file_id);
    let gone = compare(
        &store,
        &credentials,
        &sessions,
        &diffs,
        profile.id,
        request(CompareSource::File {
            file_id: file.file_id,
            schema: "public".into(),
        }),
        None,
    )
    .await
    .expect_err("closed");
    assert_eq!(gone.code(), ErrorCode::NotFound);
}

#[test]
fn files_must_be_utf8_sql() {
    let diffs = SchemaDiffs::default();
    let latin1 = open_file(
        &diffs,
        "a.sql".into(),
        b"CREATE TABLE caf\xe9 (a int);".to_vec(),
    );
    assert_eq!(latin1.expect_err("latin-1").code(), ErrorCode::InvalidInput);
    let empty = open_file(&diffs, "b.sql".into(), b"-- nothing\n".to_vec());
    assert_eq!(empty.expect_err("empty").code(), ErrorCode::InvalidInput);
    let bom = open_file(
        &diffs,
        "c.sql".into(),
        "\u{feff}CREATE TABLE t (a int);".into(),
    )
    .expect("reads");
    assert_eq!(bom.statements, 1);
    // Only the newest files are held.
    let ids: Vec<uuid::Uuid> = (0..MAX_FILES + 1)
        .map(|i| {
            open_file(&diffs, format!("{i}.sql"), b"select 1".to_vec())
                .expect("reads")
                .file_id
        })
        .collect();
    let held = lock(&diffs.files);
    assert_eq!(held.len(), MAX_FILES);
    assert!(!held.contains_key(&ids[0]));
}
