import { invoke } from '@tauri-apps/api/core'

export type AppInfo = {
  name: string
  version: string
  platform: string
}

export function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>('app_info')
}
