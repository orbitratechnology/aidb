import { z } from 'zod'
import { ActorSchema } from './audit'
import { ApprovalLevelSchema, RiskLevelSchema } from './policy'
import { ObjectNameSchema } from './catalog'
import type { ApplyStatus } from './generated/ApplyStatus'
import type { ChangeKind } from './generated/ChangeKind'
import type { ColumnValue } from './generated/ColumnValue'
import type { DataChange } from './generated/DataChange'
import { DATA_CHANGE_LIMITS } from './generated/DataChangeLimits'
import type { DataChangePreview } from './generated/DataChangePreview'
import type { DataChangeResult } from './generated/DataChangeResult'
import type { PendingChange } from './generated/PendingChange'
import type { PlannedChange } from './generated/PlannedChange'
import type { ValueSample } from './generated/ValueSample'
import { CellRefSchema, FilterSchema, TablePageSchema } from './table-data'
import { TABLE_DATA_LIMITS } from './generated/TableDataLimits'

export const ChangeKindSchema = z.enum([
  'insert',
  'update',
  'delete',
]) satisfies z.ZodType<ChangeKind>

const ValueSchema = z.string().nullable()

export const ColumnValueSchema = z.object({
  column: z.string(),
  value: ValueSchema,
}) satisfies z.ZodType<ColumnValue>

/** At least one filter: the core refuses an UPDATE or DELETE without one. */
const FiltersSchema = z.array(FilterSchema).min(1).max(TABLE_DATA_LIMITS.maxFilters)

export const DataChangeSchema = z.discriminatedUnion('kind', [
  z.object({
    kind: z.literal('insert'),
    table: ObjectNameSchema,
    columns: z.array(z.string()).min(1).max(DATA_CHANGE_LIMITS.maxChangeColumns),
    rows: z.array(z.array(ValueSchema)).min(1).max(DATA_CHANGE_LIMITS.maxInsertRows),
  }),
  z.object({
    kind: z.literal('update'),
    table: ObjectNameSchema,
    set: z.array(ColumnValueSchema).min(1).max(DATA_CHANGE_LIMITS.maxChangeColumns),
    filters: FiltersSchema,
  }),
  z.object({
    kind: z.literal('delete'),
    table: ObjectNameSchema,
    filters: FiltersSchema,
  }),
]) satisfies z.ZodType<DataChange>

export const ValueSampleSchema = z.object({
  columns: z.array(z.string()),
  rows: z.array(z.array(ValueSchema)),
  truncatedCells: z.array(CellRefSchema),
  totalRows: z.number().int().nonnegative(),
}) satisfies z.ZodType<ValueSample>

export const PlannedChangeSchema = z.object({
  kind: ChangeKindSchema,
  table: ObjectNameSchema,
  statement: z.string(),
  rows: z.number().int().nonnegative(),
  before: TablePageSchema.nullable(),
  after: ValueSampleSchema.nullable(),
}) satisfies z.ZodType<PlannedChange>

export const DataChangePreviewSchema = z.object({
  approvalId: z.uuid(),
  changes: z.array(PlannedChangeSchema).min(1),
  rows: z.number().int().nonnegative(),
  risk: RiskLevelSchema,
  approval: ApprovalLevelSchema,
  reasons: z.array(z.string()),
  confirmation: z.string().nullable(),
  expiresAt: z.string(),
}) satisfies z.ZodType<DataChangePreview>

/** The changes sent together: at least one, at most the core's limit. */
export const DataChangeSetSchema = z
  .array(DataChangeSchema)
  .min(1)
  .max(DATA_CHANGE_LIMITS.maxSetChanges)

export const ApplyStatusSchema = z.enum([
  'applied',
  'rolled_back',
  'uncertain',
]) satisfies z.ZodType<ApplyStatus>

export const DataChangeResultSchema = z.object({
  status: ApplyStatusSchema,
  rowsAffected: z.number().int().nonnegative().nullable(),
  message: z.string().nullable(),
}) satisfies z.ZodType<DataChangeResult>

export const PendingChangeSchema = z.object({
  connectionId: z.string(),
  actor: ActorSchema,
  preview: DataChangePreviewSchema,
  changes: z.array(DataChangeSchema),
}) satisfies z.ZodType<PendingChange>
