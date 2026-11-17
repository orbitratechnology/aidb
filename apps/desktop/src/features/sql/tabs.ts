/** One editor tab: a named SQL draft. */
export type EditorTab = { id: string; title: string; sql: string }

export type TabsState = { tabs: EditorTab[]; activeId: string }

/** Most tabs open at once. */
export const MAX_TABS = 20
/** Longest draft kept, in characters (the core refuses more than 1 MB anyway). */
export const MAX_DRAFT_CHARS = 1_000_000

function newId(): string {
  return crypto.randomUUID()
}

/** "Query N" with the lowest number not in use. */
function nextTitle(tabs: EditorTab[]): string {
  const used = new Set(tabs.map((tab) => tab.title))
  let n = 1
  while (used.has(`Query ${n}`)) n += 1
  return `Query ${n}`
}

export function initialTabs(): TabsState {
  const tab = { id: newId(), title: 'Query 1', sql: '' }
  return { tabs: [tab], activeId: tab.id }
}

export function addTab(state: TabsState, sql = ''): TabsState {
  if (state.tabs.length >= MAX_TABS) return state
  const tab = { id: newId(), title: nextTitle(state.tabs), sql: sql.slice(0, MAX_DRAFT_CHARS) }
  return { tabs: [...state.tabs, tab], activeId: tab.id }
}

/** Closes a tab; its right neighbour (or else its left) becomes active. Never leaves none. */
export function closeTab(state: TabsState, id: string): TabsState {
  const index = state.tabs.findIndex((tab) => tab.id === id)
  if (index === -1) return state
  const tabs = state.tabs.filter((tab) => tab.id !== id)
  if (tabs.length === 0) return initialTabs()
  const activeId =
    state.activeId === id
      ? (tabs[Math.min(index, tabs.length - 1)]?.id ?? tabs[0]!.id)
      : state.activeId
  return { tabs, activeId }
}

export function selectTab(state: TabsState, id: string): TabsState {
  return state.tabs.some((tab) => tab.id === id) ? { ...state, activeId: id } : state
}

/** Moves to the next (`+1`) or previous (`-1`) tab, wrapping around. */
export function cycleTab(state: TabsState, step: 1 | -1): TabsState {
  const index = state.tabs.findIndex((tab) => tab.id === state.activeId)
  const next = state.tabs[(index + step + state.tabs.length) % state.tabs.length]
  return next ? { ...state, activeId: next.id } : state
}

export function updateSql(state: TabsState, id: string, sql: string): TabsState {
  return {
    ...state,
    tabs: state.tabs.map((tab) =>
      tab.id === id ? { ...tab, sql: sql.slice(0, MAX_DRAFT_CHARS) } : tab,
    ),
  }
}

/** Accepts only a well-formed stored state; anything else starts afresh. */
export function restoreTabs(stored: unknown): TabsState {
  if (typeof stored !== 'object' || stored === null) return initialTabs()
  const { tabs, activeId } = stored as Partial<TabsState>
  if (!Array.isArray(tabs)) return initialTabs()
  const valid = tabs
    .filter(
      (tab): tab is EditorTab =>
        typeof tab === 'object' &&
        tab !== null &&
        typeof tab.id === 'string' &&
        typeof tab.title === 'string' &&
        typeof tab.sql === 'string',
    )
    .slice(0, MAX_TABS)
    .map((tab) => ({ ...tab, sql: tab.sql.slice(0, MAX_DRAFT_CHARS) }))
  if (valid.length === 0) return initialTabs()
  const active = valid.some((tab) => tab.id === activeId) ? (activeId as string) : valid[0]!.id
  return { tabs: valid, activeId: active }
}
