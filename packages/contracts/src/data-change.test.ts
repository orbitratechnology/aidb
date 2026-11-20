import { describe, expect, it } from 'vitest'
import {
  DataChangePreviewSchema,
  DataChangeResultSchema,
  DataChangeSchema,
  DataChangeSetSchema,
} from './data-change'
import { DATA_CHANGE_LIMITS } from './generated/DataChangeLimits'

const table = { schema: 'public', name: 'orders' }
const filters = [{ column: 'id', operator: 'equals' as const, value: '7' }]

describe('DataChangeSchema', () => {
  it('accepts each kind of change', () => {
    expect(
      DataChangeSchema.safeParse({
        kind: 'insert',
        table,
        columns: ['id', 'note'],
        rows: [['1', null]],
      }).success,
    ).toBe(true)
    expect(
      DataChangeSchema.safeParse({
        kind: 'update',
        table,
        set: [{ column: 'status', value: 'shipped' }],
        filters,
      }).success,
    ).toBe(true)
    expect(DataChangeSchema.safeParse({ kind: 'delete', table, filters }).success).toBe(true)
  })

  it('refuses changes the core would refuse', () => {
    // An UPDATE or DELETE must name the rows it changes.
    expect(DataChangeSchema.safeParse({ kind: 'delete', table, filters: [] }).success).toBe(false)
    expect(DataChangeSchema.safeParse({ kind: 'update', table, set: [], filters }).success).toBe(
      false,
    )
    expect(
      DataChangeSchema.safeParse({
        kind: 'insert',
        table,
        columns: ['id'],
        rows: Array.from({ length: DATA_CHANGE_LIMITS.maxInsertRows + 1 }, () => ['1']),
      }).success,
    ).toBe(false)
    expect(DataChangeSchema.safeParse({ kind: 'truncate', table }).success).toBe(false)
  })

  it('sends sets within the core limit', () => {
    const change = { kind: 'delete' as const, table, filters }
    expect(DataChangeSetSchema.safeParse([change]).success).toBe(true)
    expect(DataChangeSetSchema.safeParse([]).success).toBe(false)
    expect(
      DataChangeSetSchema.safeParse(
        Array.from({ length: DATA_CHANGE_LIMITS.maxSetChanges + 1 }, () => change),
      ).success,
    ).toBe(false)
  })
})

describe('DataChangePreviewSchema', () => {
  it('reads a preview as the core sends it', () => {
    const preview = {
      approvalId: '6f9619ff-8b86-4d01-b42d-00c04fc964ff',
      changes: [
        {
          kind: 'update',
          table,
          statement:
            'UPDATE public.orders SET status = CAST($1 AS text) WHERE id = CAST($2 AS int4)',
          rows: 1,
          before: null,
          after: { columns: ['status'], rows: [['shipped']], truncatedCells: [], totalRows: 1 },
        },
      ],
      rows: 1,
      risk: 'R2',
      approval: 'apply',
      reasons: ['Writes 1 row (counted).'],
      confirmation: null,
      expiresAt: '2026-11-19T10:00:00Z',
    }
    expect(DataChangePreviewSchema.parse(preview)).toEqual(preview)
  })

  it('reads every outcome', () => {
    for (const status of ['applied', 'rolled_back', 'uncertain'] as const) {
      expect(
        DataChangeResultSchema.safeParse({ status, rowsAffected: null, message: null }).success,
      ).toBe(true)
    }
  })
})
