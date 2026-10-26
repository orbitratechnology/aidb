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
];

fn main() {
    // Migrations are embedded by `sqlx::migrate!`; rebuild when they change.
    println!("cargo:rerun-if-changed=migrations");
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}
