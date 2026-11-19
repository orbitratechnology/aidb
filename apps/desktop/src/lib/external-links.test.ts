import { describe, expect, it } from 'vitest'
import { isExternalHref } from './external-links'

const bundled = { protocol: 'tauri:', host: 'localhost' }
const dev = { protocol: 'http:', host: 'localhost:5173' }

describe('isExternalHref', () => {
  it('treats web and email links as external', () => {
    expect(isExternalHref('https://github.com/orbitratechnology/aidb', bundled)).toBe(true)
    expect(isExternalHref('mailto:team@example.com', bundled)).toBe(true)
    expect(isExternalHref('http://localhost:11434/', dev)).toBe(true)
  })

  it('keeps in-app links inside the app', () => {
    expect(isExternalHref('tauri://localhost/#/settings', bundled)).toBe(false)
    expect(isExternalHref('http://localhost:5173/#/sql', dev)).toBe(false)
  })

  it('ignores schemes the app never opens', () => {
    expect(isExternalHref('file:///etc/passwd', bundled)).toBe(false)
    expect(isExternalHref('javascript:alert(1)', bundled)).toBe(false)
    expect(isExternalHref('not a url', bundled)).toBe(false)
  })
})
