import { describe, expect, it } from 'vitest'
import type { ConnectionDetails, ParsedConnectionUri } from '@aidb/contracts'
import {
  ConnectionFormSchema,
  applyParsedUri,
  emptyFormValues,
  formValuesFrom,
  isLoopbackHost,
  isValidHost,
  toDraft,
  type ConnectionFormValues,
} from './form'

const valid = (overrides: Partial<ConnectionFormValues> = {}): ConnectionFormValues => ({
  ...emptyFormValues(),
  name: 'Orders',
  host: 'db.internal',
  database: 'orders',
  username: 'reader',
  ...overrides,
})

function issues(values: ConnectionFormValues) {
  const result = ConnectionFormSchema.safeParse(values)
  return result.success ? [] : result.error.issues.map((issue) => issue.path.join('.'))
}

describe('ConnectionFormSchema', () => {
  it('accepts a complete profile with safe defaults', () => {
    expect(issues(valid())).toEqual([])
    expect(emptyFormValues()).toMatchObject({ sslMode: 'verify-full', readOnly: true })
  })

  it('rejects the same fields as the native core', () => {
    expect(issues(valid({ host: 'db internal' }))).toEqual(['host'])
    expect(issues(valid({ port: 0 }))).toEqual(['port'])
    expect(issues(valid({ database: 'x'.repeat(64) }))).toEqual(['database'])
    expect(issues(valid({ database: 'é'.repeat(32) }))).toEqual(['database'])
    expect(issues(valid({ username: 'app\nreader' }))).toEqual(['username'])
    expect(issues(valid({ connectTimeoutSecs: 301 }))).toEqual(['connectTimeoutSecs'])
  })

  it('requires an absolute CA path for verifying modes only', () => {
    expect(issues(valid({ caCertPath: 'certs/ca.pem' }))).toEqual(['caCertPath'])
    expect(issues(valid({ caCertPath: '/etc/ssl/ca.pem' }))).toEqual([])
    expect(issues(valid({ caCertPath: 'C:\\certs\\ca.pem' }))).toEqual([])
    expect(issues(valid({ sslMode: 'require', caCertPath: 'ignored' }))).toEqual([])
  })

  it('needs consent before TLS is turned off for another machine', () => {
    expect(issues(valid({ sslMode: 'disable' }))).toEqual(['allowUnencrypted'])
    expect(issues(valid({ sslMode: 'disable', allowUnencrypted: true }))).toEqual([])
    expect(issues(valid({ sslMode: 'disable', host: 'localhost' }))).toEqual([])
  })
})

describe('hosts', () => {
  it('accepts names and addresses', () => {
    for (const host of ['db.internal', 'pg_primary', '10.0.0.5', '::1', '[2001:db8::10]']) {
      expect(isValidHost(host), host).toBe(true)
    }
    for (const host of ['db/primary', 'user@db', '-db', 'db..internal', '']) {
      expect(isValidHost(host), host).toBe(false)
    }
  })

  it('recognizes this machine', () => {
    for (const host of ['localhost', 'db.localhost', '127.0.0.1', '127.8.0.1', '::1', '[::1]']) {
      expect(isLoopbackHost(host), host).toBe(true)
    }
    expect(isLoopbackHost('10.0.0.1')).toBe(false)
  })
})

describe('conversions', () => {
  it('builds a draft without the password and drops an unused CA path', () => {
    const draft = toDraft(
      valid({ name: ' Orders ', password: 'secret', sslMode: 'require', caCertPath: '/ca.pem' }),
    )

    expect(draft.name).toBe('Orders')
    expect(draft.caCertPath).toBeNull()
    expect(Object.keys(draft)).not.toContain('password')
  })

  it('starts editing with an empty password field', () => {
    const saved = {
      ...toDraft(valid()),
      id: '6f1c2a40-0000-4000-8000-000000000000',
      engine: 'postgres',
      password: 'os_store',
      unencryptedRemote: false,
      createdAt: '',
      updatedAt: '',
    } satisfies ConnectionDetails

    expect(formValuesFrom(saved)).toMatchObject({ password: '', rememberPassword: true })
  })

  it('fills only the fields a URI provides', () => {
    const parsed: ParsedConnectionUri = {
      host: 'pg.example.com',
      port: 6432,
      database: 'sales',
      username: null,
      sslMode: 'require',
      caCertPath: null,
      connectTimeoutSecs: null,
      password: 'pw',
      suggestedName: 'sales on pg.example.com',
      notes: [],
    }

    const named = applyParsedUri(valid(), parsed)
    const unnamed = applyParsedUri(valid({ name: '' }), parsed)

    expect(named.values).toMatchObject({
      name: 'Orders',
      host: 'pg.example.com',
      port: 6432,
      username: 'reader',
      password: 'pw',
    })
    expect(named.filled).toEqual(['host', 'port', 'database', 'password', 'sslMode'])
    expect(unnamed.values.name).toBe('sales on pg.example.com')
  })
})
