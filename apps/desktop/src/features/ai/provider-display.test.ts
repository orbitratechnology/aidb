import type { ProviderDetails } from '@aidb/contracts'
import { describe, expect, it } from 'vitest'
import { isLocalProvider, providerBlocker, providerHost } from './provider-display'

const provider = (patch: Partial<ProviderDetails>): ProviderDetails => ({
  id: '6f1c2a40-0000-4000-8000-000000000000',
  name: 'Claude',
  kind: 'anthropic',
  endpoint: null,
  model: 'claude-opus-5-5',
  requestTimeoutSecs: 120,
  contextBudgetTokens: null,
  allowInsecureHttp: false,
  endpointUrl: 'https://api.anthropic.com/',
  key: 'not_saved',
  keyRequired: true,
  insecureRemote: false,
  features: { toolCalls: true, structuredOutput: 'json_schema' },
  checkedAt: null,
  createdAt: '',
  updatedAt: '',
  ...patch,
})

describe('provider display', () => {
  it('tells local providers from hosted ones', () => {
    expect(isLocalProvider(provider({}))).toBe(false)
    expect(isLocalProvider(provider({ endpointUrl: 'http://localhost:11434/v1' }))).toBe(true)
    expect(providerHost(provider({}))).toBe('api.anthropic.com')
  })

  it('explains what blocks a provider', () => {
    expect(providerBlocker(provider({}))).toBe('Save an API key first.')
    expect(providerBlocker(provider({ key: 'os_store' }))).toBeNull()
    expect(providerBlocker(provider({ keyRequired: false }))).toBeNull()
  })
})
