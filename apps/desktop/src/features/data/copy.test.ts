import { describe, expect, it } from 'vitest'
import type { PageColumn, ValueKind } from '@aidb/contracts'
import { rowAsJson } from './copy'

const column = (name: string, kind: ValueKind): PageColumn => ({ name, dataType: '', kind })

describe('rowAsJson', () => {
  it('keeps numbers exact and embeds JSON, like the native export', () => {
    const text = rowAsJson(
      [
        column('n', 'number'),
        column('ok', 'boolean'),
        column('doc', 'json'),
        column('tags', 'array'),
        column('name', 'text'),
        column('gone', 'text'),
      ],
      ['12345678901234567890.5', 'true', '{"a": 1}', '["x"]', 'say "hi"', null],
    )

    expect(text).toBe(
      [
        '{',
        '  "n": 12345678901234567890.5,',
        '  "ok": true,',
        '  "doc": {"a": 1},',
        '  "tags": ["x"],',
        '  "name": "say \\"hi\\"",',
        '  "gone": null',
        '}',
      ].join('\n'),
    )
  })

  it('writes cut, invalid or non-finite values as strings', () => {
    const text = rowAsJson(
      [column('n', 'number'), column('doc', 'json'), column('cut', 'json')],
      ['NaN', 'not json', '{"a": '],
      (index) => index === 2,
    )

    expect(JSON.parse(text)).toEqual({ n: 'NaN', doc: 'not json', cut: '{"a": ' })
  })
})
