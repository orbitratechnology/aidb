// Types under `generated/` come from the Rust core via ts-rs (`cargo test` regenerates them).
// The schemas here validate data at runtime and are type-checked against those types.
export type { AppInfo } from './generated/AppInfo'
export type { ErrorCode } from './generated/ErrorCode'
export type { ErrorPayload } from './generated/ErrorPayload'

export { AppInfoSchema } from './app'
export { COMMANDS, type CommandName } from './commands'
export { ERROR_CODES, ErrorCodeSchema, ErrorPayloadSchema } from './errors'
