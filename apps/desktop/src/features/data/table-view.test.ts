import { describe, expect, it } from 'vitest'
import type { TablePage } from '@aidb/contracts'
import {
  canSort,
  describeFilter,
  formatForDetail,
  operatorsFor,
  rowRange,
  takesValue,
  truncatedKeys,
} from './table-view'

describe('filters', () => {
  it('offers operators that fit the kind of value', () => {
    expect(operatorsFor('number')).toContain('greater')
    expect(operatorsFor('number')).not.toContain('contains')
    expect(operatorsFor('json')).toEqual(['contains', 'starts_with', 'is_null', 'is_not_null'])
    expect(operatorsFor('boolean')).not.toContain('less')
  })

  it('describes filters, with values only where they apply', () => {
    expect(describeFilter({ column: 'email', operator: 'contains', value: '@x' })).toBe(
      'email contains “@x”',
    )
    expect(describeFilter({ column: 'email', operator: 'is_null' })).toBe('email is NULL')
    expect(takesValue('is_not_null')).toBe(false)
  })

  it('does not sort json columns, which have no ordering', () => {
    expect(canSort({ name: 'a', dataType: 'json', kind: 'json' })).toBe(false)
    expect(canSort({ name: 'b', dataType: 'jsonb', kind: 'json' })).toBe(true)
  })
})

describe('formatting', () => {
  it('labels page ranges', () => {
    expect(rowRange(0, 0)).toBe('No rows')
    expect(rowRange(500, 500)).toBe('Rows 501–1,000')
  })

  it('indents complete JSON and leaves cut or invalid values alone', () => {
    expect(formatForDetail('json', '{"a":[1,2]}', false)).toBe('{\n  "a": [\n    1,\n    2\n  ]\n}')
    expect(formatForDetail('json', '{"a":', true)).toBe('{"a":')
    expect(formatForDetail('array', 'not json', false)).toBe('not json')
    expect(formatForDetail('text', '{"a":1}', false)).toBe('{"a":1}')
  })

  it('indexes cut cells by row and column', () => {
    const page = { truncatedCells: [{ row: 2, column: 5 }] } as TablePage
    expect(truncatedKeys(page).has('2:5')).toBe(true)
  })
})
