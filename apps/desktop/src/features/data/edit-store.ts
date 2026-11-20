import type { ConnectionId, ObjectName } from '@aidb/contracts'
import { create } from 'zustand'
import { EMPTY_BUFFER, type EditBuffer } from './edits'

/**
 * Unsaved grid changes, per connection and table, in memory only: they survive paging, filtering
 * and moving between tables, and are gone when the app closes. Nothing here reaches the database
 * until the user reviews and saves.
 */
type EditState = {
  buffers: Record<string, EditBuffer>
  update: (scope: string, change: (buffer: EditBuffer) => EditBuffer) => void
  clear: (scope: string) => void
}

export const useEditStore = create<EditState>((set) => ({
  buffers: {},
  update: (scope, change) =>
    set((state) => ({
      buffers: { ...state.buffers, [scope]: change(state.buffers[scope] ?? EMPTY_BUFFER) },
    })),
  clear: (scope) =>
    set((state) => {
      const buffers = { ...state.buffers }
      delete buffers[scope]
      return { buffers }
    }),
}))

/** The key of one table's changes on one connection. */
export function editScope(connectionId: ConnectionId, object: ObjectName): string {
  return JSON.stringify([connectionId, object.schema, object.name])
}

/** One table's unsaved changes and the ways to change them. */
export function useEditBuffer(scope: string) {
  const buffer = useEditStore((state) => state.buffers[scope] ?? EMPTY_BUFFER)
  const update = useEditStore((state) => state.update)
  const clear = useEditStore((state) => state.clear)
  return {
    buffer,
    update: (change: (buffer: EditBuffer) => EditBuffer) => update(scope, change),
    clear: () => clear(scope),
  }
}
