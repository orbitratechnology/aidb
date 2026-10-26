/**
 * Names of the native commands the UI may invoke. Each one is also listed in
 * `src-tauri/build.rs` and granted in `src-tauri/capabilities/`.
 */
export const COMMANDS = {
  appInfo: 'app_info',
  diagnostics: 'diagnostics',
  openExternalUrl: 'open_external_url',
  getSettings: 'get_settings',
  updateSettings: 'update_settings',
  storageSummary: 'storage_summary',
  clearLocalData: 'clear_local_data',
  listConnections: 'list_connections',
  createConnection: 'create_connection',
  updateConnection: 'update_connection',
  deleteConnection: 'delete_connection',
  saveConnectionPassword: 'save_connection_password',
  forgetConnectionPassword: 'forget_connection_password',
  parseConnectionUri: 'parse_connection_uri',
  exportConnections: 'export_connections',
  previewConnectionImport: 'preview_connection_import',
  importConnections: 'import_connections',
} as const

export type CommandName = (typeof COMMANDS)[keyof typeof COMMANDS]
