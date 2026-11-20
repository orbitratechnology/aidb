import { z } from 'zod'
import { EnvironmentSchema } from './connections'
import type { FailedStep } from './generated/FailedStep'
import type { MigrationPreview } from './generated/MigrationPreview'
import type { MigrationRecord } from './generated/MigrationRecord'
import type { MigrationResult } from './generated/MigrationResult'
import { ApprovalLevelSchema, RiskLevelSchema } from './policy'

export const MigrationStatusSchema = z.enum([
  'running',
  'succeeded',
  'failed',
  'partial',
  'uncertain',
])
export const VerificationSchema = z.enum(['passed', 'failed', 'not_run'])

export const MigrationPreviewSchema = z.object({
  approvalId: z.uuid(),
  diffId: z.uuid(),
  steps: z.number().int().positive(),
  dataLoss: z.array(z.string()),
  outsideTransaction: z.number().int().nonnegative(),
  risk: RiskLevelSchema,
  approval: ApprovalLevelSchema,
  reasons: z.array(z.string()),
  confirmation: z.string().nullable(),
  expiresAt: z.string(),
}) satisfies z.ZodType<MigrationPreview>

export const FailedStepSchema = z.object({
  number: z.number().int().positive(),
  summary: z.string(),
  message: z.string(),
}) satisfies z.ZodType<FailedStep>

export const MigrationResultSchema = z.object({
  recordId: z.uuid(),
  status: MigrationStatusSchema,
  appliedSteps: z.number().int().nonnegative(),
  totalSteps: z.number().int().positive(),
  failedStep: FailedStepSchema.nullable(),
  verification: VerificationSchema,
  remaining: z.number().int().nonnegative(),
  message: z.string(),
}) satisfies z.ZodType<MigrationResult>

export const MigrationRecordSchema = z.object({
  id: z.uuid(),
  connectionId: z.string().nullable(),
  environment: EnvironmentSchema.nullable(),
  targetSchema: z.string(),
  source: z.string(),
  scriptSha256: z.string().length(64),
  steps: z.number().int().positive(),
  appliedSteps: z.number().int().nonnegative(),
  risk: RiskLevelSchema,
  startedAt: z.string(),
  finishedAt: z.string().nullable(),
  status: MigrationStatusSchema,
  verification: VerificationSchema,
  detail: z.string().nullable(),
}) satisfies z.ZodType<MigrationRecord>
