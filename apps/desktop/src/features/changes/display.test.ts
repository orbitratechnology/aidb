import type { AuditEntry } from '@aidb/contracts'
import { describe, expect, it } from 'vitest'
import { nextBefore, operationLabel, PAGE_SIZE, shortFingerprint } from './display'

describe('changes display', () => {
  it('names operations, including ones from later versions', () => {
    expect(operationLabel('approve')).toBe('Approved a change')
    expect(operationLabel('data_import')).toBe('Data import')
    expect(operationLabel('')).toBe('Operation')
  })

  it('shortens fingerprints', () => {
    expect(shortFingerprint('ab'.repeat(32))).toBe('abababababab')
  })

  it('pages through the history by id', () => {
    const entry = (id: number) => ({ id }) as AuditEntry
    const full = Array.from({ length: PAGE_SIZE }, (_, i) => entry(1000 - i))
    expect(nextBefore(full)).toBe(1000 - PAGE_SIZE + 1)
    expect(nextBefore(full.slice(0, 3))).toBeUndefined()
    expect(nextBefore([])).toBeUndefined()
  })
})
