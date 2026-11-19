import { z } from 'zod'
import type { CredentialBackend } from './generated/CredentialBackend'
import type { CredentialStoreStatus } from './generated/CredentialStoreStatus'
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

export const CredentialBackendSchema = z.enum([
  'os_store',
  'session_only',
]) satisfies z.ZodType<CredentialBackend>

export const CredentialStoreStatusSchema = z.object({
  backend: CredentialBackendSchema,
  reason: z.string().nullable(),
}) satisfies z.ZodType<CredentialStoreStatus>

export const DiagnosticsInfoSchema = z.object({
  dataDir: z.string(),
  logDir: z.string(),
  localStore: LocalStoreStatusSchema,
  credentialStore: CredentialStoreStatusSchema,
}) satisfies z.ZodType<DiagnosticsInfo>
