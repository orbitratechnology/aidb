import { invoke, isTauri } from '@tauri-apps/api/core'

export type AppInfo = {
  name: string
  version: string
  platform: string
}

// `pnpm dev:web` runs the UI in a plain browser, where no native core exists.
async function callNative<T>(command: string): Promise<T> {
  if (!isTauri()) {
    throw new Error('The native core is only available in the aidb desktop app.')
  }
  return invoke<T>(command)
}

export function getAppInfo(): Promise<AppInfo> {
  return callNative<AppInfo>('app_info')
}
