import { z } from 'zod'
import { ObjectNameSchema } from './catalog'
import type { CellRef } from './generated/CellRef'
import type { Filter } from './generated/Filter'
import type { FilterOperator } from './generated/FilterOperator'
import type { PageColumn } from './generated/PageColumn'
import type { PageRequest } from './generated/PageRequest'
import type { SortDirection } from './generated/SortDirection'
import type { SortKey } from './generated/SortKey'
import type { TablePage } from './generated/TablePage'
import { TABLE_DATA_LIMITS } from './generated/TableDataLimits'
import type { ValueKind } from './generated/ValueKind'

export const FilterOperatorSchema = z.enum([
  'equals',
  'not_equals',
  'less',
  'less_or_equal',
  'greater',
  'greater_or_equal',
  'contains',
  'starts_with',
  'is_null',
  'is_not_null',
]) satisfies z.ZodType<FilterOperator>

export const FilterSchema = z.object({
  column: z.string(),
  operator: FilterOperatorSchema,
  value: z.string().max(TABLE_DATA_LIMITS.maxFilterValueChars).optional(),
}) satisfies z.ZodType<Filter>

export const SortDirectionSchema = z.enum(['asc', 'desc']) satisfies z.ZodType<SortDirection>

export const SortKeySchema = z.object({
  column: z.string(),
  direction: SortDirectionSchema,
}) satisfies z.ZodType<SortKey>

export const PageRequestSchema = z.object({
  object: ObjectNameSchema,
  filters: z.array(FilterSchema).max(TABLE_DATA_LIMITS.maxFilters).optional(),
  sort: z.array(SortKeySchema).max(TABLE_DATA_LIMITS.maxSortKeys).optional(),
  offset: z.number().int().nonnegative().max(TABLE_DATA_LIMITS.maxOffset).optional(),
  limit: z.number().int().positive().optional(),
  operationId: z.uuid().optional(),
}) satisfies z.ZodType<PageRequest>

export const ValueKindSchema = z.enum([
  'boolean',
  'number',
  'text',
  'date_time',
  'json',
  'array',
  'uuid',
  'binary',
  'other',
]) satisfies z.ZodType<ValueKind>

export const PageColumnSchema = z.object({
  name: z.string(),
  dataType: z.string(),
  kind: ValueKindSchema,
}) satisfies z.ZodType<PageColumn>

export const CellRefSchema = z.object({
  row: z.number().int().nonnegative(),
  column: z.number().int().nonnegative(),
}) satisfies z.ZodType<CellRef>

export const TablePageSchema = z.object({
  object: ObjectNameSchema,
  columns: z.array(PageColumnSchema),
  omittedColumns: z.array(z.string()),
  rows: z.array(z.array(z.string().nullable())),
  truncatedCells: z.array(CellRefSchema),
  offset: z.number().int().nonnegative(),
  hasMore: z.boolean(),
  byteLimitReached: z.boolean(),
  order: z.array(SortKeySchema),
  stableOrder: z.boolean(),
  cellCharLimit: z.number().int().positive(),
}) satisfies z.ZodType<TablePage>
