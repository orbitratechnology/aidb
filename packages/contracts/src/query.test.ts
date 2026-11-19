import { describe, expect, it } from 'vitest'
import { QueryRunSchema } from './query'

const span = {
  start: { line: 1, column: 1, offset: 0 },
  end: { line: 1, column: 9, offset: 8 },
}

describe('QueryRunSchema', () => {
  it('reads each outcome by its status', () => {
    const completed = QueryRunSchema.parse({
      status: 'completed',
      statement: span,
      columns: [{ name: 'one', dataType: 'integer', kind: 'number' }],
      rows: [['1']],
      truncatedCells: [],
      offset: 0,
      hasMore: false,
      byteLimitReached: false,
      cellCharLimit: 2000,
      elapsedMs: 3,
    })
    const refused = QueryRunSchema.parse({
      status: 'refused',
      message: 'This statement changes data.',
      kind: 'write',
      position: span.start,
    })

    expect(completed.status === 'completed' && completed.rows).toEqual([['1']])
    expect(refused.status === 'refused' && refused.kind).toBe('write')
  })

  it('rejects an unknown status', () => {
    expect(QueryRunSchema.safeParse({ status: 'approved', message: 'x' }).success).toBe(false)
  })
})
