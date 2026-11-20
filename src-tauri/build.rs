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
    "explain_query",
    "open_schema_file",
    "close_schema_file",
    "compare_schemas",
    "save_schema_script",
    "prepare_migration",
    "apply_migration",
    "cancel_migration",
    "list_migration_records",
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
    "list_pending_changes",
    "open_csv_import",
    "reread_csv_import",
    "preview_csv_import",
    "close_csv_import",
];

fn main() {
    // Migrations are embedded by `sqlx::migrate!`; rebuild when they change.
    println!("cargo:rerun-if-changed=migrations");
    let mut attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    if is_windows_msvc() {
        // Windows: the application manifest (Common Controls v6, which the dialog and menu code
        // need) is embedded by the linker for every binary, tests included. tauri-build would
        // add it to the app binary only, and the test binaries would then fail to start
        // (STATUS_ENTRYPOINT_NOT_FOUND). This is the setup tauri-build documents for tests.
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        embed_windows_manifest();
    }
    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}

fn is_windows_msvc() -> bool {
    std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
}

/// The same manifest tauri-build embeds by default (`windows-app-manifest.xml`).
fn embed_windows_manifest() {
    let manifest =
        std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("set by Cargo"))
            .join("windows-app-manifest.xml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
}
