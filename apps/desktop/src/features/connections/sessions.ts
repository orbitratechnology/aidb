import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { ConnectionId } from '@aidb/contracts'
import { activeSessions, connect, disconnect } from '@/lib/commands'
import { useConnections } from './queries'
import type { ActiveConnection } from './types'

export const SESSIONS_QUERY_KEY = ['sessions'] as const

/** Sessions open in the native core. It keeps at most one: connecting closes the others. */
export function useActiveSessions() {
  return useQuery({ queryKey: SESSIONS_QUERY_KEY, queryFn: activeSessions })
}

/**
 * The connection the workspace is using, as the core reports it, or `null` when none is open.
 * Read-only is what the server enforces, not just what the profile asks for.
 */
export function useActiveConnection(): ActiveConnection | null {
  const sessions = useActiveSessions()
  const connections = useConnections()
  const session = sessions.data?.[0]
  if (!session) return null
  const profile = connections.data?.find((c) => c.id === session.connectionId)
  if (!profile) return null
  return {
    id: profile.id,
    name: profile.name,
    database: session.server.database,
    environment: profile.environment,
    readOnly: session.server.readOnly || session.server.inRecovery,
    encrypted: session.server.encrypted,
    serverVersion: session.server.version,
  }
}

export function useConnect() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, password }: { id: ConnectionId; password?: string }) =>
      connect(id, password),
    scope: { id: 'session' },
    onSettled: () => queryClient.invalidateQueries({ queryKey: SESSIONS_QUERY_KEY }),
  })
}

export function useDisconnect() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: disconnect,
    scope: { id: 'session' },
    onSettled: () => queryClient.invalidateQueries({ queryKey: SESSIONS_QUERY_KEY }),
  })
}
