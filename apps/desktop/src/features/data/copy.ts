import type { PageColumn, ValueKind } from '@aidb/contracts'

const JSON_NUMBER = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/

function isJson(text: string): boolean {
  try {
    JSON.parse(text)
    return true
  } catch {
    return false
  }
}

/**
 * A value as JSON text, with the same rules as the native JSON export: exact numbers, real
 * booleans, embedded JSON and arrays, and strings for everything else (including cut values).
 */
function jsonValue(kind: ValueKind, value: string | null, truncated: boolean): string {
  if (value === null) return 'null'
  if (!truncated) {
    if (kind === 'boolean' && (value === 'true' || value === 'false')) return value
    if (kind === 'number' && JSON_NUMBER.test(value)) return value
    if ((kind === 'json' || kind === 'array') && isJson(value)) return value
  }
  return JSON.stringify(value)
}

/** One row as a JSON object, keyed by column name, one field per line. */
export function rowAsJson(
  columns: PageColumn[],
  values: Array<string | null>,
  truncated: (column: number) => boolean = () => false,
): string {
  const fields = columns.map(
    (column, index) =>
      `  ${JSON.stringify(column.name)}: ${jsonValue(column.kind, values[index] ?? null, truncated(index))}`,
  )
  return fields.length === 0 ? '{}' : `{\n${fields.join(',\n')}\n}`
}
