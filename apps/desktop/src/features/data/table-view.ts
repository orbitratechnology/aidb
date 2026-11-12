import type { Filter, FilterOperator, PageColumn, TablePage, ValueKind } from '@aidb/contracts'

export const OPERATOR_LABELS: Record<FilterOperator, string> = {
  equals: 'equals',
  not_equals: 'does not equal',
  less: 'is less than',
  less_or_equal: 'is at most',
  greater: 'is greater than',
  greater_or_equal: 'is at least',
  contains: 'contains',
  starts_with: 'starts with',
  is_null: 'is NULL',
  is_not_null: 'is not NULL',
}

const NULL_CHECKS: FilterOperator[] = ['is_null', 'is_not_null']
const TEXT_MATCH: FilterOperator[] = ['contains', 'starts_with']
const ORDERED: FilterOperator[] = [
  'equals',
  'not_equals',
  'less',
  'less_or_equal',
  'greater',
  'greater_or_equal',
]

/** Operators that make sense for a column's values. The database has the final say. */
export function operatorsFor(kind: ValueKind): FilterOperator[] {
  switch (kind) {
    case 'number':
    case 'date_time':
      return [...ORDERED, ...NULL_CHECKS]
    case 'text':
    case 'uuid':
      return [...ORDERED, ...TEXT_MATCH, ...NULL_CHECKS]
    case 'boolean':
      return ['equals', 'not_equals', ...NULL_CHECKS]
    case 'json':
    case 'array':
    case 'binary':
    case 'other':
      return [...TEXT_MATCH, ...NULL_CHECKS]
  }
}

export function takesValue(operator: FilterOperator): boolean {
  return !NULL_CHECKS.includes(operator)
}

/** `name equals 'x'`, for chips and announcements. */
export function describeFilter({ column, operator, value }: Filter): string {
  const label = `${column} ${OPERATOR_LABELS[operator]}`
  return takesValue(operator) ? `${label} “${value ?? ''}”` : label
}

/** `json` has no ordering operator in PostgreSQL; `jsonb` does. */
export function canSort(column: PageColumn): boolean {
  return column.dataType !== 'json'
}

/** A starting width that fits typical values of the kind, in pixels. */
export function defaultColumnWidth(kind: ValueKind): number {
  switch (kind) {
    case 'boolean':
      return 100
    case 'number':
      return 140
    case 'uuid':
      return 310
    case 'date_time':
      return 230
    case 'json':
    case 'array':
      return 260
    default:
      return 200
  }
}

/** Numbers are right-aligned so their digits line up. */
export function alignsRight(kind: ValueKind): boolean {
  return kind === 'number'
}

const count = new Intl.NumberFormat('en-US')

/** `Rows 501–1,000`, or `No rows`. */
export function rowRange(offset: number, rows: number): string {
  if (rows === 0) return 'No rows'
  return `Rows ${count.format(offset + 1)}–${count.format(offset + rows)}`
}

/** Keys of cut cells, `row:column`, for quick lookup. */
export function truncatedKeys(page: TablePage): Set<string> {
  return new Set(page.truncatedCells.map(({ row, column }) => `${row}:${column}`))
}

/** A value laid out for reading: JSON and arrays indented, everything else as stored. */
export function formatForDetail(kind: ValueKind, value: string, truncated: boolean): string {
  if ((kind === 'json' || kind === 'array') && !truncated) {
    try {
      return JSON.stringify(JSON.parse(value), null, 2)
    } catch {
      return value
    }
  }
  return value
}
