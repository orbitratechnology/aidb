import { z } from 'zod'
import type { DiagnosticsInfo } from './generated/DiagnosticsInfo'
import type { LocalStoreHealth } from './generated/LocalStoreHealth'
import type { LocalStoreStatus } from './generated/LocalStoreStatus'

export const LocalStoreHealthSchema = z.enum([
  'ready',
  'unavailable',
]) satisfies z.ZodType<LocalStoreHealth>

export const LocalStoreStatusSchema = z.object({
  state: LocalStoreHealthSchema,
  schemaVersion: z.number().int().nonnegative().nullable(),
  message: z.string().nullable(),
}) satisfies z.ZodType<LocalStoreStatus>

export const DiagnosticsInfoSchema = z.object({
  dataDir: z.string(),
  logDir: z.string(),
  localStore: LocalStoreStatusSchema,
}) satisfies z.ZodType<DiagnosticsInfo>
