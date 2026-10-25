import { describe, expect, it } from 'vitest'
import { ErrorPayloadSchema } from './errors'

describe('ErrorPayloadSchema', () => {
  it('accepts the payload the native core sends', () => {
    const payload = {
      code: 'unavailable',
      message: 'The database did not respond.',
      retryable: true,
    }
    expect(ErrorPayloadSchema.parse(payload)).toEqual(payload)
  })

  it('rejects unknown codes and plain strings', () => {
    expect(
      ErrorPayloadSchema.safeParse({ code: 'oops', message: '', retryable: false }).success,
    ).toBe(false)
    expect(ErrorPayloadSchema.safeParse('app_info not allowed').success).toBe(false)
  })
})
