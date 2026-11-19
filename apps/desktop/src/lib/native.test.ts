import { describe, expect, it } from 'vitest'
import { toNativeError } from './native'

describe('toNativeError', () => {
  it('keeps structured errors from the native core', () => {
    const error = toNativeError({ code: 'timeout', message: 'Timed out.', retryable: true })
    expect(error).toMatchObject({ code: 'timeout', message: 'Timed out.', retryable: true })
  })

  it('maps permission and unknown-command strings to internal errors', () => {
    const error = toNativeError('run_sql not allowed')
    expect(error).toMatchObject({
      code: 'internal',
      message: 'run_sql not allowed',
      retryable: false,
    })
  })

  it('never surfaces an empty message', () => {
    expect(toNativeError(undefined).message).toBe('The native core returned an unexpected error.')
  })
})
