import { describe, expect, it } from 'vitest'
import { isTheme, resolveTheme } from './theme'

describe('isTheme', () => {
  it('accepts only known theme names', () => {
    expect(isTheme('dark')).toBe(true)
    expect(isTheme('system')).toBe(true)
    expect(isTheme('')).toBe(false)
    expect(isTheme('blue')).toBe(false)
    expect(isTheme(null)).toBe(false)
  })
})

describe('resolveTheme', () => {
  it('follows the OS only for the system theme', () => {
    expect(resolveTheme('system', 'dark')).toBe('dark')
    expect(resolveTheme('system', 'light')).toBe('light')
    expect(resolveTheme('light', 'dark')).toBe('light')
  })
})
