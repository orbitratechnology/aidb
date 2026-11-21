import type { ConnectionId, ObjectName } from '@aidb/contracts'
import { create } from 'zustand'
import { askAssistant, cancelAssistantTask } from '@/lib/commands'
import { applyEvent, emptyActivity } from './activity'
import { historyFor, type DraftAttachment, type Turn } from './conversation'

type Draft = {
  question: string
  attachments: DraftAttachment[]
  selected: ObjectName[]
}

type AskState = {
  /** The connection this conversation is about; a different one starts a new conversation. */
  connectionId: ConnectionId | null
  turns: Turn[]
  /** The user allowed sending query results and files to a hosted provider, for this conversation. */
  shareData: boolean
  draft: Draft
  /** The turn being answered. */
  runningTurn: string | null

  /** Starts a new conversation (on another connection, or when asked to). */
  reset: (connectionId: ConnectionId | null) => void
  setShareData: (share: boolean) => void
  setQuestion: (question: string) => void
  addAttachment: (attachment: DraftAttachment) => void
  removeAttachment: (id: string) => void
  toggleTable: (object: ObjectName) => void
  /** Sends the draft as the next question. */
  send: () => Promise<void>
  /** Stops the running question. */
  stop: () => Promise<void>
}

const emptyDraft = (): Draft => ({ question: '', attachments: [], selected: [] })

function sameObject(a: ObjectName, b: ObjectName) {
  return a.schema === b.schema && a.name === b.name
}

/**
 * The conversation on the Ask screen. Client-only and kept in memory: answers and attached
 * files are never written to the webview's storage (finished tasks are saved by the core).
 */
export const useAsk = create<AskState>()((set, get) => {
  const updateTurn = (id: string, change: (turn: Turn) => Turn) =>
    set((state) => ({ turns: state.turns.map((turn) => (turn.id === id ? change(turn) : turn)) }))

  return {
    connectionId: null,
    turns: [],
    shareData: false,
    draft: emptyDraft(),
    runningTurn: null,

    reset: (connectionId) => {
      void get().stop()
      set({ connectionId, turns: [], shareData: false, draft: emptyDraft(), runningTurn: null })
    },
    setShareData: (shareData) => set({ shareData }),
    setQuestion: (question) => set((state) => ({ draft: { ...state.draft, question } })),
    addAttachment: (attachment) =>
      set((state) => ({
        draft: { ...state.draft, attachments: [...state.draft.attachments, attachment] },
      })),
    removeAttachment: (id) =>
      set((state) => ({
        draft: {
          ...state.draft,
          attachments: state.draft.attachments.filter((file) => file.id !== id),
        },
      })),
    toggleTable: (object) =>
      set((state) => {
        const selected = state.draft.selected.some((other) => sameObject(other, object))
          ? state.draft.selected.filter((other) => !sameObject(other, object))
          : [...state.draft.selected, object]
        return { draft: { ...state.draft, selected } }
      }),

    send: async () => {
      const { connectionId, draft, turns, shareData, runningTurn } = get()
      const question = draft.question.trim()
      if (!connectionId || !question || runningTurn) return
      const id = crypto.randomUUID()
      const turn: Turn = {
        id,
        question,
        attachments: draft.attachments.map(({ name, size }) => ({ name, size })),
        selected: draft.selected,
        activity: emptyActivity(),
        state: 'running',
      }
      const request = {
        question,
        selected: draft.selected,
        attachments: draft.attachments.map(({ name, content }) => ({ name, content })),
        history: historyFor(turns),
        shareData,
      }
      set({ turns: [...turns, turn], draft: emptyDraft(), runningTurn: id })
      try {
        const outcome = await askAssistant(connectionId, request, (event) =>
          updateTurn(id, (current) => ({
            ...current,
            activity: applyEvent(current.activity, event),
          })),
        )
        updateTurn(id, (current) => ({ ...current, state: 'done', outcome }))
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error)
        updateTurn(id, (current) => ({ ...current, state: 'error', error: message }))
        // The question was not accepted: give it back so it can be fixed and sent again.
        if (get().connectionId === connectionId && get().draft.question === '') {
          set({ draft: { ...draft, question } })
        }
      } finally {
        if (get().runningTurn === id) set({ runningTurn: null })
      }
    },

    stop: async () => {
      const { runningTurn, turns } = get()
      const taskId = turns.find((turn) => turn.id === runningTurn)?.activity.taskId
      if (taskId) await cancelAssistantTask(taskId).catch(() => false)
    },
  }
})
