import { z } from 'zod'
import type { ExportFormat } from './generated/ExportFormat'
import type { TableExportOutcome } from './generated/TableExportOutcome'

export const ExportFormatSchema = z.enum(['csv', 'json']) satisfies z.ZodType<ExportFormat>

export const TableExportOutcomeSchema = z.object({
  fileName: z.string(),
  rows: z.number().int().nonnegative(),
  truncated: z.boolean(),
  maxRows: z.number().int().positive(),
}) satisfies z.ZodType<TableExportOutcome>
