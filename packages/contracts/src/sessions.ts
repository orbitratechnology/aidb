import { z } from 'zod'
import { EngineSchema } from './connections'
import type { ActiveSession } from './generated/ActiveSession'
import type { Capabilities } from './generated/Capabilities'
import type { ConnectionTestReport } from './generated/ConnectionTestReport'
import type { ServerInfo } from './generated/ServerInfo'

export const CapabilitiesSchema = z.object({
  engine: EngineSchema,
  cancelStatements: z.boolean(),
  statementTimeout: z.boolean(),
  lockTimeout: z.boolean(),
  readOnlySessions: z.boolean(),
  transactionalDdl: z.boolean(),
  tls: z.boolean(),
}) satisfies z.ZodType<Capabilities>

export const ServerInfoSchema = z.object({
  engine: EngineSchema,
  version: z.string(),
  versionNumber: z.number().int().nonnegative(),
  supportedVersion: z.boolean(),
  user: z.string(),
  database: z.string(),
  encrypted: z.boolean(),
  tlsVersion: z.string().nullable(),
  readOnly: z.boolean(),
  inRecovery: z.boolean(),
}) satisfies z.ZodType<ServerInfo>

export const ActiveSessionSchema = z.object({
  connectionId: z.uuid(),
  capabilities: CapabilitiesSchema,
  server: ServerInfoSchema,
  connectedAt: z.string(),
}) satisfies z.ZodType<ActiveSession>

export const ConnectionTestReportSchema = z.object({
  server: ServerInfoSchema,
  roundTripMs: z.number().int().nonnegative(),
}) satisfies z.ZodType<ConnectionTestReport>
