import { create } from 'zustand'
import type { ActiveConnection } from '@/features/connections/types'

type ConnectionState = {
  /** The connection the workspace is bound to, or `null` before the user connects. */
  active: ActiveConnection | null
  setActive: (connection: ActiveConnection | null) => void
}

export const useConnectionStore = create<ConnectionState>()((set) => ({
  active: null,
  setActive: (active) => set({ active }),
}))
