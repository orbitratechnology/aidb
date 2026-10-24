import { describe, expect, it } from 'vitest'
import { destinationTitle, isActivePath } from './navigation'

describe('isActivePath', () => {
  it('matches the destination and its nested routes', () => {
    expect(isActivePath('/settings', '/settings')).toBe(true)
    expect(isActivePath('/settings/appearance', '/settings')).toBe(true)
  })

  it('does not match a destination that only shares a prefix', () => {
    expect(isActivePath('/sqlx', '/sql')).toBe(false)
    expect(isActivePath('/ask', '/settings')).toBe(false)
  })
})

describe('destinationTitle', () => {
  it('names the top-level destination', () => {
    expect(destinationTitle('/data')).toBe('Data')
    expect(destinationTitle('/settings/diagnostics')).toBe('Settings')
  })

  it('falls back to the app name for unknown paths', () => {
    expect(destinationTitle('/nowhere')).toBe('aidb')
  })
})
