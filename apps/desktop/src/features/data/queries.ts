import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { ConnectionId, ObjectName } from '@aidb/contracts'
import { catalogOverview, describeObject, refreshCatalog } from '@/lib/commands'
import { NativeError } from '@/lib/native'

export const catalogKey = (connectionId: ConnectionId) => ['catalog', connectionId] as const

const objectsKey = (connectionId: ConnectionId) => [...catalogKey(connectionId), 'object'] as const

const objectKey = (connectionId: ConnectionId, { schema, name }: ObjectName) =>
  [...objectsKey(connectionId), schema, name] as const

/** Schemas, tables and views of the open connection. The core caches and revalidates them. */
export function useCatalog(connectionId: ConnectionId) {
  return useQuery({
    queryKey: catalogKey(connectionId),
    queryFn: () => catalogOverview(connectionId),
  })
}

/**
 * Columns, keys, references and indexes of one object, or nothing while none is selected. When
 * the object is gone, the core has already reloaded the schema, so the tree is re-read too.
 */
export function useObjectDetails(connectionId: ConnectionId, object: ObjectName | null) {
  const queryClient = useQueryClient()
  return useQuery({
    queryKey: object ? objectKey(connectionId, object) : objectsKey(connectionId),
    queryFn: async () => {
      try {
        return await describeObject(connectionId, object as ObjectName)
      } catch (error) {
        if (error instanceof NativeError && error.code === 'not_found') {
          void queryClient.invalidateQueries({ queryKey: catalogKey(connectionId), exact: true })
        }
        throw error
      }
    },
    enabled: object !== null,
  })
}

/** Reloads the schema from the database, dropping every object's cached details. */
export function useRefreshCatalog(connectionId: ConnectionId) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: () => refreshCatalog(connectionId),
    onSuccess: async (snapshot) => {
      queryClient.setQueryData(catalogKey(connectionId), snapshot)
      await queryClient.invalidateQueries({ queryKey: objectsKey(connectionId) })
    },
  })
}
