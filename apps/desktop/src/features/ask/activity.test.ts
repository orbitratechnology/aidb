import type { PendingChange, PlanStep, TaskEvent } from '@aidb/contracts'
import { describe, expect, it } from 'vitest'
import { applyEvent, emptyActivity, streamingText, traceEntries } from './activity'

const step = (status: PlanStep['status'], outcome: string | null = null): PlanStep => ({
  tool: 'run_read_query',
  summary: 'Run a read query: count orders',
  input: { sql: 'SELECT count(*) FROM orders', purpose: 'count orders' },
  risk: 'R1',
  status,
  outcome,
})

function play(events: TaskEvent[]) {
  return events.reduce(applyEvent, emptyActivity())
}

describe('assistant activity', () => {
  it('builds the trace from streamed events', () => {
    const activity = play([
      { event: 'started', taskId: 't1', items: [] },
      { event: 'round_started', round: 0 },
      { event: 'text', text: 'Let me ' },
      { event: 'text', text: 'count them.' },
      { event: 'step_started', index: 0, step: step('running') },
      { event: 'call_rejected', tool: 'drop_table', reason: 'There is no tool named drop_table.' },
      { event: 'step_finished', index: 0, step: step('succeeded', '1 row') },
      { event: 'round_started', round: 1 },
      { event: 'text', text: 'There are ' },
      { event: 'text', text: '12 orders.' },
    ])
    expect(activity.taskId).toBe('t1')
    expect(activity.entries).toHaveLength(4)
    expect(activity.entries[0]).toEqual({ kind: 'note', round: 0, text: 'Let me count them.' })
    // The step was updated in place, not added twice.
    expect(activity.entries[1]).toMatchObject({ kind: 'step', step: { status: 'succeeded' } })
    expect(activity.entries[2]).toMatchObject({ kind: 'rejected', tool: 'drop_table' })
    expect(streamingText(activity)).toBe('There are 12 orders.')

    // While running everything shows; once answered, the answer's note moves out of the trace.
    expect(traceEntries(activity, null)).toHaveLength(4)
    const answered = traceEntries(activity, 'There are 12 orders.')
    expect(answered).toHaveLength(3)
    expect(answered.some((entry) => entry.kind === 'note' && entry.round === 1)).toBe(false)
  })

  it('has no streaming text once a step starts', () => {
    const activity = play([
      { event: 'round_started', round: 0 },
      { event: 'text', text: '   ' },
      { event: 'step_started', index: 0, step: step('running') },
    ])
    expect(streamingText(activity)).toBe('')
    // Blank notes are never shown.
    expect(traceEntries(activity, null)).toHaveLength(1)
  })

  it('keeps the change the assistant prepared', () => {
    const change: PendingChange = {
      connectionId: 'c',
      actor: 'agent',
      preview: {
        approvalId: '6f1c2a40-0000-4000-8000-000000000003',
        changes: [],
        rows: 1,
        risk: 'R2',
        approval: 'apply',
        reasons: [],
        confirmation: null,
        expiresAt: '2026-11-20T10:00:00Z',
      },
      changes: [],
    }
    const activity = play([{ event: 'change_proposed', index: 1, change }])
    expect(activity.proposal?.preview.approvalId).toBe(change.preview.approvalId)
    expect(emptyActivity().proposal).toBeNull()
  })
})
