import { z } from 'zod'
import type { ConnectionDetails } from './generated/ConnectionDetails'
import type { ConnectionDraft } from './generated/ConnectionDraft'
import type { Engine } from './generated/Engine'
import type { Environment } from './generated/Environment'
import type { ExportOutcome } from './generated/ExportOutcome'
import type { ImportCandidate } from './generated/ImportCandidate'
import type { ImportOutcome } from './generated/ImportOutcome'
import type { ImportPreview } from './generated/ImportPreview'
import type { ImportStatus } from './generated/ImportStatus'
import type { ParsedConnectionUri } from './generated/ParsedConnectionUri'
import type { PasswordStorage } from './generated/PasswordStorage'
import type { SkippedImport } from './generated/SkippedImport'
import type { SslMode } from './generated/SslMode'

// Response shapes. Field rules live in the native core, which validates every profile it stores.

export const EngineSchema = z.enum(['postgres']) satisfies z.ZodType<Engine>

export const EnvironmentSchema = z.enum([
  'local',
  'development',
  'staging',
  'production',
  'other',
]) satisfies z.ZodType<Environment>

export const SslModeSchema = z.enum([
  'disable',
  'require',
  'verify-ca',
  'verify-full',
]) satisfies z.ZodType<SslMode>

export const PasswordStorageSchema = z.enum([
  'os_store',
  'session_only',
  'not_saved',
]) satisfies z.ZodType<PasswordStorage>

const port = z.number().int().min(1).max(65_535)
const count = z.number().int().nonnegative()

export const ConnectionDraftSchema = z.object({
  name: z.string(),
  host: z.string(),
  port,
  database: z.string(),
  username: z.string(),
  sslMode: SslModeSchema,
  caCertPath: z.string().nullable(),
  connectTimeoutSecs: z.number().int().positive(),
  environment: EnvironmentSchema,
  readOnly: z.boolean(),
}) satisfies z.ZodType<ConnectionDraft>

export const ConnectionDetailsSchema = ConnectionDraftSchema.extend({
  id: z.uuid(),
  engine: EngineSchema,
  password: PasswordStorageSchema,
  unencryptedRemote: z.boolean(),
  createdAt: z.string(),
  updatedAt: z.string(),
}) satisfies z.ZodType<ConnectionDetails>

export const ParsedConnectionUriSchema = z.object({
  host: z.string(),
  port,
  database: z.string().nullable(),
  username: z.string().nullable(),
  sslMode: SslModeSchema.nullable(),
  caCertPath: z.string().nullable(),
  connectTimeoutSecs: z.number().int().nonnegative().nullable(),
  password: z.string().nullable(),
  suggestedName: z.string(),
  notes: z.array(z.string()),
}) satisfies z.ZodType<ParsedConnectionUri>

export const ExportOutcomeSchema = z.object({
  count,
  fileName: z.string(),
}) satisfies z.ZodType<ExportOutcome>

export const ImportStatusSchema = z.enum([
  'ready',
  'duplicate',
  'invalid',
]) satisfies z.ZodType<ImportStatus>

export const ImportCandidateSchema = z.object({
  label: z.string(),
  status: ImportStatusSchema,
  problem: z.string().nullable(),
  connection: ConnectionDraftSchema.nullable(),
  writesDisabled: z.boolean(),
  unencryptedRemote: z.boolean(),
}) satisfies z.ZodType<ImportCandidate>

export const ImportPreviewSchema = z.object({
  fileName: z.string(),
  candidates: z.array(ImportCandidateSchema),
}) satisfies z.ZodType<ImportPreview>

export const SkippedImportSchema = z.object({
  name: z.string(),
  reason: z.string(),
}) satisfies z.ZodType<SkippedImport>

export const ImportOutcomeSchema = z.object({
  imported: count,
  skipped: z.array(SkippedImportSchema),
}) satisfies z.ZodType<ImportOutcome>
