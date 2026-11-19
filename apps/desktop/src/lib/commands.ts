import {
  ActiveSessionSchema,
  AppInfoSchema,
  AppSettingsSchema,
  CatalogSnapshotSchema,
  ClearResultSchema,
  COMMANDS,
  ConnectionDetailsSchema,
  ConnectionTestReportSchema,
  ContextPreviewSchema,
  CredentialBackendSchema,
  ProviderDetailsSchema,
  ProviderTestReportSchema,
  DiagnosticsInfoSchema,
  ExportOutcomeSchema,
  HistoryEntrySchema,
  ObjectDetailsSchema,
  ImportOutcomeSchema,
  ImportPreviewSchema,
  ParsedConnectionUriSchema,
  QueryRunSchema,
  SavedQuerySchema,
  SqlAnalysisSchema,
  StorageSummarySchema,
  TableExportOutcomeSchema,
  TablePageSchema,
  TaskEventSchema,
  TaskOutcomeSchema,
  type ActiveSession,
  type AskRequest,
  type AppInfo,
  type AppSettings,
  type CatalogSnapshot,
  type ClearResult,
  type ClearTarget,
  type ConnectionDetails,
  type ConnectionDraft,
  type ConnectionId,
  type ConnectionTestReport,
  type ContextPreview,
  type CredentialBackend,
  type ProviderDetails,
  type ProviderDraft,
  type ProviderId,
  type ProviderTestReport,
  type DiagnosticsInfo,
  type ExportOutcome,
  type ExportRequest,
  type HistoryEntry,
  type ImportOutcome,
  type ImportPreview,
  type ObjectDetails,
  type ObjectName,
  type OperationId,
  type PageRequest,
  type ParsedConnectionUri,
  type QueryRequest,
  type QueryRun,
  type SavedQuery,
  type SavedQueryDraft,
  type SettingsPatch,
  type SqlAnalysis,
  type StorageSummary,
  type TableExportOutcome,
  type TablePage,
  type TaskEvent,
  type TaskId,
  type TaskOutcome,
} from '@aidb/contracts'
import { Channel } from '@tauri-apps/api/core'
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

/**
 * Tests connection settings, saved or not. With no password, the one saved for `savedId` is
 * used. Nothing is kept open.
 */
export function testConnection(
  connection: ConnectionDraft,
  {
    password,
    savedId,
    allowUnencrypted,
  }: SaveOptions & { password?: string; savedId?: ConnectionId },
): Promise<ConnectionTestReport> {
  return callCommand(COMMANDS.testConnection, ConnectionTestReportSchema, {
    connection,
    password: password ?? null,
    savedId: savedId ?? null,
    allowUnencrypted,
  })
}

/** Opens a session for a saved profile. A password given here is used for this session only. */
export function connect(id: ConnectionId, password?: string): Promise<ActiveSession> {
  return callCommand(COMMANDS.connect, ActiveSessionSchema, { id, password: password ?? null })
}

export function disconnect(id: ConnectionId): Promise<boolean> {
  return callCommand(COMMANDS.disconnect, z.boolean(), { id })
}

export function activeSessions(): Promise<ActiveSession[]> {
  return callCommand(COMMANDS.activeSessions, z.array(ActiveSessionSchema))
}

/** Schemas, tables and views of an open connection; cached by the core until the schema changes. */
export function catalogOverview(connectionId: ConnectionId): Promise<CatalogSnapshot> {
  return callCommand(COMMANDS.catalogOverview, CatalogSnapshotSchema, { connectionId })
}

/** Reloads an open connection's schema details from the database. */
export function refreshCatalog(connectionId: ConnectionId): Promise<CatalogSnapshot> {
  return callCommand(COMMANDS.refreshCatalog, CatalogSnapshotSchema, { connectionId })
}

/** Columns, keys, references and indexes of one table or view. */
export function describeObject(
  connectionId: ConnectionId,
  object: ObjectName,
): Promise<ObjectDetails> {
  return callCommand(COMMANDS.describeObject, ObjectDetailsSchema, { connectionId, object })
}

/** One bounded page of a table or view. Pass `operationId` to be able to cancel it. */
export function readTablePage(
  connectionId: ConnectionId,
  request: PageRequest,
): Promise<TablePage> {
  return callCommand(COMMANDS.readTablePage, TablePageSchema, { connectionId, request })
}

/**
 * Asks where to save, then writes every matching row as CSV or JSON. `null` if the user closed
 * the dialog. Pass `operationId` to be able to cancel it.
 */
export function exportTable(
  connectionId: ConnectionId,
  request: ExportRequest,
): Promise<TableExportOutcome | null> {
  return callCommand(COMMANDS.exportTable, TableExportOutcomeSchema.nullable(), {
    connectionId,
    request,
  })
}

/** Splits SQL into statements and says which can run. Needs no connection. */
export function analyzeSql(sql: string): Promise<SqlAnalysis> {
  return callCommand(COMMANDS.analyzeSql, SqlAnalysisSchema, { sql })
}

