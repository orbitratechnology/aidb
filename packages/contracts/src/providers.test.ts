import { describe, expect, it } from 'vitest'
import { ProviderDetailsSchema, ProviderDraftSchema, ProviderTestReportSchema } from './providers'

const draft = {
  name: ' Local ',
  kind: 'ollama',
  endpoint: null,
  model: 'qwen2.5',
  requestTimeoutSecs: 120,
  contextBudgetTokens: null,
  allowInsecureHttp: false,
} as const

describe('provider contracts', () => {
  it('validates drafts with the core limits', () => {
    expect(ProviderDraftSchema.parse(draft).name).toBe('Local')
    expect(ProviderDraftSchema.safeParse({ ...draft, kind: 'google' }).success).toBe(false)
    expect(ProviderDraftSchema.safeParse({ ...draft, requestTimeoutSecs: 1 }).success).toBe(false)
    expect(ProviderDraftSchema.safeParse({ ...draft, model: '  ' }).success).toBe(false)
  })

  it('reads details and test reports as the core sends them', () => {
    const details = ProviderDetailsSchema.parse({
      ...draft,
      id: '6f1c2a40-0000-4000-8000-000000000000',
      endpointUrl: 'http://localhost:11434/v1',
      key: 'not_saved',
      keyRequired: false,
      insecureRemote: false,
      features: { toolCalls: true, structuredOutput: 'json_schema' },
      checkedAt: null,
      createdAt: '2026-11-17T10:00:00.000Z',
      updatedAt: '2026-11-17T10:00:00.000Z',
    })
    expect(details.features.toolCalls).toBe(true)
    const report = ProviderTestReportSchema.parse({
      checks: [{ kind: 'tool_calls', outcome: 'warning', detail: 'Did not call the tool.' }],
      models: [{ id: 'qwen2.5', displayName: null, contextWindow: null, maxOutputTokens: null }],
      answerMs: 812,
      features: { toolCalls: true, structuredOutput: 'json_object' },
    })
    expect(report.checks[0]?.outcome).toBe('warning')
  })
})
