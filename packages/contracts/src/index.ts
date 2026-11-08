// Types under `generated/` come from the Rust core via ts-rs (`cargo test` regenerates them).
// The schemas here validate data at runtime and are type-checked against those types.
export type { ActiveSession } from './generated/ActiveSession'
export type { AppearanceSettings } from './generated/AppearanceSettings'
export type { AppInfo } from './generated/AppInfo'
export type { AppSettings } from './generated/AppSettings'
export type { Capabilities } from './generated/Capabilities'
export type { CatalogOverview } from './generated/CatalogOverview'
export type { CatalogSnapshot } from './generated/CatalogSnapshot'
export type { ClearResult } from './generated/ClearResult'
export type { ClearTarget } from './generated/ClearTarget'
export type { Column } from './generated/Column'
export type { ConnectionDetails } from './generated/ConnectionDetails'
export type { ConnectionDraft } from './generated/ConnectionDraft'
export type { ConnectionId } from './generated/ConnectionId'
export type { ConnectionTestReport } from './generated/ConnectionTestReport'
export type { CredentialBackend } from './generated/CredentialBackend'
export type { CredentialStoreStatus } from './generated/CredentialStoreStatus'
export type { DiagnosticsInfo } from './generated/DiagnosticsInfo'
export type { Engine } from './generated/Engine'
export type { Environment } from './generated/Environment'
export type { ErrorCode } from './generated/ErrorCode'
export type { ErrorPayload } from './generated/ErrorPayload'
export type { ExportOutcome } from './generated/ExportOutcome'
export type { ForeignKey } from './generated/ForeignKey'
export type { Identity } from './generated/Identity'
export type { ImportCandidate } from './generated/ImportCandidate'
export type { ImportOutcome } from './generated/ImportOutcome'
export type { ImportPreview } from './generated/ImportPreview'
export type { ImportStatus } from './generated/ImportStatus'
export type { Index } from './generated/Index'
export type { KeyConstraint } from './generated/KeyConstraint'
export type { LocalStoreHealth } from './generated/LocalStoreHealth'
export type { LocalStoreStatus } from './generated/LocalStoreStatus'
export type { ObjectDetails } from './generated/ObjectDetails'
export type { ObjectKind } from './generated/ObjectKind'
export type { ObjectName } from './generated/ObjectName'
export type { ObjectSummary } from './generated/ObjectSummary'
export type { OperationId } from './generated/OperationId'
export type { ParsedConnectionUri } from './generated/ParsedConnectionUri'
export type { PasswordStorage } from './generated/PasswordStorage'
export type { ReferentialAction } from './generated/ReferentialAction'
export type { SafetySettings } from './generated/SafetySettings'
export type { SchemaSummary } from './generated/SchemaSummary'
export type { ServerInfo } from './generated/ServerInfo'
export type { SettingsPatch } from './generated/SettingsPatch'
export type { SkippedImport } from './generated/SkippedImport'
export type { SslMode } from './generated/SslMode'
export type { StorageSummary } from './generated/StorageSummary'
export type { ThemePreference } from './generated/ThemePreference'
export { SAFETY_DEFAULTS, SAFETY_LIMITS } from './generated/SafetyLimits'
export { CONNECTION_LIMITS } from './generated/ConnectionLimits'

export { AppInfoSchema } from './app'
export {
  CatalogSnapshotSchema,
  ColumnSchema,
  ForeignKeySchema,
  IdentitySchema,
  IndexSchema,
  KeyConstraintSchema,
  ObjectDetailsSchema,
  ObjectKindSchema,
  ObjectNameSchema,
  ObjectSummarySchema,
  ReferentialActionSchema,
  SchemaSummarySchema,
} from './catalog'
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
  ActiveSessionSchema,
  CapabilitiesSchema,
  ConnectionTestReportSchema,
  ServerInfoSchema,
} from './sessions'
export {
  AppSettingsSchema,
  AppearanceSettingsSchema,
  ClearResultSchema,
  ClearTargetSchema,
  SafetySettingsSchema,
  StorageSummarySchema,
  ThemePreferenceSchema,
} from './settings'
