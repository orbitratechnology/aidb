import { describe, expect, it } from 'vitest'
import { TABLE_DATA_LIMITS } from './generated/TableDataLimits'
import { PageRequestSchema } from './table-data'

const object = { schema: 'public', name: 'orders' }

describe('PageRequestSchema', () => {
  it('accepts a bare request and filters within the native limits', () => {
    expect(PageRequestSchema.safeParse({ object }).success).toBe(true)
    const filters = Array.from({ length: TABLE_DATA_LIMITS.maxFilters }, () => ({
      column: 'id',
      operator: 'is_null' as const,
    }))
    expect(PageRequestSchema.safeParse({ object, filters }).success).toBe(true)
  })

  it('rejects requests the native core would refuse', () => {
    const tooMany = Array.from({ length: TABLE_DATA_LIMITS.maxSortKeys + 1 }, (_, i) => ({
      column: `c${i}`,
      direction: 'asc' as const,
    }))
    expect(PageRequestSchema.safeParse({ object, sort: tooMany }).success).toBe(false)
    expect(PageRequestSchema.safeParse({ object, limit: 0 }).success).toBe(false)
    expect(PageRequestSchema.safeParse({ object, offset: -1 }).success).toBe(false)
    expect(
      PageRequestSchema.safeParse({
        object,
        filters: [{ column: 'id', operator: 'equals', value: 'x'.repeat(1001) }],
      }).success,
    ).toBe(false)
  })
})
