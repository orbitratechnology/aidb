import { describe, expect, it } from 'vitest'
import { CompareRequestSchema, SchemaDiffSchema } from './schema-diff'

describe('schema diff contracts', () => {
  it('sends one of three sources', () => {
    const base = { targetSchema: 'public', options: { concurrentIndexes: false } }
    expect(
      CompareRequestSchema.safeParse({ ...base, source: { kind: 'schema', schema: 'staging' } })
        .success,
    ).toBe(true)
    expect(
      CompareRequestSchema.safeParse({
        ...base,
        source: { kind: 'file', fileId: '6f1c2a40-0000-4000-8000-000000000001', schema: 'public' },
      }).success,
    ).toBe(true)
    expect(
      CompareRequestSchema.safeParse({ ...base, source: { kind: 'url', schema: 'x' } }).success,
    ).toBe(false)
    expect(
      CompareRequestSchema.safeParse({
        ...base,
        source: { kind: 'schema', schema: 'é'.repeat(32) },
      }).success,
    ).toBe(false)
  })

  it('reads a comparison with its steps and notes', () => {
    const diff = {
      diffId: '6f1c2a40-0000-4000-8000-000000000002',
      source: 'staging on Dev',
      target: 'public on Dev',
      targetSchema: 'public',
      summary: {
        tablesAdded: 0,
        tablesRemoved: 1,
        tablesChanged: 0,
        viewsAdded: 0,
        viewsRemoved: 0,
        viewsChanged: 0,
      },
      steps: [
        {
          summary: 'Drop table old_log',
          action: 'remove',
          object: 'old_log',
          sql: 'DROP TABLE public.old_log',
          transactional: true,
          risk: 'R4',
          notes: [{ kind: 'data_loss', text: 'Deletes the table and all its rows.' }],
        },
      ],
      script: 'BEGIN;\n…',
      notes: [],
      risk: 'R4',
    }
    expect(SchemaDiffSchema.parse(diff).steps[0]?.notes[0]?.kind).toBe('data_loss')
  })
})
