import { z } from 'zod'
import { ErrorCodeSchema } from './errors'
import type { HistoryEntry } from './generated/HistoryEntry'
import type { HistoryStatus } from './generated/HistoryStatus'
import type { SavedQuery } from './generated/SavedQuery'

export const HistoryStatusSchema = z.enum([
  'completed',
  'refused',
  'failed',
  'cancelled',
]) satisfies z.ZodType<HistoryStatus>

export const HistoryEntrySchema = z.object({
  id: z.number().int(),
  executedAt: z.string(),
  connectionId: z.uuid().nullable(),
  sql: z.string(),
  status: HistoryStatusSchema,
  rowCount: z.number().int().nonnegative().nullable(),
  elapsedMs: z.number().int().nonnegative().nullable(),
  errorCode: ErrorCodeSchema.nullable(),
}) satisfies z.ZodType<HistoryEntry>

export const SavedQuerySchema = z.object({
  id: z.uuid(),
  name: z.string(),
  sql: z.string(),
  connectionId: z.uuid().nullable(),
  createdAt: z.string(),
  updatedAt: z.string(),
}) satisfies z.ZodType<SavedQuery>
