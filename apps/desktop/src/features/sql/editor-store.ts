import { create } from 'zustand'
import { createJSONStorage, persist, type StateStorage } from 'zustand/middleware'
import {
  addTab,
  closeTab,
  cycleTab,
  initialTabs,
  restoreTabs,
  selectTab,
  updateSql,
  type TabsState,
} from './tabs'

type Actions = {
  add: (sql?: string) => void
  close: (id: string) => void
  select: (id: string) => void
  cycle: (step: 1 | -1) => void
  update: (id: string, sql: string) => void
  /** Closes every tab and forgets all drafts, on disk too. */
  clearDrafts: () => void
}

const STORAGE_KEY = 'aidb.sql-editor.tabs'
const WRITE_DELAY_MS = 500

/**
 * `localStorage`, with writes delayed while typing so a draft is not re-serialized on every key.
 * Pending writes are flushed when the window is hidden or closed.
 */
function debouncedLocalStorage(): StateStorage {
  let pending: { name: string; value: string } | null = null
  let timer: ReturnType<typeof setTimeout> | undefined
  const flush = () => {
    clearTimeout(timer)
    if (pending) {
      try {
        localStorage.setItem(pending.name, pending.value)
      } catch {
        // Storage full or unavailable: the drafts stay in memory for this session.
      }
      pending = null
    }
  }
  window.addEventListener('pagehide', flush)
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') flush()
  })
  return {
    getItem: (name) => localStorage.getItem(name),
    setItem: (name, value) => {
      pending = { name, value }
      clearTimeout(timer)
      timer = setTimeout(flush, WRITE_DELAY_MS)
    },
    removeItem: (name) => {
      pending = null
      clearTimeout(timer)
      localStorage.removeItem(name)
    },
  }
}

/**
 * Open SQL editor tabs and their drafts. Client-only UI state, kept in the webview's local
 * storage so drafts survive restarts; results are never stored.
 */
export const useEditorTabs = create<TabsState & Actions>()(
  persist(
    (set) => ({
      ...initialTabs(),
      add: (sql) => set((state) => addTab(state, sql)),
      close: (id) => set((state) => closeTab(state, id)),
      select: (id) => set((state) => selectTab(state, id)),
      cycle: (step) => set((state) => cycleTab(state, step)),
      update: (id, sql) => set((state) => updateSql(state, id, sql)),
      clearDrafts: () => {
        useEditorTabs.persist.clearStorage()
        set(initialTabs())
      },
    }),
    {
      name: STORAGE_KEY,
      version: 1,
      storage: createJSONStorage(debouncedLocalStorage),
      partialize: ({ tabs, activeId }) => ({ tabs, activeId }),
      merge: (stored, current) => ({ ...current, ...restoreTabs(stored) }),
    },
  ),
)
