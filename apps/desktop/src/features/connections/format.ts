import type { ConnectionDraft } from '@aidb/contracts'

/** `user@host:port/database`, with IPv6 hosts in brackets as in a URI. */
export function connectionAddress({
  username,
  host,
  port,
  database,
}: Pick<ConnectionDraft, 'username' | 'host' | 'port' | 'database'>): string {
  const shownHost = host.includes(':') ? `[${host}]` : host
  return `${username}@${shownHost}:${port}/${database}`
}

const formatNumber = new Intl.NumberFormat('en-US')

export function pluralize(count: number, one: string, other: string): string {
  return `${formatNumber.format(count)} ${count === 1 ? one : other}`
}
