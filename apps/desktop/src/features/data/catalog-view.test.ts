import { describe, expect, it } from 'vitest'
import type { CatalogSnapshot, ObjectDetails, ObjectSummary } from '@aidb/contracts'
import { columnKeys, formatRowEstimate, groupObjects, objectPath } from './catalog-view'

const object = (schema: string, name: string): ObjectSummary => ({
  schema,
  name,
  kind: 'table',
  rowEstimate: null,
  comment: null,
  readable: true,
})

const catalog: CatalogSnapshot = {
  connectionId: '6f1c2a40-0000-4000-8000-000000000000',
  loadedAt: '',
  database: 'shop',
  fingerprint: '',
  schemas: ['empty', 'public', 'sales'].map((name) => ({ name, comment: null, accessible: true })),
  objects: [object('public', 'customers'), object('public', 'orders'), object('sales', 'orders')],
  truncated: false,
}

describe('groupObjects', () => {
  it('lists every schema, empty ones included, when there is no query', () => {
    const groups = groupObjects(catalog, '')

    expect(groups.map((g) => [g.schema.name, g.objects.length])).toEqual([
      ['empty', 0],
      ['public', 2],
      ['sales', 1],
    ])
  })

  it('matches schema-qualified names, ignoring case', () => {
    expect(groupObjects(catalog, 'ORD').map((g) => g.schema.name)).toEqual(['public', 'sales'])
    expect(groupObjects(catalog, 'sales.').flatMap((g) => g.objects.map((o) => o.name))).toEqual([
      'orders',
    ])
    expect(groupObjects(catalog, 'nothing')).toEqual([])
  })
})

describe('formatting', () => {
  it('formats row estimates compactly or precisely', () => {
    expect(formatRowEstimate(null)).toBeNull()
    expect(formatRowEstimate(1)).toBe('≈1 row')
    expect(formatRowEstimate(1234)).toBe('≈1.2K rows')
    expect(formatRowEstimate(1234, { precise: true })).toBe('≈1,234 rows')
  })

  it('encodes object paths', () => {
    expect(objectPath({ schema: 'my schema', name: 'a/b' })).toBe('/data/my%20schema/a%2Fb')
  })
})

describe('columnKeys', () => {
  it('marks primary, unique and foreign key columns', () => {
    const details = {
      primaryKey: { name: 'pk', columns: ['id'] },
      uniqueConstraints: [{ name: 'uq', columns: ['email', 'tenant_id'] }],
      foreignKeys: [{ columns: ['tenant_id'] }],
    } as unknown as ObjectDetails

    const keys = columnKeys(details)

    expect(keys.get('id')).toEqual({ primary: true, unique: false, foreign: false })
    expect(keys.get('tenant_id')).toEqual({ primary: false, unique: true, foreign: true })
    expect(keys.has('name')).toBe(false)
  })
})
