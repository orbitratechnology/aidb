import { useQuery } from '@tanstack/react-query'
import { listProviders } from '@/lib/commands'

export const PROVIDERS_QUERY_KEY = ['providers'] as const

/** Saved AI provider profiles, ordered by name. */
export function useProviders() {
  return useQuery({ queryKey: PROVIDERS_QUERY_KEY, queryFn: listProviders })
}
