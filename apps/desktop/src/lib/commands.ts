import {
  AppInfoSchema,
  AppSettingsSchema,
  ClearResultSchema,
  COMMANDS,
  ConnectionDetailsSchema,
  CredentialBackendSchema,
  DiagnosticsInfoSchema,
  ExportOutcomeSchema,
  ImportOutcomeSchema,
  ImportPreviewSchema,
  ParsedConnectionUriSchema,
  StorageSummarySchema,
  type AppInfo,
  type AppSettings,
  type ClearResult,
  type ClearTarget,
  type ConnectionDetails,
  type ConnectionDraft,
  type ConnectionId,
  type CredentialBackend,
  type DiagnosticsInfo,
  type ExportOutcome,
  type ImportOutcome,
  type ImportPreview,
  type ParsedConnectionUri,
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

export function listConnections(): Promise<ConnectionDetails[]> {
  return callCommand(COMMANDS.listConnections, z.array(ConnectionDetailsSchema))
}

type SaveOptions = {
  /** The user agreed to send the password and data unencrypted to another machine. */
  allowUnencrypted: boolean
}

export function createConnection(
  connection: ConnectionDraft,
  { allowUnencrypted }: SaveOptions,
): Promise<ConnectionDetails> {
  return callCommand(COMMANDS.createConnection, ConnectionDetailsSchema, {
    connection,
    allowUnencrypted,
  })
}

export function updateConnection(
  id: ConnectionId,
  connection: ConnectionDraft,
  { allowUnencrypted }: SaveOptions,
): Promise<ConnectionDetails> {
  return callCommand(COMMANDS.updateConnection, ConnectionDetailsSchema, {
    id,
    connection,
    allowUnencrypted,
  })
}

/** Deletes a profile; `forgetPassword` also removes its saved password. */
export async function deleteConnection(
  id: ConnectionId,
  { forgetPassword }: { forgetPassword: boolean },
): Promise<void> {
  await callCommand(COMMANDS.deleteConnection, z.null(), { id, forgetPassword })
}

/** Stores a password in the OS credential store, or for this session if none is available. */
export function saveConnectionPassword(
  id: ConnectionId,
  password: string,
): Promise<CredentialBackend> {
  return callCommand(COMMANDS.saveConnectionPassword, CredentialBackendSchema, { id, password })
}

export function forgetConnectionPassword(id: ConnectionId): Promise<boolean> {
  return callCommand(COMMANDS.forgetConnectionPassword, z.boolean(), { id })
}

/** Splits a PostgreSQL URI into form fields. The URI itself is never stored or logged. */
export function parseConnectionUri(uri: string): Promise<ParsedConnectionUri> {
  return callCommand(COMMANDS.parseConnectionUri, ParsedConnectionUriSchema, { uri })
}

/** Saves every profile, without passwords, to a file the user picks. `null` if cancelled. */
export function exportConnections(): Promise<ExportOutcome | null> {
  return callCommand(COMMANDS.exportConnections, ExportOutcomeSchema.nullable())
}

/** Reads a file the user picks and previews the import. `null` if cancelled. */
export function previewConnectionImport(): Promise<ImportPreview | null> {
  return callCommand(COMMANDS.previewConnectionImport, ImportPreviewSchema.nullable())
}

/** Saves the chosen profiles read-only; entries that cannot be saved are skipped and listed. */
export function importConnections(
  connections: ConnectionDraft[],
  { allowUnencrypted }: SaveOptions,
): Promise<ImportOutcome> {
  return callCommand(COMMANDS.importConnections, ImportOutcomeSchema, {
    connections,
    allowUnencrypted,
  })
}

const EXTERNAL_URL = z.url({ protocol: /^(https?|mailto)$/ })

/** Opens a link in the system browser. The native core enforces the same rules. */
export async function openExternalUrl(url: string): Promise<void> {
  if (!EXTERNAL_URL.safeParse(url).success) {
    throw new NativeError('invalid_input', 'Only web and email links can be opened.')
  }
  await callCommand(COMMANDS.openExternalUrl, z.null(), { url })
}
