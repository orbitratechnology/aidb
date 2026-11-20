import type { DataChangePreview, ObjectDetails, PageColumn } from '@aidb/contracts'
import { DATA_CHANGE_LIMITS, TABLE_DATA_LIMITS } from '@aidb/contracts'
import { describe, expect, it } from 'vitest'
import {
  bufferProblem,
  deleteRow,
  describeCounts,
  editing,
  editRow,
  EMPTY_BUFFER,
  failedChange,
  keyId,
  pendingCounts,
  proposalItems,
  putNewRow,
  removeNewRow,
  reviewItems,
  revertRow,
  rowKey,
  rowValues,
  staleChanges,
  toChanges,
  type EditBuffer,
} from './edits'

const table = { schema: 'public', name: 'items' }

const columns: PageColumn[] = [
  { name: 'id', dataType: 'integer', kind: 'number' },
  { name: 'status', dataType: 'text', kind: 'text' },
  { name: 'doc', dataType: 'jsonb', kind: 'json' },
  { name: 'tags', dataType: 'integer[]', kind: 'array' },
  { name: 'note', dataType: 'text', kind: 'text' },
]

const page = {
  columns,
  rows: [
    ['1', 'new', '{"a": 1}', '[1]', null],
    ['2', 'old', null, null, 'long…'],
    [null, 'orphan', null, null, null],
  ],
}

function details(overrides: Partial<ObjectDetails> = {}): ObjectDetails {
  return {
    summary: {
      schema: 'public',
      name: 'items',
      kind: 'table',
      rowEstimate: 3,
      comment: null,
      readable: true,
    },
    columns: [
      {
        name: 'id',
        position: 1,
        dataType: 'integer',
        nullable: false,
        default: null,
        identity: null,
        generated: null,
        comment: null,
      },
      {
        name: 'total',
        position: 2,
        dataType: 'numeric',
        nullable: true,
        default: null,
        identity: null,
        generated: 'price * 2',
        comment: null,
      },
      {
        name: 'serial',
        position: 3,
        dataType: 'integer',
        nullable: false,
        default: null,
        identity: 'always',
        generated: null,
        comment: null,
      },
    ],
    primaryKey: { name: 'items_pkey', columns: ['id'] },
    uniqueConstraints: [],
    foreignKeys: [],
    referencedBy: [],
    indexes: [],
    definition: null,
    ...overrides,
  }
}

describe('editing', () => {
  it('allows writable tables with a primary key, and marks what the database fills in', () => {
    const result = editing({ details: details(), columns, readOnly: false })
    expect(result.blocked).toBeNull()
    expect(result.rowsBlocked).toBeNull()
    expect(result.keyColumns).toEqual(['id'])
    expect(Object.keys(result.readOnlyColumns).sort()).toEqual(['serial', 'tags', 'total'])
  })

  it('explains why rows cannot be changed', () => {
    expect(editing({ details: details(), columns, readOnly: true }).blocked).toMatch(/read-only/)
    const view = details({ summary: { ...details().summary, kind: 'view' } })
    expect(editing({ details: view, columns, readOnly: false }).blocked).toMatch(/a view/)
    const noKey = editing({ details: details({ primaryKey: null }), columns, readOnly: false })
    expect(noKey.blocked).toBeNull()
    expect(noKey.rowsBlocked).toMatch(/no primary key/)
    const hiddenKey = editing({
      details: details(),
      columns: columns.filter((c) => c.name !== 'id'),
      readOnly: false,
    })
    expect(hiddenKey.rowsBlocked).toMatch(/cannot read the whole primary key/)
  })
})

describe('rows', () => {
  it('are addressed by a complete, uncut primary key', () => {
    const none = new Set<string>()
    expect(rowKey(page, 0, ['id'], none)).toEqual([{ column: 'id', value: '1' }])
    expect(rowKey(page, 2, ['id'], none)).toBeNull()
    expect(rowKey(page, 0, ['id'], new Set(['0:0']))).toBeNull()
    expect(rowKey(page, 0, [], none)).toBeNull()
  })

  it('leave cut cells out of their values', () => {
    expect(rowValues(page, 1, new Set(['1:4']))).toEqual({
      id: '2',
      status: 'old',
      doc: null,
      tags: null,
    })
  })
})

describe('the buffer', () => {
  const key = [{ column: 'id', value: '1' }]
  const original = rowValues(page, 0, new Set())

  it('keeps only real changes', () => {
    let buffer = editRow(EMPTY_BUFFER, key, original, { status: 'sold', note: null })
    expect(buffer.edits[keyId(key)]?.values).toEqual({ status: 'sold' })
    buffer = editRow(buffer, key, original, { status: 'new' })
    expect(buffer.edits).toEqual({})
  })

  it('lets a deletion replace edits, and undoes either', () => {
    let buffer = editRow(EMPTY_BUFFER, key, original, { status: 'sold' })
    buffer = deleteRow(buffer, key, original)
    expect(pendingCounts(buffer)).toEqual({ edited: 0, deleted: 1, added: 0, total: 1 })
    buffer = revertRow(buffer, keyId(key))
    expect(pendingCounts(buffer).total).toBe(0)
  })

  it('adds, replaces and removes new rows', () => {
    let buffer = putNewRow(EMPTY_BUFFER, { id: 'a', values: { id: '9' } })
    buffer = putNewRow(buffer, { id: 'a', values: { id: '10' } })
    expect(buffer.inserts).toEqual([{ id: 'a', values: { id: '10' } }])
    expect(removeNewRow(buffer, 'a').inserts).toEqual([])
  })

  it('says what is pending and what blocks saving', () => {
    expect(describeCounts({ edited: 2, deleted: 1, added: 1, total: 4 })).toBe(
      '2 edited rows, 1 deleted, 1 new',
    )
    expect(describeCounts({ edited: 1, deleted: 0, added: 0, total: 1 })).toBe('1 edited row')
    expect(bufferProblem(putNewRow(EMPTY_BUFFER, { id: 'a', values: {} }))).toMatch(
      /at least one value/,
    )
    let many: EditBuffer = EMPTY_BUFFER
    for (let i = 0; i <= DATA_CHANGE_LIMITS.maxSetChanges; i++) {
      many = putNewRow(many, { id: String(i), values: { id: String(i) } })
    }
    expect(bufferProblem(many)).toMatch(/at most/)
  })
})

