import {
  AppInfoSchema,
  AppSettingsSchema,
  ClearResultSchema,
  COMMANDS,
  DiagnosticsInfoSchema,
  StorageSummarySchema,
  type AppInfo,
  type AppSettings,
  type ClearResult,
  type ClearTarget,
  type DiagnosticsInfo,
  type SettingsPatch,
  type StorageSummary,
} from '@aidb/contracts'
import { z } from 'zod'
import { callCommand, NativeError } from './native'

export function getAppInfo(): Promise<AppInfo> {
  return callCommand(COMMANDS.appInfo, AppInfoSchema)
}

export function getDiagnostics(): Promise<DiagnosticsInfo> {
  return callCommand(COMMANDS.diagnostics, DiagnosticsInfoSchema)
}

export function getSettings(): Promise<AppSettings> {
  return callCommand(COMMANDS.getSettings, AppSettingsSchema)
}

/** Replaces the sections in `patch` and returns every setting as saved. */
export function updateSettings(patch: SettingsPatch): Promise<AppSettings> {
  return callCommand(COMMANDS.updateSettings, AppSettingsSchema, { patch })
}

export function getStorageSummary(): Promise<StorageSummary> {
  return callCommand(COMMANDS.storageSummary, StorageSummarySchema)
}

export function clearLocalData(target: ClearTarget): Promise<ClearResult> {
  return callCommand(COMMANDS.clearLocalData, ClearResultSchema, { target })
}

const EXTERNAL_URL = z.url({ protocol: /^(https?|mailto)$/ })

/** Opens a link in the system browser. The native core enforces the same rules. */
export async function openExternalUrl(url: string): Promise<void> {
  if (!EXTERNAL_URL.safeParse(url).success) {
    throw new NativeError('invalid_input', 'Only web and email links can be opened.')
  }
  await callCommand(COMMANDS.openExternalUrl, z.null(), { url })
}
