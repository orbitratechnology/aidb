import { describe, expect, it } from 'vitest'
import { filterFormSchema, fromFormValues, NO_SORT, newFilter, toFormValues } from './filter-form'

describe('filter form', () => {
  it('round-trips filters and the first sort key', () => {
    const values = toFormValues(
      [
        { column: 'email', operator: 'contains', value: '@x' },
        { column: 'deleted_at', operator: 'is_null' },
      ],
      [
        { column: 'id', direction: 'desc' },
        { column: 'name', direction: 'asc' },
      ],
    )

    expect(values.sortColumn).toBe('id')
    expect(fromFormValues(values)).toEqual({
      filters: [
        { column: 'email', operator: 'contains', value: '@x' },
        { column: 'deleted_at', operator: 'is_null' },
      ],
      sort: [{ column: 'id', direction: 'desc' }],
    })
  })

  it('needs values for comparisons only', () => {
    const base = { sortColumn: NO_SORT, sortDirection: 'asc' as const }
    const missing = filterFormSchema.safeParse({
      ...base,
      filters: [{ column: 'id', operator: 'equals', value: '' }],
    })
    const nullCheck = filterFormSchema.safeParse({
      ...base,
      filters: [{ column: 'id', operator: 'is_null', value: '' }],
    })

    expect(missing.success).toBe(false)
    expect(missing.error?.issues[0]?.path).toEqual(['filters', 0, 'value'])
    expect(nullCheck.success).toBe(true)
    expect(fromFormValues({ ...base, filters: [] }).sort).toEqual([])
  })

  it('starts new filters with an operator that suits the column', () => {
    expect(newFilter([{ name: 'doc', dataType: 'jsonb', kind: 'json' }])).toEqual({
      column: 'doc',
      operator: 'contains',
      value: '',
    })
  })
})