describe('toChanges', () => {
  it('orders deletions, edits and new rows, each checking the values it was based on', () => {
    let buffer = editRow(
      EMPTY_BUFFER,
      [{ column: 'id', value: '1' }],
      rowValues(page, 0, new Set()),
      {
        status: 'sold',
        doc: '{"a": 2}',
      },
    )
    buffer = deleteRow(buffer, [{ column: 'id', value: '2' }], rowValues(page, 1, new Set(['1:4'])))
    buffer = putNewRow(buffer, { id: 'n', values: { id: '9', note: null } })

    expect(toChanges(buffer, table, columns)).toEqual([
      {
        kind: 'delete',
        table,
        // Key, then the loaded values that compare exactly; JSON, arrays and cut cells do not.
        filters: [
          { column: 'id', operator: 'equals', value: '2' },
          { column: 'status', operator: 'equals', value: 'old' },
        ],
      },
      {
        kind: 'update',
        table,
        set: [
          { column: 'status', value: 'sold' },
          { column: 'doc', value: '{"a": 2}' },
        ],
        filters: [
          { column: 'id', operator: 'equals', value: '1' },
          { column: 'status', operator: 'equals', value: 'new' },
        ],
      },
      { kind: 'insert', table, columns: ['id', 'note'], rows: [['9', null]] },
    ])
  })

  it('checks NULLs with IS NULL and stays within the filter limit', () => {
    const wide: PageColumn[] = Array.from({ length: 30 }, (_, i) => ({
      name: `c${i}`,
      dataType: 'text',
      kind: 'text',
    }))
    const values = Object.fromEntries(wide.map((c) => [c.name, null]))
    const buffer = deleteRow(EMPTY_BUFFER, [{ column: 'c0', value: 'k' }], values)
    const [change] = toChanges(buffer, table, wide)
    expect(change?.kind).toBe('delete')
    const filters = change?.kind === 'delete' ? change.filters : []
    expect(filters).toHaveLength(TABLE_DATA_LIMITS.maxFilters)
    expect(filters[1]).toEqual({ column: 'c1', operator: 'is_null' })
  })
})

describe('the review', () => {
  it('shows each change before and after, in the order they run', () => {
    let buffer = editRow(
      EMPTY_BUFFER,
      [{ column: 'id', value: '1' }],
      rowValues(page, 0, new Set()),
      {
        status: 'sold',
      },
    )
    buffer = deleteRow(buffer, [{ column: 'id', value: '2' }], { id: '2', status: 'old' })
    buffer = putNewRow(buffer, { id: 'n', values: { id: '9' } })
    expect(reviewItems(buffer, columns)).toEqual([
      {
        kind: 'delete',
        id: keyId([{ column: 'id', value: '2' }]),
        label: 'id = 2',
        values: [
          { column: 'id', value: '2' },
          { column: 'status', value: 'old' },
        ],
      },
      {
        kind: 'update',
        id: keyId([{ column: 'id', value: '1' }]),
        label: 'id = 1',
        cells: [{ column: 'status', before: 'new', after: 'sold' }],
      },
      {
        kind: 'insert',
        id: 'n',
        label: 'New row',
        values: [{ column: 'id', value: '9' }],
        defaults: ['status', 'doc', 'tags', 'note'],
      },
    ])
  })

  it('flags rows that changed since they were loaded, and errors that name a change', () => {
    const buffer = putNewRow(deleteRow(EMPTY_BUFFER, [{ column: 'id', value: '2' }], { id: '2' }), {
      id: 'n',
      values: { id: '9' },
    })
    const changes = toChanges(buffer, table, columns)
    const counted = (rows: number[]) =>
      ({ changes: rows.map((r) => ({ rows: r })) }) as unknown as DataChangePreview
    expect(staleChanges(changes, counted([1, 1]))).toEqual([])
    expect(staleChanges(changes, counted([0, 1]))).toEqual([0])
    expect(failedChange('Change 2 of 3: invalid input syntax for type integer.')).toBe(1)
    expect(failedChange('The connection is read-only.')).toBeNull()
  })
})

describe('proposalItems', () => {
  it('says what each of the assistant’s changes targets and sets', () => {
    const filters = [{ column: 'id', operator: 'equals' as const, value: '7' }]
    expect(
      proposalItems([
        { kind: 'update', table, set: [{ column: 'status', value: 'sold' }], filters },
        { kind: 'delete', table, filters },
        { kind: 'insert', table, columns: ['id'], rows: [['8'], ['9']] },
      ]),
    ).toEqual([
      {
        kind: 'update',
        id: '0',
        label: 'public.items where id equals “7”',
        cells: [{ column: 'status', after: 'sold' }],
      },
      { kind: 'delete', id: '1', label: 'public.items where id equals “7”', values: [] },
      { kind: 'insert', id: '2', label: 'public.items: 2 new rows', values: [], defaults: [] },
    ])
  })
})
