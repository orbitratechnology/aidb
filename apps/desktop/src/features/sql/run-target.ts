import type { AnalyzedStatement, SourcePosition, SqlAnalysis } from '@aidb/contracts'

/**
 * The statement to run for a cursor at `cursor` (a UTF-16 offset): the one containing it,
 * otherwise the last one before it, otherwise the first one after it.
 */
export function statementAt(analysis: SqlAnalysis, cursor: number): AnalyzedStatement | null {
  const { statements } = analysis
  const containing = statements.find(
    (s) => s.span.start.offset <= cursor && cursor <= s.span.end.offset,
  )
  if (containing) return containing
  const before = statements.filter((s) => s.span.end.offset < cursor).at(-1)
  return before ?? statements.find((s) => s.span.start.offset > cursor) ?? null
}

/**
 * The range to mark for a problem reported at `position` within text that started at `base` in
 * the document: the word there, or one character.
 */
export function problemRange(
  doc: string,
  base: number,
  position: SourcePosition,
): { from: number; to: number } {
  const from = Math.min(base + position.offset, doc.length)
  const word = /^[\w$."]+/.exec(doc.slice(from, from + 200))
  const to = Math.min(from + Math.max(word?.[0].length ?? 0, 1), doc.length)
  return { from, to: Math.max(to, from) }
}

/** Line and column (from 1) of a UTF-16 offset in `doc`. */
export function docPosition(doc: string, offset: number): { line: number; column: number } {
  const before = doc.slice(0, Math.min(offset, doc.length))
  const lineStart = before.lastIndexOf('\n') + 1
  return { line: before.split('\n').length, column: before.length - lineStart + 1 }
}
