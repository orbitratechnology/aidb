import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { getSettings, updateSettings } from '@/lib/commands'

export const SETTINGS_QUERY_KEY = ['settings'] as const

/** App settings as saved in the local database. */
export function useSettings() {
  return useQuery({ queryKey: SETTINGS_QUERY_KEY, queryFn: getSettings })
}

/** Saves settings sections. Saves run one at a time, so the last change made is the one kept. */
export function useUpdateSettings() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: updateSettings,
    scope: { id: 'settings' },
    onSuccess: (settings) => queryClient.setQueryData(SETTINGS_QUERY_KEY, settings),
  })
}
