import { describe, expect, it } from 'vitest'
import { isSql, splitAnswer } from './answer'

describe('answer text', () => {
  it('separates prose from fenced code', () => {
    const parts = splitAnswer(
      'There are 12 orders.\n\n```sql\nSELECT count(*)\nFROM orders;\n```\nDone.',
    )
    expect(parts).toEqual([
      { kind: 'text', text: 'There are 12 orders.' },
      { kind: 'code', language: 'sql', code: 'SELECT count(*)\nFROM orders;', complete: true },
      { kind: 'text', text: 'Done.' },
    ])
    expect(isSql(parts[1]!)).toBe(true)
    expect(isSql(parts[0]!)).toBe(false)
  })

  it('keeps an unfinished block while streaming', () => {
    expect(splitAnswer('Running:\n```SQL\nSELECT 1', true)).toEqual([
      { kind: 'text', text: 'Running:' },
      { kind: 'code', language: 'sql', code: 'SELECT 1', complete: false },
    ])
    // A finished answer whose last fence was never closed still ends in a complete block.
    expect(splitAnswer('```sql\nSELECT 1')).toEqual([
      { kind: 'code', language: 'sql', code: 'SELECT 1', complete: true },
    ])
  })

  it('treats markup as text and needs a matching fence to close', () => {
    expect(splitAnswer('<img src=x onerror=alert(1)>')).toEqual([
      { kind: 'text', text: '<img src=x onerror=alert(1)>' },
    ])
    const parts = splitAnswer('````\n```\ninner\n````')
    expect(parts).toEqual([{ kind: 'code', language: '', code: '```\ninner', complete: true }])
    expect(splitAnswer('')).toEqual([])
  })
})
