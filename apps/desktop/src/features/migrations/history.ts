import type { MigrationRecord, MigrationStatus, Verification } from '@aidb/contracts'

export const MIGRATION_RECORDS_KEY = ['migrationRecords'] as const

export const STATUS_LABELS: Record<MigrationStatus, string> = {
  running: 'Running',
  succeeded: 'Applied',
  failed: 'Rolled back',
  partial: 'Partly applied',
  uncertain: 'Outcome unknown',
}

export const VERIFICATION_LABELS: Record<Verification, string> = {
  passed: 'Schemas match',
  failed: 'Still differ',
  not_run: 'Not checked',
}

/** "3 of 4 steps", or "4 steps" when all ran. */
export function stepsText(record: Pick<MigrationRecord, 'steps' | 'appliedSteps'>): string {
  const total = `${record.steps} ${record.steps === 1 ? 'step' : 'steps'}`
  return record.appliedSteps === record.steps ? total : `${record.appliedSteps} of ${total}`
}

/** Whether a status needs the user's attention: the schema may be between two states. */
export function needsAttention(status: MigrationStatus): boolean {
  return status === 'partial' || status === 'uncertain' || status === 'running'
}
