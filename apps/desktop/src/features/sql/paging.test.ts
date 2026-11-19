import { describe, expect, it } from 'vitest'
import { pageTrail, rowSummary, shiftPosition } from './paging'

const page = (offset: number, rows: number, hasMore: boolean) => ({
  offset,
  hasMore,
  rows: Array.from({ length: rows }, () => []),
})

describe('result paging', () => {
  it('moves forward by the rows a page held and back the way it came', () => {
    // A page cut short by the size limit holds fewer rows than the next one.
    const first = pageTrail(page(0, 40, true), [], 'next')
    expect(first).toEqual({ offset: 40, previous: [0] })
    const second = pageTrail(page(40, 500, true), first!.previous, 'next')
    expect(second).toEqual({ offset: 540, previous: [0, 40] })
    expect(pageTrail(page(540, 3, false), second!.previous, 'previous')).toEqual({
      offset: 40,
      previous: [0],
    })
    expect(pageTrail(page(0, 3, false), [], 'previous')).toBeNull()
    expect(pageTrail(page(540, 3, false), [0, 40], 'next')).toBeNull()
  })

  it('summarises a page by its row range', () => {
    expect(rowSummary(page(0, 1, false))).toBe('1 row')
    expect(rowSummary(page(0, 1200, false))).toBe('1,200 rows')
    expect(rowSummary(page(0, 500, true))).toBe('Rows 1–500')
    expect(rowSummary(page(500, 20, false))).toBe('Rows 501–520')
    expect(rowSummary(page(500, 0, false))).toBe('No rows after row 500')
  })

  it('maps positions in the text that ran into the draft', () => {
    const start = { line: 3, column: 5 }
    expect(shiftPosition(start, { line: 1, column: 8, offset: 7 })).toEqual({ line: 3, column: 12 })
    expect(shiftPosition(start, { line: 2, column: 4, offset: 20 })).toEqual({ line: 4, column: 4 })
  })
})
