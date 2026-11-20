import type { ConnectionDetails, MigrationStep, SchemaFile } from '@aidb/contracts'
import { describe, expect, it } from 'vitest'
import {
  compareRequest,
  defaultSchema,
  needsPassword,
  stepsWith,
  summaryParts,
  type CompareForm,
} from './compare'

const ACTIVE = '6f1c2a40-0000-4000-8000-000000000001'
const OTHER = '6f1c2a40-0000-4000-8000-000000000002'

const form: CompareForm = {
  targetSchema: 'public',
  sourceKind: 'schema',
  sourceConnection: ACTIVE,
  sourceSchema: 'staging',
  file: null,
  fileSchema: '',
  concurrentIndexes: false,
}

describe('compare requests', () => {
  it('names a schema here, on another connection, or in a file', () => {
    expect(compareRequest(form, ACTIVE)).toEqual({
      request: {
        targetSchema: 'public',
        source: { kind: 'schema', schema: 'staging' },
        options: { concurrentIndexes: false },
      },
    })
    expect(
      compareRequest({ ...form, sourceConnection: OTHER, sourceSchema: ' public ' }, ACTIVE),
    ).toEqual({
      request: {
        targetSchema: 'public',
        source: { kind: 'connection', connectionId: OTHER, schema: 'public' },
        options: { concurrentIndexes: false },
      },
    })
    const file = { fileId: OTHER, fileName: 'a.sql', schemas: ['app'], statements: 3 } as SchemaFile
    const fromFile = compareRequest(
      { ...form, sourceKind: 'file', file, fileSchema: 'app', concurrentIndexes: true },
      ACTIVE,
    )
    expect(fromFile).toEqual({
      request: {
        targetSchema: 'public',
        source: { kind: 'file', fileId: OTHER, schema: 'app' },
        options: { concurrentIndexes: true },
      },
    })
  })

  it('says what is missing', () => {
    expect(compareRequest({ ...form, sourceSchema: 'public' }, ACTIVE)).toHaveProperty('missing')
    expect(compareRequest({ ...form, sourceKind: 'file' }, ACTIVE)).toEqual({
      missing: 'Choose a .sql file.',
    })
    expect(compareRequest({ ...form, sourceSchema: '  ' }, ACTIVE)).toHaveProperty('missing')
  })

  it('asks for a password only for another connection without one saved', () => {
    const connection = (id: string, password: string) =>
      ({ id, password }) as unknown as ConnectionDetails
    expect(needsPassword(connection(OTHER, 'not_saved'), ACTIVE)).toBe(true)
    expect(needsPassword(connection(OTHER, 'os_store'), ACTIVE)).toBe(false)
    expect(needsPassword(connection(ACTIVE, 'not_saved'), ACTIVE)).toBe(false)
  })

  it('defaults to public, or the first other schema', () => {
    expect(defaultSchema(['app', 'public'])).toBe('public')
    expect(defaultSchema(['app', 'public'], 'public')).toBe('app')
    expect(defaultSchema([])).toBe('')
  })
})

describe('comparison display', () => {
  it('counts what changed', () => {
    expect(
      summaryParts({
        tablesAdded: 1,
        tablesRemoved: 0,
        tablesChanged: 2,
        viewsAdded: 0,
        viewsRemoved: 1,
        viewsChanged: 0,
      }),
    ).toEqual(['1 table added', '2 tables changed', '1 view removed'])
  })

  it('finds the steps with a kind of note', () => {
    const step = (kind: 'data_loss' | 'lock') =>
      ({ summary: kind, notes: [{ kind, text: '' }] }) as unknown as MigrationStep
    expect(stepsWith([step('data_loss'), step('lock')], 'data_loss')).toHaveLength(1)
  })
})
