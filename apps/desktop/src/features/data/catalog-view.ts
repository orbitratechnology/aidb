import type {
  CatalogSnapshot,
  ObjectDetails,
  ObjectKind,
  ObjectName,
  ObjectSummary,
  ReferentialAction,
  SchemaSummary,
} from '@aidb/contracts'

export const KIND_LABELS: Record<ObjectKind, string> = {
  table: 'Table',
  partitioned_table: 'Partitioned table',
  view: 'View',
  materialized_view: 'Materialized view',
  foreign_table: 'Foreign table',
}

export type SchemaGroup = {
  schema: SchemaSummary
  objects: ObjectSummary[]
}

/**
 * Objects grouped by schema, filtered by `query`. A query matches `schema.name`, so `sales.` lists
 * a whole schema and `ord` finds `orders` in every schema. Empty schemas show only unfiltered.
 */
export function groupObjects(catalog: CatalogSnapshot, query: string): SchemaGroup[] {
  const needle = query.trim().toLowerCase()
  const bySchema = new Map<string, ObjectSummary[]>()
  for (const object of catalog.objects) {
    if (needle && !`${object.schema}.${object.name}`.toLowerCase().includes(needle)) continue
    const list = bySchema.get(object.schema)
    if (list) list.push(object)
    else bySchema.set(object.schema, [object])
  }
  return catalog.schemas
    .map((schema) => ({ schema, objects: bySchema.get(schema.name) ?? [] }))
    .filter((group) => !needle || group.objects.length > 0)
}

const compact = new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 1 })
const exact = new Intl.NumberFormat('en-US')

/** `≈1.2K`, or `null` when the table was never analyzed. */
export function approximateCount(rows: number | null, { precise = false } = {}): string | null {
  if (rows === null) return null
  return `≈${precise ? exact.format(rows) : compact.format(rows)}`
}

/** `≈1.2K rows`, or `null` when the table was never analyzed. */
export function formatRowEstimate(rows: number | null, options?: { precise?: boolean }) {
  const count = approximateCount(rows, options)
  return count && `${count} ${rows === 1 ? 'row' : 'rows'}`
}

export function objectPath({ schema, name }: ObjectName): string {
  return `/data/${encodeURIComponent(schema)}/${encodeURIComponent(name)}`
}

export function sameObject(a: ObjectName, b: ObjectName): boolean {
  return a.schema === b.schema && a.name === b.name
}

export type ColumnKeys = { primary: boolean; unique: boolean; foreign: boolean }

/** Which keys each column takes part in. */
export function columnKeys(details: ObjectDetails): Map<string, ColumnKeys> {
  const keys = new Map<string, ColumnKeys>()
  const mark = (columns: readonly string[], key: keyof ColumnKeys) => {
    for (const column of columns) {
      const current = keys.get(column) ?? { primary: false, unique: false, foreign: false }
      current[key] = true
      keys.set(column, current)
    }
  }
  if (details.primaryKey) mark(details.primaryKey.columns, 'primary')
  for (const unique of details.uniqueConstraints) mark(unique.columns, 'unique')
  for (const foreign of details.foreignKeys) mark(foreign.columns, 'foreign')
  return keys
}

export const ACTION_LABELS: Record<ReferentialAction, string> = {
  no_action: 'no action',
  restrict: 'restrict',
  cascade: 'cascade',
  set_null: 'set null',
  set_default: 'set default',
}
