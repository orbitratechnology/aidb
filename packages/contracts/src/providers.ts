import { z } from 'zod'
import type { CheckKind } from './generated/CheckKind'
import type { CheckOutcome } from './generated/CheckOutcome'
import type { KeyStorage } from './generated/KeyStorage'
import type { ModelInfo } from './generated/ModelInfo'
import type { ProviderCheck } from './generated/ProviderCheck'
import type { ProviderDetails } from './generated/ProviderDetails'
import type { ProviderDraft } from './generated/ProviderDraft'
import type { ProviderFeatures } from './generated/ProviderFeatures'
import type { ProviderPreset } from './generated/ProviderPreset'
import type { ProviderTestReport } from './generated/ProviderTestReport'
import type { StructuredOutput } from './generated/StructuredOutput'

/** Limits shared with `domain::providers::limits` in the native core, which checks again. */
export const PROVIDER_LIMITS = {
  nameMax: 100,
  modelMax: 200,
  endpointMax: 2048,
  requestTimeoutSecs: { min: 5, max: 600 },
  contextBudgetTokens: { min: 1_000, max: 10_000_000 },
} as const

export const ProviderPresetSchema = z.enum([
  'anthropic',
  'openai',
  'ollama',
  'openai_compatible',
]) satisfies z.ZodType<ProviderPreset>

export const StructuredOutputSchema = z.enum([
  'json_schema',
  'json_object',
  'instructions',
]) satisfies z.ZodType<StructuredOutput>

export const KeyStorageSchema = z.enum([
  'os_store',
  'session_only',
  'not_saved',
]) satisfies z.ZodType<KeyStorage>

export const ProviderFeaturesSchema = z.object({
  toolCalls: z.boolean(),
  structuredOutput: StructuredOutputSchema,
}) satisfies z.ZodType<ProviderFeatures>

export const ProviderDraftSchema = z.object({
  name: z.string().trim().min(1).max(PROVIDER_LIMITS.nameMax),
  kind: ProviderPresetSchema,
  endpoint: z.string().trim().max(PROVIDER_LIMITS.endpointMax).nullable(),
  model: z.string().trim().min(1).max(PROVIDER_LIMITS.modelMax),
  requestTimeoutSecs: z
    .number()
    .int()
    .min(PROVIDER_LIMITS.requestTimeoutSecs.min)
    .max(PROVIDER_LIMITS.requestTimeoutSecs.max),
  contextBudgetTokens: z
    .number()
    .int()
    .min(PROVIDER_LIMITS.contextBudgetTokens.min)
    .max(PROVIDER_LIMITS.contextBudgetTokens.max)
    .nullable(),
  allowInsecureHttp: z.boolean(),
}) satisfies z.ZodType<ProviderDraft>

export const ProviderDetailsSchema = ProviderDraftSchema.extend({
  id: z.uuid(),
  endpointUrl: z.string(),
  key: KeyStorageSchema,
  keyRequired: z.boolean(),
  insecureRemote: z.boolean(),
  features: ProviderFeaturesSchema,
  checkedAt: z.string().nullable(),
  createdAt: z.string(),
  updatedAt: z.string(),
}) satisfies z.ZodType<ProviderDetails>

export const ModelInfoSchema = z.object({
  id: z.string(),
  displayName: z.string().nullable(),
  contextWindow: z.number().int().nonnegative().nullable(),
  maxOutputTokens: z.number().int().nonnegative().nullable(),
}) satisfies z.ZodType<ModelInfo>

export const CheckKindSchema = z.enum([
  'endpoint',
  'model',
  'answer',
  'tool_calls',
  'structured_output',
]) satisfies z.ZodType<CheckKind>

export const CheckOutcomeSchema = z.enum([
  'passed',
  'warning',
  'failed',
  'skipped',
]) satisfies z.ZodType<CheckOutcome>

export const ProviderCheckSchema = z.object({
  kind: CheckKindSchema,
  outcome: CheckOutcomeSchema,
  detail: z.string(),
}) satisfies z.ZodType<ProviderCheck>

export const ProviderTestReportSchema = z.object({
  checks: z.array(ProviderCheckSchema),
  models: z.array(ModelInfoSchema),
  answerMs: z.number().int().nonnegative().nullable(),
  features: ProviderFeaturesSchema.nullable(),
}) satisfies z.ZodType<ProviderTestReport>
