import type { HistoryEntry } from '@aidb/contracts'

/** Query keys of the history and saved query lists. */
export const HISTORY_KEY = 'query-history'
export const SAVED_KEY = 'saved-queries'

/** The most history entries or saved queries one list shows (the core's cap). */
export const LIST_LIMIT = 200

const PREVIEW_LINES = 4
const PREVIEW_CHARS = 400

/** The start of a statement for a list: a few lines, without leading blank lines. */
export function sqlPreview(sql: string): string {
  const lines = sql
    .replace(/^\s*\n/, '')
    .slice(0, PREVIEW_CHARS)
    .split('\n')
  const shown = lines.slice(0, PREVIEW_LINES).join('\n').trimEnd()
  return shown.length < sql.trim().length ? `${shown} …` : shown
}

const STATUS_TEXT: Record<HistoryEntry['status'], string> = {
  completed: 'Ran',
  refused: 'Not run',
  failed: 'Failed',
  cancelled: 'Cancelled',
}

/** What happened to a run, in words: "Ran · 12 rows · 4 ms". */
export function historyOutcome(entry: HistoryEntry): string {
  const parts = [STATUS_TEXT[entry.status]]
  if (entry.status === 'completed' && entry.rowCount !== null) {
    const rows = entry.rowCount.toLocaleString('en-US')
    parts.push(`${entry.rowCount === 1 ? '1 row' : `${rows} rows`}`)
  }
  if (entry.status === 'failed' && entry.errorCode === 'timeout') parts[0] = 'Timed out'
  if (entry.elapsedMs !== null) parts.push(`${entry.elapsedMs.toLocaleString('en-US')} ms`)
  return parts.join(' · ')
}
