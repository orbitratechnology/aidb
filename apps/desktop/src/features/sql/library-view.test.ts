import type { HistoryEntry } from '@aidb/contracts'
import { describe, expect, it } from 'vitest'
import { historyOutcome, sqlPreview } from './library-view'

const entry = (patch: Partial<HistoryEntry>): HistoryEntry => ({
  id: 1,
  executedAt: '2026-11-17T10:00:00.000Z',
  connectionId: null,
  sql: 'SELECT 1',
  status: 'completed',
  rowCount: null,
  elapsedMs: null,
  errorCode: null,
  ...patch,
})

describe('library view', () => {
  it('previews the first lines of a statement', () => {
    expect(sqlPreview('\n  \nSELECT 1')).toBe('SELECT 1')
    expect(sqlPreview('SELECT 1')).toBe('SELECT 1')
    expect(sqlPreview('a\nb\nc\nd\ne')).toBe('a\nb\nc\nd …')
    expect(sqlPreview('x'.repeat(1000))).toBe(`${'x'.repeat(400)} …`)
  })

  it('describes what happened to a run in words', () => {
    expect(historyOutcome(entry({ rowCount: 1200, elapsedMs: 4 }))).toBe('Ran · 1,200 rows · 4 ms')
    expect(historyOutcome(entry({ rowCount: 1, elapsedMs: 0 }))).toBe('Ran · 1 row · 0 ms')
    expect(historyOutcome(entry({ status: 'refused' }))).toBe('Not run')
    expect(
      historyOutcome(entry({ status: 'failed', errorCode: 'timeout', elapsedMs: 30000 })),
    ).toBe('Timed out · 30,000 ms')
    expect(historyOutcome(entry({ status: 'cancelled', elapsedMs: 12 }))).toBe('Cancelled · 12 ms')
  })
})
