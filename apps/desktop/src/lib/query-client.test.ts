import { describe, expect, it } from 'vitest'
import { NativeError } from './native'
import { shouldRetry } from './query-client'

describe('shouldRetry', () => {
  it('retries temporary native failures a limited number of times', () => {
    const busy = new NativeError('storage', 'The local database is busy.', true)

    expect(shouldRetry(0, busy)).toBe(true)
    expect(shouldRetry(1, busy)).toBe(true)
    expect(shouldRetry(2, busy)).toBe(false)
  })

  it('never retries permanent or unknown failures', () => {
    expect(shouldRetry(0, new NativeError('invalid_input', 'Out of range.'))).toBe(false)
    expect(shouldRetry(0, new Error('boom'))).toBe(false)
  })
})
