import type { Actor, AuditEntry, AuditStatus } from '@aidb/contracts'
import {
  BanIcon,
  CircleCheckIcon,
  CircleDashedIcon,
  CircleStopIcon,
  CircleXIcon,
  type LucideIcon,
} from 'lucide-react'

/** What each recorded operation was, in words. */
const OPERATION_LABELS: Record<string, string> = {
  approve: 'Approved a change',
  approval_check: 'Approval check',
  data_change: 'Data change',
  assistant_task: 'Assistant answered a question',
  tool_call: 'Assistant request',
  sql_refused: 'Statement in the SQL editor',
  connection_created: 'Connection added',
  connection_updated: 'Connection changed',
  connection_deleted: 'Connection removed',
  connection_imported: 'Connection imported',
}

/** The label for an operation; unknown ones (from a later version) are shown readably. */
export function operationLabel(operation: string): string {
  const known = OPERATION_LABELS[operation]
  if (known) return known
  const words = operation.replaceAll('_', ' ').trim()
  return words ? words[0]!.toUpperCase() + words.slice(1) : 'Operation'
}

export const STATUS_DISPLAY: Record<
  AuditStatus,
  { label: string; icon: LucideIcon; variant: 'secondary' | 'outline' | 'destructive' }
> = {
  succeeded: { label: 'Done', icon: CircleCheckIcon, variant: 'secondary' },
  partial: { label: 'Partly done', icon: CircleDashedIcon, variant: 'outline' },
  failed: { label: 'Failed', icon: CircleXIcon, variant: 'destructive' },
  cancelled: { label: 'Stopped', icon: CircleStopIcon, variant: 'outline' },
  rejected: { label: 'Refused', icon: BanIcon, variant: 'destructive' },
}

export const ACTOR_LABELS: Record<Actor, string> = {
  user: 'You',
  agent: 'Assistant',
  system: 'aidb',
}

/** The first 12 hex characters of a fingerprint: enough to tell entries apart. */
export function shortFingerprint(fingerprint: string): string {
  return fingerprint.slice(0, 12)
}

/** Events per page of the history. */
export const PAGE_SIZE = 50

/** The id to continue the history after `page`, or `undefined` when it was the last page. */
export function nextBefore(page: AuditEntry[]): number | undefined {
  return page.length < PAGE_SIZE ? undefined : page.at(-1)?.id
}
