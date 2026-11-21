import type { PlanStep, StepStatus, TaskStatus, ToolName } from '@aidb/contracts'
import {
  BanIcon,
  CircleAlertIcon,
  CircleCheckIcon,
  CircleStopIcon,
  CircleXIcon,
  FilePenLineIcon,
  FolderTreeIcon,
  GaugeIcon,
  PlayIcon,
  TableIcon,
  TablePropertiesIcon,
  type LucideIcon,
} from 'lucide-react'

export const TOOL_ICONS: Record<ToolName, LucideIcon> = {
  list_schemas: FolderTreeIcon,
  list_tables: TableIcon,
  describe_table: TablePropertiesIcon,
  run_read_query: PlayIcon,
  explain_query: GaugeIcon,
  preview_data_change: FilePenLineIcon,
}

/** Words for a step's state, so it never rests on color or animation alone. */
export const STEP_STATUS_LABELS: Record<StepStatus, string> = {
  planned: 'Planned',
  running: 'Running',
  succeeded: 'Done',
  failed: 'Failed',
  skipped: 'Skipped',
  cancelled: 'Stopped',
}

/** The `TraceLine` status for a step. */
export function traceStatus(status: StepStatus): 'running' | 'error' | 'pending' | undefined {
  switch (status) {
    case 'running':
      return 'running'
    case 'failed':
      return 'error'
    case 'planned':
      return 'pending'
    default:
      return undefined
  }
}

/** The SQL a step ran, if it ran any. */
export function stepSql(step: PlanStep): string | null {
  const input = step.input
  if (input && typeof input === 'object' && !Array.isArray(input) && 'sql' in input) {
    const sql = (input as { sql?: unknown }).sql
    return typeof sql === 'string' ? sql : null
  }
  return null
}

export type TaskStatusDisplay = {
  label: string
  description: string
  icon: LucideIcon
  variant: 'secondary' | 'outline' | 'destructive'
}

export const TASK_STATUS_DISPLAY: Record<
  Exclude<TaskStatus, 'executing' | 'verifying'>,
  TaskStatusDisplay
> = {
  succeeded: {
    label: 'Checked',
    description: 'Every check passed.',
    icon: CircleCheckIcon,
    variant: 'secondary',
  },
  partial: {
    label: 'Partly checked',
    description: 'Some of what the answer relies on did not pass a check.',
    icon: CircleAlertIcon,
    variant: 'outline',
  },
  failed: {
    label: 'Failed',
    description: 'The assistant could not answer.',
    icon: CircleXIcon,
    variant: 'destructive',
  },
  cancelled: {
    label: 'Stopped',
    description: 'You stopped this question.',
    icon: CircleStopIcon,
    variant: 'outline',
  },
}

export const REJECTED_ICON = BanIcon

/** "~1.2k tokens" for a token estimate. */
export function formatTokens(tokens: number): string {
  if (tokens < 1000) return `~${tokens} tokens`
  return `~${(Math.round(tokens / 100) / 10).toLocaleString()}k tokens`
}
