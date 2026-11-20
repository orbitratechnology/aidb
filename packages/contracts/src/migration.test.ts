import { describe, expect, it } from 'vitest'
import { MigrationPreviewSchema, MigrationRecordSchema, MigrationResultSchema } from './migration'

describe('migration contracts', () => {
  it('reads a prepared migration with the steps that delete data', () => {
    const preview = MigrationPreviewSchema.parse({
      approvalId: '6f1c2a40-0000-4000-8000-000000000001',
      diffId: '6f1c2a40-0000-4000-8000-000000000002',
      steps: 3,
      dataLoss: ['Drop table old_log'],
      outsideTransaction: 0,
      risk: 'R4',
      approval: 'elevated',
      reasons: ['Drops objects and the data in them.'],
      confirmation: 'shop',
      expiresAt: '2026-11-20T10:00:00Z',
    })
    expect(preview.dataLoss).toHaveLength(1)
  })

  it('reads how a migration ended and its record', () => {
    const result = MigrationResultSchema.parse({
      recordId: '6f1c2a40-0000-4000-8000-000000000003',
      status: 'failed',
      appliedSteps: 0,
      totalSteps: 3,
      failedStep: { number: 2, summary: 'Add column code to items', message: 'null value' },
      verification: 'not_run',
      remaining: 0,
      message: 'Step 2 of 3 failed.',
    })
    expect(result.failedStep?.number).toBe(2)
    expect(
      MigrationRecordSchema.safeParse({
        id: '6f1c2a40-0000-4000-8000-000000000003',
        connectionId: null,
        environment: 'production',
        targetSchema: 'public',
        source: 'staging on Dev',
        scriptSha256: 'a'.repeat(64),
        steps: 3,
        appliedSteps: 3,
        risk: 'R4',
        startedAt: '2026-11-20T10:00:00Z',
        finishedAt: null,
        status: 'cancelled',
        verification: 'passed',
        detail: null,
      }).success,
    ).toBe(false)
  })
})
