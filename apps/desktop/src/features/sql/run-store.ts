import type { ConnectionId, QueryRun } from '@aidb/contracts'
import { create } from 'zustand'

/** A line and column in the draft, from 1. */
export type DraftPosition = { line: number; column: number }

/** A run of one tab. Positions in `outcome` are within the text that ran; these are in the draft. */
export type TabRun =
  | { status: 'running'; connectionId: ConnectionId; operationId: string }
  | {
      status: 'done'
      connectionId: ConnectionId
      outcome: QueryRun
      /** Where the text that ran starts in the draft. */
      start: DraftPosition
      /** Where the reported problem is in the draft, if any. */
      problemAt: DraftPosition | null
    }

type RunStore = {
  runs: Record<string, TabRun>
  set: (tabId: string, run: TabRun) => void
  forget: (tabId: string) => void
}

/** The latest run of each tab, for this session only: results are never written to disk. */
export const useTabRuns = create<RunStore>()((set) => ({
  runs: {},
  set: (tabId, run) => set((state) => ({ runs: { ...state.runs, [tabId]: run } })),
  forget: (tabId) =>
    set((state) => {
      const { [tabId]: _, ...rest } = state.runs
      return { runs: rest }
    }),
}))
