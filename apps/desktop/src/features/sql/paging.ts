import type { QueryResult, SourcePosition } from '@aidb/contracts'
import type { DraftPosition } from './run-store'

/**
 * Where a page starts, and the starts of the pages before it. Pages end where the size limits
 * stop them, so the way back is remembered rather than computed.
 */
export type PageTrail = { offset: number; previous: number[] }

/** The trail for the page after or before `result`; `null` if there is none that way. */
export function pageTrail(
  result: Pick<QueryResult, 'offset' | 'hasMore'> & { rows: unknown[] },
  previous: number[],
  direction: 'next' | 'previous',
): PageTrail | null {
  if (direction === 'next') {
    if (!result.hasMore || result.rows.length === 0) return null
    return { offset: result.offset + result.rows.length, previous: [...previous, result.offset] }
  }
  const offset = previous.at(-1)
  return offset === undefined ? null : { offset, previous: previous.slice(0, -1) }
}

/** "12 rows", or the range a later or partial page covers: "Rows 101–200". */
export function rowSummary(result: Pick<QueryResult, 'offset' | 'hasMore'> & { rows: unknown[] }) {
  const count = result.rows.length
  const format = (n: number) => n.toLocaleString('en-US')
  if (result.offset === 0 && !result.hasMore)
    return `${format(count)} ${count === 1 ? 'row' : 'rows'}`
  if (count === 0) return `No rows after row ${format(result.offset)}`
  return `Rows ${format(result.offset + 1)}–${format(result.offset + count)}`
}

/** A position within the text that ran, in the draft, given where that text starts there. */
export function shiftPosition(start: DraftPosition, position: SourcePosition): DraftPosition {
  return position.line === 1
    ? { line: start.line, column: start.column + position.column - 1 }
    : { line: start.line + position.line - 1, column: position.column }
}
