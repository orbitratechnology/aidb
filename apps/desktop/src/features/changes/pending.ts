import type { ConnectionId, PendingChange } from '@aidb/contracts'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { listPendingChanges } from '@/lib/commands'

export const PENDING_CHANGES_KEY = ['pendingChanges'] as const

/**
 * Changes waiting for the user's decision, as the core holds them. Re-read now and then, since
 * they expire and connection changes drop them.
 */
export function usePendingChanges(connectionId: ConnectionId | null) {
  return useQuery({
    queryKey: [...PENDING_CHANGES_KEY, connectionId],
    queryFn: () => listPendingChanges(connectionId),
    refetchInterval: 30_000,
  })
}

/** After a change is applied or declined: what waits, the rows shown and the record. */
export function useRefreshAfterDecision() {
  const queryClient = useQueryClient()
  return (change: PendingChange) => {
    void queryClient.invalidateQueries({ queryKey: PENDING_CHANGES_KEY })
    void queryClient.invalidateQueries({ queryKey: ['audit'] })
    for (const { table } of change.changes) {
      void queryClient.invalidateQueries({
        queryKey: ['tablePage', change.connectionId, table.schema, table.name],
      })
    }
  }
}
