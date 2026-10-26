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
} as const

export type CommandName = (typeof COMMANDS)[keyof typeof COMMANDS]
