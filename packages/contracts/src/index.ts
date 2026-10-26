// Types under `generated/` come from the Rust core via ts-rs (`cargo test` regenerates them).
// The schemas here validate data at runtime and are type-checked against those types.
export type { AppInfo } from './generated/AppInfo'
export type { AppSettings } from './generated/AppSettings'
export type { AppearanceSettings } from './generated/AppearanceSettings'
export type { ClearResult } from './generated/ClearResult'
export type { ClearTarget } from './generated/ClearTarget'
export type { CredentialBackend } from './generated/CredentialBackend'
export type { CredentialStoreStatus } from './generated/CredentialStoreStatus'
export type { DiagnosticsInfo } from './generated/DiagnosticsInfo'
export type { ErrorCode } from './generated/ErrorCode'
export type { ErrorPayload } from './generated/ErrorPayload'
export type { LocalStoreHealth } from './generated/LocalStoreHealth'
export type { LocalStoreStatus } from './generated/LocalStoreStatus'
export type { SafetySettings } from './generated/SafetySettings'
export type { SettingsPatch } from './generated/SettingsPatch'
export type { StorageSummary } from './generated/StorageSummary'
export type { ThemePreference } from './generated/ThemePreference'
export { SAFETY_DEFAULTS, SAFETY_LIMITS } from './generated/SafetyLimits'

export { AppInfoSchema } from './app'
export { COMMANDS, type CommandName } from './commands'
export {
  CredentialBackendSchema,
  CredentialStoreStatusSchema,
  DiagnosticsInfoSchema,
  LocalStoreHealthSchema,
  LocalStoreStatusSchema,
} from './diagnostics'
export { ERROR_CODES, ErrorCodeSchema, ErrorPayloadSchema } from './errors'
export {
  AppSettingsSchema,
  AppearanceSettingsSchema,
  ClearResultSchema,
  ClearTargetSchema,
  SafetySettingsSchema,
  StorageSummarySchema,
  ThemePreferenceSchema,
} from './settings'
