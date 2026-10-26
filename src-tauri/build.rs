// App commands listed here get generated `allow-*` / `deny-*` permissions. Once listed, a command
// runs only in windows whose capability grants it. Keep in sync with `commands::handler`.
const COMMANDS: &[&str] = &["app_info", "diagnostics", "open_external_url"];

fn main() {
    // Migrations are embedded by `sqlx::migrate!`; rebuild when they change.
    println!("cargo:rerun-if-changed=migrations");
    let attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS));
    tauri_build::try_build(attributes).expect("failed to run tauri-build");
}
