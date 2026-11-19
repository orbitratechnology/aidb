import { z } from 'zod'
import {
  CONNECTION_LIMITS,
  EnvironmentSchema,
  SslModeSchema,
  type ConnectionDetails,
  type ConnectionDraft,
  type ParsedConnectionUri,
  type SslMode,
} from '@aidb/contracts'

// The same rules as `ConnectionDraft::validated` in the native core, which checks again on save.
// These give immediate feedback; the core's answer is the one that counts.

const VERIFYING_MODES: readonly SslMode[] = ['verify-ca', 'verify-full']

function hasControlCharacters(text: string): boolean {
  for (const char of text) {
    const code = char.codePointAt(0) ?? 0
    if (code < 0x20 || (code >= 0x7f && code <= 0x9f)) return true
  }
  return false
}

function byteLength(text: string): number {
  return new TextEncoder().encode(text).length
}

function unbracketed(host: string): string {
  const trimmed = host.trim()
  return trimmed.startsWith('[') && trimmed.endsWith(']') ? trimmed.slice(1, -1) : trimmed
}

const IPV4 = z.ipv4()
const IPV6 = z.ipv6()
const HOST_LABEL = new RegExp(`^(?!-)[A-Za-z0-9_-]{1,${CONNECTION_LIMITS.hostLabelMax}}(?<!-)$`)

/** An IP address, or a DNS name of letters, digits, hyphens and underscores. */
export function isValidHost(host: string): boolean {
  const value = unbracketed(host)
  if (IPV4.safeParse(value).success || IPV6.safeParse(value).success) return true
  return value.length > 0 && value.split('.').every((label) => HOST_LABEL.test(label))
}

/** Whether `host` names this machine. */
export function isLoopbackHost(host: string): boolean {
  const value = unbracketed(host).replace(/\.$/, '').toLowerCase()
  return (
    value === 'localhost' ||
    value.endsWith('.localhost') ||
    value === '::1' ||
    (IPV4.safeParse(value).success && value.startsWith('127.'))
  )
}

/** Absolute on Unix (`/…`) or Windows (`C:\…`, `\\server\…`). */
export function isAbsolutePath(path: string): boolean {
  return /^\//.test(path) || /^[A-Za-z]:[\\/]/.test(path) || /^\\\\/.test(path)
}

const text = (label: string, max: number) =>
  z
    .string()
    .trim()
    .min(1, { error: `Enter a ${label}.` })
    .max(max, { error: `Use at most ${max} characters.` })
    .refine((value) => !hasControlCharacters(value), {
      error: 'Remove control characters.',
    })

const identifier = (label: string) =>
  z
    .string()
    .min(1, { error: `Enter a ${label}.` })
    .refine((value) => byteLength(value) <= CONNECTION_LIMITS.identifierMaxBytes, {
      error: `PostgreSQL allows at most ${CONNECTION_LIMITS.identifierMaxBytes} bytes.`,
    })
    .refine((value) => !hasControlCharacters(value), { error: 'Remove control characters.' })

const wholeNumber = (min: number, max: number) =>
  z
    .number({ error: 'Enter a whole number.' })
    .int({ error: 'Enter a whole number.' })
    .min(min, { error: `Must be between ${min} and ${max}.` })
    .max(max, { error: `Must be between ${min} and ${max}.` })

export const ConnectionFormSchema = z
  .object({
    name: text('name', CONNECTION_LIMITS.nameMax),
    host: z
      .string()
      .trim()
      .min(1, { error: 'Enter a host.' })
      .max(CONNECTION_LIMITS.hostMax, {
        error: `Use at most ${CONNECTION_LIMITS.hostMax} characters.`,
      })
      .refine(isValidHost, { error: 'Enter a host name or an IP address.' }),
    port: wholeNumber(1, 65_535),
    database: identifier('database'),
    username: identifier('user name'),
    password: z.string(),
    rememberPassword: z.boolean(),
    sslMode: SslModeSchema,
    caCertPath: z.string().trim().max(CONNECTION_LIMITS.caCertPathMax),
    connectTimeoutSecs: wholeNumber(
      CONNECTION_LIMITS.connectTimeoutSecs.min,
      CONNECTION_LIMITS.connectTimeoutSecs.max,
    ),
    environment: EnvironmentSchema,
    readOnly: z.boolean(),
    allowUnencrypted: z.boolean(),
  })
  .superRefine((values, context) => {
    if (values.caCertPath && VERIFYING_MODES.includes(values.sslMode)) {
      if (!isAbsolutePath(values.caCertPath)) {
        context.addIssue({
          code: 'custom',
          path: ['caCertPath'],
          message: 'Enter the full path to the certificate file.',
        })
      }
    }
    if (needsUnencryptedConsent(values) && !values.allowUnencrypted) {
      context.addIssue({
        code: 'custom',
        path: ['allowUnencrypted'],
        message: 'Confirm that the password and data may cross the network unencrypted.',
      })
    }
  })

