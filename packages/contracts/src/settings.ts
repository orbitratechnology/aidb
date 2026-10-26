import { z } from 'zod'
import type { AppearanceSettings } from './generated/AppearanceSettings'
import type { AppSettings } from './generated/AppSettings'
import type { ClearResult } from './generated/ClearResult'
import type { ClearTarget } from './generated/ClearTarget'
import type { SafetySettings } from './generated/SafetySettings'
import { SAFETY_LIMITS } from './generated/SafetyLimits'
import type { StorageSummary } from './generated/StorageSummary'
import type { ThemePreference } from './generated/ThemePreference'

type Range = { readonly min: number; readonly max: number }

const formatNumber = new Intl.NumberFormat('en-US')

function withinRange({ min, max }: Range) {
  const outOfRange = `Must be between ${formatNumber.format(min)} and ${formatNumber.format(max)}.`
  return z
    .number({ error: 'Enter a whole number.' })
    .int({ error: 'Enter a whole number.' })
    .min(min, { error: outOfRange })
    .max(max, { error: outOfRange })
}

export const ThemePreferenceSchema = z.enum([
  'light',
  'dark',
  'system',
]) satisfies z.ZodType<ThemePreference>

export const AppearanceSettingsSchema = z.object({
  theme: ThemePreferenceSchema,
}) satisfies z.ZodType<AppearanceSettings>

/** Same rules as `SafetySettings::validate` in the native core, which checks again on save. */
export const SafetySettingsSchema = z
  .object({
    maxRowsPerPage: withinRange(SAFETY_LIMITS.maxRowsPerPage),
    maxResultMegabytes: withinRange(SAFETY_LIMITS.maxResultMegabytes),
    statementTimeoutSecs: withinRange(SAFETY_LIMITS.statementTimeoutSecs),
    lockTimeoutSecs: withinRange(SAFETY_LIMITS.lockTimeoutSecs),
  })
  .refine((settings) => settings.lockTimeoutSecs <= settings.statementTimeoutSecs, {
    path: ['lockTimeoutSecs'],
    error: 'Cannot be longer than the statement timeout.',
  }) satisfies z.ZodType<SafetySettings>

export const AppSettingsSchema = z.object({
  appearance: AppearanceSettingsSchema,
  safety: SafetySettingsSchema,
}) satisfies z.ZodType<AppSettings>

const count = z.number().int().nonnegative()

export const StorageSummarySchema = z.object({
  connections: count,
  savedQueries: count,
  agentTasks: count,
  auditRecords: count,
  schemaCacheEntries: count,
}) satisfies z.ZodType<StorageSummary>

export const ClearTargetSchema = z.enum([
  'task_history',
  'audit_log',
  'schema_cache',
]) satisfies z.ZodType<ClearTarget>

export const ClearResultSchema = z.object({
  removed: count,
}) satisfies z.ZodType<ClearResult>
