import { describe, expect, it } from 'vitest'
import { AuditEntrySchema } from './audit'

describe('audit contracts', () => {
  it('reads entries as the core sends them', () => {
    const entry = {
      id: 7,
      occurredAt: '2026-11-19T10:00:00.000Z',
      connectionId: '6f1c2a40-0000-4000-8000-000000000002',
      environment: 'production',
      actor: 'agent',
      operation: 'tool_call',
      risk: null,
      fingerprint: null,
      approvalState: null,
      status: 'rejected',
      rowsAffected: null,
      detail: 'a tool that does not exist: unknown_tool',
    }
    expect(AuditEntrySchema.parse(entry)).toEqual(entry)
    expect(AuditEntrySchema.safeParse({ ...entry, status: 'approved' }).success).toBe(false)
    expect(AuditEntrySchema.safeParse({ ...entry, actor: 'model' }).success).toBe(false)
  })
})
