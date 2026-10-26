// Types under `generated/` come from the Rust core via ts-rs (`cargo test` regenerates them).
// The schemas here validate data at runtime and are type-checked against those types.
export type { AppInfo } from './generated/AppInfo'
export type { CredentialBackend } from './generated/CredentialBackend'
export type { CredentialStoreStatus } from './generated/CredentialStoreStatus'
export type { DiagnosticsInfo } from './generated/DiagnosticsInfo'
export type { ErrorCode } from './generated/ErrorCode'
export type { ErrorPayload } from './generated/ErrorPayload'
export type { LocalStoreHealth } from './generated/LocalStoreHealth'
export type { LocalStoreStatus } from './generated/LocalStoreStatus'

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
