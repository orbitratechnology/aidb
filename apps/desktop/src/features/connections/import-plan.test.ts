import { describe, expect, it } from 'vitest'
import type { ConnectionDraft, ImportCandidate } from '@aidb/contracts'
import { connectionAddress } from './format'
import { planImport } from './import-plan'

const draft = (name: string): ConnectionDraft => ({
  name,
  host: 'db.internal',
  port: 5432,
  database: 'orders',
  username: 'reader',
  sslMode: 'verify-full',
  caCertPath: null,
  connectTimeoutSecs: 10,
  environment: 'development',
  readOnly: true,
})

const candidate = (name: string, overrides: Partial<ImportCandidate> = {}): ImportCandidate => ({
  label: name,
  status: 'ready',
  problem: null,
  connection: draft(name),
  writesDisabled: false,
  unencryptedRemote: false,
  ...overrides,
})

describe('planImport', () => {
  const candidates = [
    candidate('Orders'),
    candidate('Plain', { unencryptedRemote: true }),
    candidate('orders', { status: 'duplicate' }),
    candidate('Broken', { status: 'invalid', connection: null }),
  ]

  it('holds back unencrypted remote profiles until the user allows them', () => {
    const plan = planImport(candidates, false)

    expect(plan.connections.map((c) => c.name)).toEqual(['Orders'])
    expect(plan).toMatchObject({ heldBack: 1, unusable: 2, needsUnencryptedConsent: true })
  })

  it('includes them once allowed', () => {
    const plan = planImport(candidates, true)

    expect(plan.connections.map((c) => c.name)).toEqual(['Orders', 'Plain'])
    expect(plan.heldBack).toBe(0)
  })
})

describe('connectionAddress', () => {
  it('brackets IPv6 hosts like a URI', () => {
    expect(connectionAddress(draft('x'))).toBe('reader@db.internal:5432/orders')
    expect(connectionAddress({ ...draft('x'), host: '2001:db8::7' })).toBe(
      'reader@[2001:db8::7]:5432/orders',
    )
  })
})