export type ConnectionFormValues = z.infer<typeof ConnectionFormSchema>

/** TLS is off for a host other than this machine. */
export function needsUnencryptedConsent(values: Pick<ConnectionFormValues, 'sslMode' | 'host'>) {
  return values.sslMode === 'disable' && values.host.trim() !== '' && !isLoopbackHost(values.host)
}

/** Whether the CA certificate path applies to this TLS mode. */
export function usesCaCertificate(sslMode: SslMode): boolean {
  return VERIFYING_MODES.includes(sslMode)
}

export function emptyFormValues(): ConnectionFormValues {
  return {
    name: '',
    host: '',
    port: CONNECTION_LIMITS.defaultPort,
    database: '',
    username: '',
    password: '',
    rememberPassword: true,
    sslMode: 'verify-full',
    caCertPath: '',
    connectTimeoutSecs: CONNECTION_LIMITS.defaultConnectTimeoutSecs,
    environment: 'development',
    readOnly: true,
    allowUnencrypted: false,
  }
}

/** Form values for editing a saved profile. The password field starts empty: blank keeps it. */
export function formValuesFrom(connection: ConnectionDetails): ConnectionFormValues {
  return {
    ...emptyFormValues(),
    name: connection.name,
    host: connection.host,
    port: connection.port,
    database: connection.database,
    username: connection.username,
    sslMode: connection.sslMode,
    caCertPath: connection.caCertPath ?? '',
    connectTimeoutSecs: connection.connectTimeoutSecs,
    environment: connection.environment,
    readOnly: connection.readOnly,
    rememberPassword: connection.password !== 'not_saved',
  }
}

/** The profile to send to the core; never includes the password. */
export function toDraft(values: ConnectionFormValues): ConnectionDraft {
  return {
    name: values.name.trim(),
    host: values.host.trim(),
    port: values.port,
    database: values.database,
    username: values.username,
    sslMode: values.sslMode,
    caCertPath: usesCaCertificate(values.sslMode) && values.caCertPath ? values.caCertPath : null,
    connectTimeoutSecs: values.connectTimeoutSecs,
    environment: values.environment,
    readOnly: values.readOnly,
  }
}

type UriFields = keyof Pick<
  ConnectionFormValues,
  | 'name'
  | 'host'
  | 'port'
  | 'database'
  | 'username'
  | 'password'
  | 'sslMode'
  | 'caCertPath'
  | 'connectTimeoutSecs'
>

/**
 * Fills the form from a parsed URI. Fields the URI leaves out keep their values; the name is
 * filled only when it is empty.
 */
export function applyParsedUri(
  values: ConnectionFormValues,
  parsed: ParsedConnectionUri,
): { values: ConnectionFormValues; filled: UriFields[] } {
  const next: ConnectionFormValues = { ...values, host: parsed.host, port: parsed.port }
  const filled: UriFields[] = ['host', 'port']
  const set = <K extends UriFields>(field: K, value: ConnectionFormValues[K] | null) => {
    if (value === null) return
    next[field] = value
    filled.push(field)
  }
  set('database', parsed.database)
  set('username', parsed.username)
  set('password', parsed.password)
  set('sslMode', parsed.sslMode)
  set('caCertPath', parsed.caCertPath)
  set('connectTimeoutSecs', parsed.connectTimeoutSecs)
  if (!values.name.trim()) set('name', parsed.suggestedName.slice(0, CONNECTION_LIMITS.nameMax))
  return { values: next, filled }
}
