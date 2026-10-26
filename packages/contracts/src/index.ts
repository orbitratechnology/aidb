// Types under `generated/` come from the Rust core via ts-rs (`cargo test` regenerates them).
// The schemas here validate data at runtime and are type-checked against those types.
export type { AppearanceSettings } from './generated/AppearanceSettings'
export type { AppInfo } from './generated/AppInfo'
export type { AppSettings } from './generated/AppSettings'
export type { ClearResult } from './generated/ClearResult'
export type { ClearTarget } from './generated/ClearTarget'
export type { ConnectionDetails } from './generated/ConnectionDetails'
export type { ConnectionDraft } from './generated/ConnectionDraft'
export type { ConnectionId } from './generated/ConnectionId'
export type { CredentialBackend } from './generated/CredentialBackend'
export type { CredentialStoreStatus } from './generated/CredentialStoreStatus'
export type { DiagnosticsInfo } from './generated/DiagnosticsInfo'
export type { Engine } from './generated/Engine'
export type { Environment } from './generated/Environment'
export type { ErrorCode } from './generated/ErrorCode'
export type { ErrorPayload } from './generated/ErrorPayload'
export type { ExportOutcome } from './generated/ExportOutcome'
export type { ImportCandidate } from './generated/ImportCandidate'
export type { ImportOutcome } from './generated/ImportOutcome'
export type { ImportPreview } from './generated/ImportPreview'
export type { ImportStatus } from './generated/ImportStatus'
export type { LocalStoreHealth } from './generated/LocalStoreHealth'
export type { LocalStoreStatus } from './generated/LocalStoreStatus'
export type { ParsedConnectionUri } from './generated/ParsedConnectionUri'
export type { PasswordStorage } from './generated/PasswordStorage'
export type { SafetySettings } from './generated/SafetySettings'
export type { SettingsPatch } from './generated/SettingsPatch'
export type { SkippedImport } from './generated/SkippedImport'
export type { SslMode } from './generated/SslMode'
export type { StorageSummary } from './generated/StorageSummary'
export type { ThemePreference } from './generated/ThemePreference'
export { SAFETY_DEFAULTS, SAFETY_LIMITS } from './generated/SafetyLimits'

export { AppInfoSchema } from './app'
export { COMMANDS, type CommandName } from './commands'
export {
  ConnectionDetailsSchema,
  ConnectionDraftSchema,
  EngineSchema,
  EnvironmentSchema,
  ExportOutcomeSchema,
  ImportCandidateSchema,
  ImportOutcomeSchema,
  ImportPreviewSchema,
  ImportStatusSchema,
  ParsedConnectionUriSchema,
  PasswordStorageSchema,
  SkippedImportSchema,
  SslModeSchema,
} from './connections'
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
