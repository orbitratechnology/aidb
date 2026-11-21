import type { Column, CsvImportDraft, ObjectDetails } from '@aidb/contracts'
import { describe, expect, it } from 'vitest'
import {
  delimiterLabel,
  importRequest,
  initialMapping,
  INSERT_ONLY,
  keyOptions,
  required,
  unfilled,
  unmappable,
} from './csv-import'

function column(name: string, overrides: Partial<Column> = {}): Column {
  return {
    name,
    position: 1,
    dataType: 'text',
    nullable: true,
    default: null,
    identity: null,
    generated: null,
    comment: null,
    ...overrides,
  }
}

const details = {
  columns: [
    column('id', { nullable: false, identity: 'always' }),
    column('sku', { nullable: false }),
    column('name', { nullable: false, default: "'unnamed'::text" }),
    column('price'),
    column('total', { generated: 'price * 2' }),
  ],
  primaryKey: { name: 'items_pkey', columns: ['id'] },
  uniqueConstraints: [{ name: 'items_sku_key', columns: ['sku'] }],
} as unknown as ObjectDetails

const draft = {
  columns: ['ID', 'SKU', 'Price', 'Total', 'Price again'],
  suggested: ['id', 'sku', 'price', 'total', 'price'],
} as unknown as CsvImportDraft

describe('CSV import mapping', () => {
  it('takes the suggestions the table can accept, first file column first', () => {
    expect(initialMapping(draft, details)).toEqual({ sku: 1, price: 2 })
  })

  it('says which columns the database fills and which every row needs', () => {
    const [id, sku, name, price, total] = details.columns
    expect(unmappable(id!)).toContain('identity')
    expect(unmappable(total!)).toContain('Generated')
    expect(unmappable(sku!)).toBeNull()
    expect([id, sku, name, price].map((c) => required(c!))).toEqual([false, true, false, false])
    expect(unfilled({ price: 2 }, details)).toEqual(['sku'])
    expect(unfilled({ sku: 1 }, details)).toEqual([])
  })

  it('offers the primary key and unique constraints to update by', () => {
    expect(keyOptions(details).map((key) => key.columns)).toEqual([['id'], ['sku']])
  })

  it('adds rows only unless a key is chosen, in table column order', () => {
    const mapping = { price: 2, sku: 1 }
    expect(importRequest(mapping, details, INSERT_ONLY)).toEqual({
      mapping: [
        { fileColumn: 1, column: 'sku' },
        { fileColumn: 2, column: 'price' },
      ],
    })
    expect(importRequest(mapping, details, 'key:items_sku_key').upsertKey).toEqual(['sku'])
  })

  it('names delimiters', () => {
    expect(delimiterLabel(';')).toBe('Semicolon (;)')
    expect(delimiterLabel('\t')).toBe('Tab')
  })
})
