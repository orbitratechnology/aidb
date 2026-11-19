import { describe, expect, it } from 'vitest'
import { ContextPreviewSchema, TaskEventSchema, TaskOutcomeSchema } from './assistant'

const step = {
  tool: 'run_read_query',
  summary: 'Run a read query: count orders',
  input: { sql: 'SELECT count(*) FROM orders', purpose: 'count orders' },
  risk: 'R1',
  status: 'succeeded',
  outcome: '1 row',
} as const

const plan = {
  risk: 'R1',
  requiredApproval: 'none',
  version: 1,
  toolsVersion: 1,
  taskId: '6f1c2a40-0000-4000-8000-000000000001',
  target: {
    connectionId: '6f1c2a40-0000-4000-8000-000000000002',
    database: 'shop',
    environment: 'development',
    readOnly: true,
  },
  intent: 'How many orders?',
  assumptions: [],
  steps: [step],
  verification: [{ description: 'The assistant gave an answer.', status: 'passed', summary: null }],
} as const

describe('assistant contracts', () => {
  it('reads every event the planner sends', () => {
    const events = [
      { event: 'started', taskId: plan.taskId, items: [] },
      { event: 'round_started', round: 0 },
      { event: 'text', text: 'Looking' },
      { event: 'step_started', index: 0, step: { ...step, status: 'running', outcome: null } },
      { event: 'step_finished', index: 0, step },
      { event: 'call_rejected', tool: 'drop_table', reason: 'There is no tool named drop_table.' },
    ]
    for (const event of events) {
      expect(TaskEventSchema.parse(event)).toEqual(event)
    }
    expect(TaskEventSchema.safeParse({ event: 'approve', taskId: plan.taskId }).success).toBe(false)
  })

  it('reads outcomes and previews', () => {
    const outcome = { taskId: plan.taskId, status: 'partial', answer: '12', notice: null, plan }
    expect(TaskOutcomeSchema.parse(outcome).plan.steps[0]?.input).toEqual(step.input)
    expect(TaskOutcomeSchema.safeParse({ ...outcome, status: 'approved' }).success).toBe(false)
    const preview = ContextPreviewSchema.parse({
      provider: { name: 'Local', model: 'qwen', hosted: false },
      items: [
        {
          kind: 'attachment',
          label: 'orders.csv',
          includedChars: 10,
          totalChars: 20,
          truncated: true,
          containsDataValues: true,
        },
      ],
      estimatedTokens: 500,
      budgetTokens: 16000,
      dataAllowed: true,
    })
    expect(preview.items[0]?.truncated).toBe(true)
  })
})
