// App commands listed here get generated `allow-*` / `deny-*` permissions. Once listed, a command
// runs only in windows whose capability grants it. Keep in sync with `commands::handler`.
const COMMANDS: &[&str] = &[
    "app_info",
    "diagnostics",
    "open_external_url",
    "get_settings",
    "update_settings",
    "storage_summary",
    "clear_local_data",
    "list_connections",
    "create_connection",
    "update_connection",
    "delete_connection",
    "save_connection_password",
    "forget_connection_password",
    "parse_connection_uri",
    "export_connections",
    "preview_connection_import",
    "import_connections",
    "test_connection",
    "connect",
    "disconnect",
    "active_sessions",
    "catalog_overview",
    "refresh_catalog",
    "describe_object",
    "read_table_page",
    "cancel_operation",
    "export_table",
    "analyze_sql",
    "run_query",
    "list_query_history",
    "delete_query_history_entry",
    "list_saved_queries",
    "save_query",
    "delete_saved_query",
    "list_providers",
    "create_provider",
    "update_provider",
    "delete_provider",
    "save_provider_key",
    "forget_provider_key",
    "test_provider",
    "preview_assistant_context",
    "ask_assistant",
    "cancel_assistant_task",
    "list_audit_events",
    "preview_data_change",
    "apply_data_change",
    "cancel_data_change",
];

fn main() {
    // Migrations are embedded by `sqlx::migrate!`; rebuild when they change.
    println!("cargo:rerun-if-changed=migrations");
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}
