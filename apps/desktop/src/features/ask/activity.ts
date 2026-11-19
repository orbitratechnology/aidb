import type { ContextItem, PlanStep, TaskEvent, TaskId } from '@aidb/contracts'

/** One line of what the assistant did, in the order it happened. */
export type ActivityEntry =
  | { kind: 'note'; round: number; text: string }
  | { kind: 'step'; index: number; step: PlanStep }
  | { kind: 'rejected'; tool: string; reason: string }

/** A task's progress, built from the events the core streams. */
export type Activity = {
  taskId: TaskId | null
  /** What was sent as context. */
  items: ContextItem[]
  /** The model call in progress, from 0. */
  round: number
  entries: ActivityEntry[]
}

export function emptyActivity(): Activity {
  return { taskId: null, items: [], round: 0, entries: [] }
}

/** Adds one event. Text of one model call is one note; a step is updated in place. */
export function applyEvent(activity: Activity, event: TaskEvent): Activity {
  switch (event.event) {
    case 'started':
      return { ...activity, taskId: event.taskId, items: event.items }
    case 'round_started':
      return { ...activity, round: event.round }
    case 'text': {
      const last = activity.entries.at(-1)
      if (last?.kind === 'note' && last.round === activity.round) {
        const entries = activity.entries.slice(0, -1)
        entries.push({ ...last, text: last.text + event.text })
        return { ...activity, entries }
      }
      return {
        ...activity,
        entries: [...activity.entries, { kind: 'note', round: activity.round, text: event.text }],
      }
    }
    case 'step_started':
    case 'step_finished': {
      const at = activity.entries.findIndex(
        (entry) => entry.kind === 'step' && entry.index === event.index,
      )
      const entry: ActivityEntry = { kind: 'step', index: event.index, step: event.step }
      if (at === -1) return { ...activity, entries: [...activity.entries, entry] }
      const entries = activity.entries.slice()
      entries[at] = entry
      return { ...activity, entries }
    }
    case 'call_rejected':
      return {
        ...activity,
        entries: [
          ...activity.entries,
          { kind: 'rejected', tool: event.tool, reason: event.reason },
        ],
      }
  }
}

/**
 * The entries to show in the trace: everything but empty notes and, once the task has its
 * answer, the note that became that answer (it is shown on its own).
 */
export function traceEntries(activity: Activity, answer: string | null): ActivityEntry[] {
  const final = answer?.trim() ?? ''
  let answerAt = -1
  if (final !== '') {
    activity.entries.forEach((entry, at) => {
      if (entry.kind === 'note' && entry.text.trim() === final) answerAt = at
    })
  }
  return activity.entries.filter(
    (entry, at) => at !== answerAt && !(entry.kind === 'note' && entry.text.trim() === ''),
  )
}

/** The answer as it streams in: the current model call's text. */
export function streamingText(activity: Activity): string {
  const last = activity.entries.at(-1)
  return last?.kind === 'note' && last.round === activity.round ? last.text : ''
}
