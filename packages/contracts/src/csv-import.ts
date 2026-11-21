import { z } from 'zod'
import { ValueSampleSchema } from './data-change'
import type { ColumnMapping } from './generated/ColumnMapping'
import type { CsvImportDraft } from './generated/CsvImportDraft'
import { CSV_IMPORT_LIMITS } from './generated/CsvImportLimits'
import type { CsvImportPreview } from './generated/CsvImportPreview'
import type { CsvImportReady } from './generated/CsvImportReady'
import type { CsvImportRequest } from './generated/CsvImportRequest'
import type { CsvOptions } from './generated/CsvOptions'
import { ApprovalLevelSchema, RiskLevelSchema } from './policy'

export const CsvOptionsSchema = z.object({
  encoding: z.enum(CSV_IMPORT_LIMITS.encodings),
  delimiter: z.enum(CSV_IMPORT_LIMITS.delimiters),
  hasHeader: z.boolean(),
}) satisfies z.ZodType<CsvOptions>

export const CsvImportDraftSchema = z.object({
  importId: z.uuid(),
  fileName: z.string(),
  options: CsvOptionsSchema,
  columns: z.array(z.string()),
  suggested: z.array(z.string().nullable()),
  sample: z.array(z.array(z.string())),
  totalRows: z.number().int().nonnegative(),
  warnings: z.array(z.string()),
}) satisfies z.ZodType<CsvImportDraft>

export const ColumnMappingSchema = z.object({
  fileColumn: z.number().int().nonnegative(),
  column: z.string(),
}) satisfies z.ZodType<ColumnMapping>

export const CsvImportRequestSchema = z.object({
  mapping: z.array(ColumnMappingSchema).max(CSV_IMPORT_LIMITS.maxColumns),
  upsertKey: z.array(z.string()).min(1).optional(),
}) satisfies z.ZodType<CsvImportRequest>

export const CsvImportReadySchema = z.object({
  approvalId: z.uuid(),
  rows: z.number().int().nonnegative(),
  batches: z.number().int().positive(),
  after: ValueSampleSchema,
  statement: z.string(),
  risk: RiskLevelSchema,
  approval: ApprovalLevelSchema,
  reasons: z.array(z.string()),
  confirmation: z.string().nullable(),
  expiresAt: z.string(),
}) satisfies z.ZodType<CsvImportReady>

export const CsvImportPreviewSchema = z.object({
  problems: z.array(z.string()),
  ready: CsvImportReadySchema.nullable(),
}) satisfies z.ZodType<CsvImportPreview>
