import { describe, expect, it } from 'vitest'
import type { AnalyzedStatement, SqlAnalysis } from '@aidb/contracts'
import { docPosition, problemRange, statementAt } from './run-target'

const at = (offset: number) => ({ line: 1, column: offset + 1, offset })

const statement = (from: number, to: number): AnalyzedStatement => ({
  span: { start: at(from), end: at(to) },
  kind: 'read',
  reason: null,
  problemAt: null,
})

// "SELECT 1;  SELECT 2;"  → statements at 0–8 and 11–19
const analysis: SqlAnalysis = { statements: [statement(0, 8), statement(11, 19)], problem: null }

describe('statementAt', () => {
  it('picks the statement under the cursor, else the one before, else the next', () => {
    expect(statementAt(analysis, 3)).toBe(analysis.statements[0])
    expect(statementAt(analysis, 8)).toBe(analysis.statements[0])
    expect(statementAt(analysis, 10)).toBe(analysis.statements[0])
    expect(statementAt(analysis, 15)).toBe(analysis.statements[1])
    expect(statementAt(analysis, 25)).toBe(analysis.statements[1])
    expect(
      statementAt({ statements: [statement(5, 9)], problem: null }, 0)?.span.start.offset,
    ).toBe(5)
    expect(statementAt({ statements: [], problem: null }, 0)).toBeNull()
  })
})

describe('problemRange', () => {
  it('marks the word at the reported position, shifted by where the text started', () => {
    const doc = 'SELECT 1;\nSELECT nope FROM t'
    expect(problemRange(doc, 10, at(7))).toEqual({ from: 17, to: 21 })
    expect(problemRange(doc, 0, at(8))).toEqual({ from: 8, to: 9 })
    expect(problemRange(doc, 0, at(500))).toEqual({ from: doc.length, to: doc.length })
  })
})

describe('docPosition', () => {
  it('counts lines and columns from 1', () => {
    expect(docPosition('ab\ncd', 0)).toEqual({ line: 1, column: 1 })
    expect(docPosition('ab\ncd', 4)).toEqual({ line: 2, column: 2 })
    expect(docPosition('ab', 99)).toEqual({ line: 1, column: 3 })
  })
})
