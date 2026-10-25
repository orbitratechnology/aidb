/**
 * Names of the native commands the UI may invoke. Each one is also listed in
 * `src-tauri/build.rs` and granted in `src-tauri/capabilities/`.
 */
export const COMMANDS = {
  appInfo: 'app_info',
  openExternalUrl: 'open_external_url',
} as const

export type CommandName = (typeof COMMANDS)[keyof typeof COMMANDS]
