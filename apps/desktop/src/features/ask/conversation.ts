import { ASSISTANT_LIMITS, type Exchange, type ObjectName, type TaskOutcome } from '@aidb/contracts'
import type { Activity } from './activity'

/** A file the user attached to the next question, read in the webview. */
export type DraftAttachment = { id: string; name: string; size: number; content: string }

/** One question and what came of it. */
export type Turn = {
  id: string
  question: string
  attachments: { name: string; size: number }[]
  selected: ObjectName[]
  activity: Activity
  /** Running until the core answers; `error` when the question was not accepted. */
  state: 'running' | 'done' | 'error'
  outcome?: TaskOutcome
  error?: string
}

/**
 * Earlier questions and answers a follow-up may build on: the most recent finished turns with
 * an answer, oldest first, each answer cut to the core's limit.
 */
export function historyFor(turns: Turn[]): Exchange[] {
  return turns
    .filter((turn) => turn.state === 'done' && turn.outcome && turn.outcome.answer.trim() !== '')
    .slice(-ASSISTANT_LIMITS.maxHistory)
    .map((turn) => ({
      question: turn.question,
      answer: Array.from(turn.outcome?.answer ?? '')
        .slice(0, ASSISTANT_LIMITS.answerMaxChars)
        .join(''),
    }))
}

const EXTENSIONS = ['.sql', '.csv', '.json', '.md', '.markdown', '.txt']

/** The `accept` list for the file picker. */
export const ATTACHMENT_ACCEPT = EXTENSIONS.join(',')

/** Why a file cannot be attached, or `null` when it can. The core checks again. */
export function attachmentProblem(
  file: { name: string; size: number },
  attached: { name: string }[],
): string | null {
  const lower = file.name.toLowerCase()
  if (!EXTENSIONS.some((extension) => lower.endsWith(extension))) {
    return `“${file.name}” cannot be attached: use a .sql, .csv, .json, .md or .txt file.`
  }
  if (file.size > ASSISTANT_LIMITS.attachmentMaxBytes) {
    return `“${file.name}” is too large to attach (${ASSISTANT_LIMITS.attachmentMaxBytes / 1024} KB at most).`
  }
  if (attached.length >= ASSISTANT_LIMITS.maxAttachments) {
    return `Attach at most ${ASSISTANT_LIMITS.maxAttachments} files.`
  }
  if (attached.some((other) => other.name === file.name)) {
    return `“${file.name}” is already attached.`
  }
  return null
}

/** A file's size for people: bytes, KB or MB. */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 102.4) / 10} KB`
  return `${Math.round(bytes / (1024 * 102.4)) / 10} MB`
}

/** "public.orders" for a table name. */
export function objectLabel(object: ObjectName): string {
  return `${object.schema}.${object.name}`
}