/**
 * Runs one read statement. Refusals and database errors are outcomes (`status`), not thrown
 * errors. Pass `operationId` to be able to cancel it.
 */
export function runQuery(connectionId: ConnectionId, request: QueryRequest): Promise<QueryRun> {
  return callCommand(COMMANDS.runQuery, QueryRunSchema, { connectionId, request })
}

/** SQL run on a connection, newest first, optionally filtered by text. */
export function listQueryHistory(
  connectionId: ConnectionId,
  search?: string,
): Promise<HistoryEntry[]> {
  return callCommand(COMMANDS.listQueryHistory, z.array(HistoryEntrySchema), {
    connectionId,
    search: search || null,
  })
}

export function deleteQueryHistoryEntry(id: number): Promise<void> {
  return callCommand(COMMANDS.deleteQueryHistoryEntry, z.null(), { id }).then(() => undefined)
}

/** Saved queries, most recently changed first, optionally filtered by name or text. */
export function listSavedQueries(search?: string): Promise<SavedQuery[]> {
  return callCommand(COMMANDS.listSavedQueries, z.array(SavedQuerySchema), {
    search: search || null,
  })
}

/** Creates a saved query, or updates the one named by `draft.id`. */
export function saveQuery(draft: SavedQueryDraft): Promise<SavedQuery> {
  return callCommand(COMMANDS.saveQuery, SavedQuerySchema, { draft })
}

export function deleteSavedQuery(id: string): Promise<void> {
  return callCommand(COMMANDS.deleteSavedQuery, z.null(), { id }).then(() => undefined)
}

/** Asks the database to stop a running read. `false` if it already finished. */
export function cancelOperation(
  connectionId: ConnectionId,
  operationId: OperationId,
): Promise<boolean> {
  return callCommand(COMMANDS.cancelOperation, z.boolean(), { connectionId, operationId })
}

const EXTERNAL_URL = z.url({ protocol: /^(https?|mailto)$/ })

/** Opens a link in the system browser. The native core enforces the same rules. */
export async function openExternalUrl(url: string): Promise<void> {
  if (!EXTERNAL_URL.safeParse(url).success) {
    throw new NativeError('invalid_input', 'Only web and email links can be opened.')
  }
  await callCommand(COMMANDS.openExternalUrl, z.null(), { url })
}

/** AI provider profiles, ordered by name. Never includes API keys. */
export function listProviders(): Promise<ProviderDetails[]> {
  return callCommand(COMMANDS.listProviders, z.array(ProviderDetailsSchema))
}

export function createProvider(provider: ProviderDraft): Promise<ProviderDetails> {
  return callCommand(COMMANDS.createProvider, ProviderDetailsSchema, { provider })
}

export function updateProvider(id: ProviderId, provider: ProviderDraft): Promise<ProviderDetails> {
  return callCommand(COMMANDS.updateProvider, ProviderDetailsSchema, { id, provider })
}

/** Deletes the profile and its API key. */
export function deleteProvider(id: ProviderId): Promise<void> {
  return callCommand(COMMANDS.deleteProvider, z.null(), { id }).then(() => undefined)
}

/** Sends the key once to the core, which keeps it in the OS credential store. */
export function saveProviderKey(id: ProviderId, key: string): Promise<CredentialBackend> {
  return callCommand(COMMANDS.saveProviderKey, CredentialBackendSchema, { id, key })
}

export function forgetProviderKey(id: ProviderId): Promise<boolean> {
  return callCommand(COMMANDS.forgetProviderKey, z.boolean(), { id })
}

/** Sends a few short, fixed prompts to the provider and saves what it supports. */
export function testProvider(id: ProviderId): Promise<ProviderTestReport> {
  return callCommand(COMMANDS.testProvider, ProviderTestReportSchema, { id })
}

/** What a question would send to the provider, for review. Nothing is sent. */
export function previewAssistantContext(
  connectionId: ConnectionId,
  request: AskRequest,
): Promise<ContextPreview> {
  return callCommand(COMMANDS.previewAssistantContext, ContextPreviewSchema, {
    connectionId,
    request,
  })
}

/**
 * Asks the assistant a question on the open connection. Progress arrives through `onEvent`;
 * events that do not match the contract are dropped.
 */
export function askAssistant(
  connectionId: ConnectionId,
  request: AskRequest,
  onEvent: (event: TaskEvent) => void,
): Promise<TaskOutcome> {
  const channel = new Channel<unknown>((raw) => {
    const event = TaskEventSchema.safeParse(raw)
    if (event.success) onEvent(event.data)
  })
  return callCommand(COMMANDS.askAssistant, TaskOutcomeSchema, {
    connectionId,
    request,
    onEvent: channel,
  })
}

/** Stops a running question. Resolves to whether it was still running. */
export function cancelAssistantTask(taskId: TaskId): Promise<boolean> {
  return callCommand(COMMANDS.cancelAssistantTask, z.boolean(), { taskId })
}
