import { describe, expect, it } from 'vitest'
import { SAFETY_DEFAULTS } from './generated/SafetyLimits'
import { SafetySettingsSchema } from './settings'

function issues(input: unknown) {
  const result = SafetySettingsSchema.safeParse(input)
  return result.success
    ? []
    : result.error.issues.map((issue) => [issue.path.join('.'), issue.message])
}

describe('SafetySettingsSchema', () => {
  it('accepts the native defaults', () => {
    expect(issues(SAFETY_DEFAULTS)).toEqual([])
  })

  it('names the allowed range of an out-of-range value', () => {
    expect(issues({ ...SAFETY_DEFAULTS, maxRowsPerPage: 20_000 })).toEqual([
      ['maxRowsPerPage', 'Must be between 10 and 10,000.'],
    ])
  })

  it('rejects empty and fractional input', () => {
    expect(issues({ ...SAFETY_DEFAULTS, statementTimeoutSecs: Number.NaN })).toEqual([
      ['statementTimeoutSecs', 'Enter a whole number.'],
    ])
    expect(issues({ ...SAFETY_DEFAULTS, maxResultMegabytes: 1.5 })).toEqual([
      ['maxResultMegabytes', 'Enter a whole number.'],
    ])
  })

  it('keeps the lock timeout within the statement timeout', () => {
    expect(issues({ ...SAFETY_DEFAULTS, statementTimeoutSecs: 10, lockTimeoutSecs: 20 })).toEqual([
      ['lockTimeoutSecs', 'Cannot be longer than the statement timeout.'],
    ])
  })
})
