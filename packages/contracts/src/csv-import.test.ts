import { describe, expect, it } from 'vitest'
import { CsvImportDraftSchema, CsvImportPreviewSchema, CsvImportRequestSchema } from './csv-import'
import { DataChangeSchema } from './data-change'

describe('CSV import contracts', () => {
  it('reads a draft with the options the core offers', () => {
    const draft = {
      importId: '6f1c2a40-0000-4000-8000-000000000004',
      fileName: 'items.csv',
      options: { encoding: 'windows-1252', delimiter: ';', hasHeader: true },
      columns: ['SKU', 'Name'],
      suggested: ['sku', null],
      sample: [['B2', 'Café mug']],
      totalRows: 1,
      warnings: [],
    }
    expect(CsvImportDraftSchema.parse(draft)).toEqual(draft)
    expect(
      CsvImportDraftSchema.safeParse({ ...draft, options: { ...draft.options, delimiter: ':' } })
        .success,
    ).toBe(false)
  })

  it('sends insert-only by default and an explicit key to update by', () => {
    const mapping = [{ fileColumn: 0, column: 'sku' }]
    expect(CsvImportRequestSchema.safeParse({ mapping }).success).toBe(true)
    expect(CsvImportRequestSchema.safeParse({ mapping, upsertKey: ['sku'] }).success).toBe(true)
    expect(CsvImportRequestSchema.safeParse({ mapping, upsertKey: [] }).success).toBe(false)
    expect(
      DataChangeSchema.safeParse({
        kind: 'insert',
        table: { schema: 'public', name: 'items' },
        columns: ['sku'],
        rows: [['B2']],
        upsertKey: ['sku'],
      }).success,
    ).toBe(true)
  })

  it('reads problems or a ready import', () => {
    expect(
      CsvImportPreviewSchema.parse({ problems: ['Row 3: invalid input syntax.'], ready: null })
        .problems,
    ).toHaveLength(1)
    const ready = {
      approvalId: '6f1c2a40-0000-4000-8000-000000000005',
      rows: 2,
      batches: 1,
      after: { columns: ['sku'], rows: [['B2']], truncatedCells: [], totalRows: 2 },
      statement: 'INSERT INTO public.items …',
      risk: 'R2',
      approval: 'apply',
      reasons: ['Writes 2 rows (counted).'],
      confirmation: null,
      expiresAt: '2026-11-20T10:00:00Z',
    }
    expect(CsvImportPreviewSchema.parse({ problems: [], ready }).ready?.rows).toBe(2)
  })
})
