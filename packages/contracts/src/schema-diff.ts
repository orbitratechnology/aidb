import { z } from 'zod'
import type { CompareRequest } from './generated/CompareRequest'
import type { DiffSummary } from './generated/DiffSummary'
import type { MigrationStep } from './generated/MigrationStep'
import type { SchemaDiff } from './generated/SchemaDiff'
import type { SchemaFile } from './generated/SchemaFile'
import type { StepNote } from './generated/StepNote'
import { RiskLevelSchema } from './policy'

function utf8Length(text: string): number {
  let bytes = 0
  for (const char of text) {
    const code = char.codePointAt(0) ?? 0
    bytes += code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4
  }
  return bytes
}

/** PostgreSQL's limit on a name, in bytes; checked again in the core. */
const SchemaNameSchema = z
  .string()
  .min(1)
  .refine((name) => utf8Length(name) <= 63, 'Names are at most 63 bytes.')

export const CompareRequestSchema = z.object({
  targetSchema: SchemaNameSchema,
  source: z.discriminatedUnion('kind', [
    z.object({ kind: z.literal('schema'), schema: SchemaNameSchema }),
    z.object({ kind: z.literal('connection'), connectionId: z.uuid(), schema: SchemaNameSchema }),
    z.object({ kind: z.literal('file'), fileId: z.uuid(), schema: SchemaNameSchema }),
  ]),
  options: z.object({ concurrentIndexes: z.boolean() }),
}) satisfies z.ZodType<CompareRequest>

export const SchemaFileSchema = z.object({
  fileId: z.uuid(),
  fileName: z.string(),
  schemas: z.array(z.string()),
  statements: z.number().int().nonnegative(),
}) satisfies z.ZodType<SchemaFile>

export const StepNoteSchema = z.object({
  kind: z.enum(['data_loss', 'lock', 'may_fail', 'non_transactional']),
  text: z.string(),
}) satisfies z.ZodType<StepNote>

export const MigrationStepSchema = z.object({
  summary: z.string(),
  action: z.enum(['add', 'remove', 'change']),
  object: z.string(),
  sql: z.string(),
  transactional: z.boolean(),
  risk: RiskLevelSchema,
  notes: z.array(StepNoteSchema),
}) satisfies z.ZodType<MigrationStep>

const count = z.number().int().nonnegative()

export const DiffSummarySchema = z.object({
  tablesAdded: count,
  tablesRemoved: count,
  tablesChanged: count,
  viewsAdded: count,
  viewsRemoved: count,
  viewsChanged: count,
}) satisfies z.ZodType<DiffSummary>

export const SchemaDiffSchema = z.object({
  diffId: z.uuid(),
  source: z.string(),
  target: z.string(),
  targetSchema: z.string(),
  summary: DiffSummarySchema,
  steps: z.array(MigrationStepSchema),
  script: z.string(),
  notes: z.array(z.string()),
  risk: RiskLevelSchema,
}) satisfies z.ZodType<SchemaDiff>
