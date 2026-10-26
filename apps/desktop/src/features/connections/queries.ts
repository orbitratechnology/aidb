import { useQuery } from '@tanstack/react-query'
import { listConnections } from '@/lib/commands'

export const CONNECTIONS_QUERY_KEY = ['connections'] as const

/** Saved connection profiles, ordered by name. */
export function useConnections() {
  return useQuery({ queryKey: CONNECTIONS_QUERY_KEY, queryFn: listConnections })
}
