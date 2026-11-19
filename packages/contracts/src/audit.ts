import { z } from 'zod'
import { RiskLevelSchema } from './assistant'
import { EnvironmentSchema } from './connections'
import type { Actor } from './generated/Actor'
import type { AuditEntry } from './generated/AuditEntry'
import type { AuditStatus } from './generated/AuditStatus'

export const ActorSchema = z.enum(['user', 'agent', 'system']) satisfies z.ZodType<Actor>

export const AuditStatusSchema = z.enum([
  'succeeded',
  'failed',
  'partial',
  'cancelled',
  'rejected',
]) satisfies z.ZodType<AuditStatus>

export const AuditEntrySchema = z.object({
  id: z.number().int(),
  occurredAt: z.string(),
  connectionId: z.string().nullable(),
  environment: EnvironmentSchema.nullable(),
  actor: ActorSchema,
  operation: z.string(),
  risk: RiskLevelSchema.nullable(),
  fingerprint: z.string().nullable(),
  approvalState: z.string().nullable(),
  status: AuditStatusSchema,
  rowsAffected: z.number().int().nonnegative().nullable(),
  detail: z.string().nullable(),
}) satisfies z.ZodType<AuditEntry>
