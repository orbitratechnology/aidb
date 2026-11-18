import { describe, expect, it } from 'vitest'
import {
  emptyFormValues,
  endpointProblem,
  needsInsecureConsent,
  ProviderFormSchema,
  switchKind,
  toDraft,
} from './provider-form'

const errors = (values: unknown) => {
  const result = ProviderFormSchema.safeParse(values)
  return result.success
    ? {}
    : Object.fromEntries(result.error.issues.map((i) => [i.path[0], i.message]))
}

describe('provider form', () => {
  it('starts from the kind defaults and needs a key for hosted providers', () => {
    const anthropic = emptyFormValues('anthropic')
    expect(anthropic).toMatchObject({ name: 'Anthropic', model: 'claude-opus-5-5' })
    expect(errors(anthropic)).toEqual({ apiKey: 'Enter the API key.' })
    expect(errors({ ...anthropic, hasSavedKey: true })).toEqual({})
    expect(errors({ ...anthropic, apiKey: 'sk ant' }).apiKey).toBe('API keys have no spaces.')
    expect(errors({ ...emptyFormValues('ollama'), model: 'qwen2.5' })).toEqual({})
  })

  it('needs an endpoint only for a generic server, and checks it', () => {
    const generic = { ...emptyFormValues('openai_compatible'), model: 'm' }
    expect(errors(generic).endpoint).toBe('Enter the server address.')
    expect(endpointProblem('llm.example.com')).not.toBeNull()
    expect(endpointProblem('https://user:pw@llm.example.com')).not.toBeNull()
    expect(endpointProblem('https://llm.example.com/v1?x=1')).not.toBeNull()
    expect(endpointProblem('https://llm.example.com/v1')).toBeNull()
  })

  it('asks for consent only for plain HTTP to another machine', () => {
    expect(needsInsecureConsent('ollama', '')).toBe(false)
    expect(needsInsecureConsent('openai_compatible', 'http://127.0.0.1:8080/v1')).toBe(false)
    expect(needsInsecureConsent('openai_compatible', 'http://[::1]:8080/v1')).toBe(false)
    expect(needsInsecureConsent('openai_compatible', 'https://10.0.0.5/v1')).toBe(false)
    expect(needsInsecureConsent('openai_compatible', 'http://10.0.0.5:8000/v1')).toBe(true)
    const remote = {
      ...emptyFormValues('openai_compatible'),
      model: 'm',
      endpoint: 'http://10.0.0.5:8000/v1',
    }
    expect(errors(remote).allowInsecureHttp).toBeDefined()
    expect(errors({ ...remote, allowInsecureHttp: true })).toEqual({})
  })

  it('builds the draft without the key, clearing consent that is not needed', () => {
    const draft = toDraft({
      ...emptyFormValues('ollama'),
      name: ' Local ',
      model: ' qwen2.5 ',
      apiKey: 'secret',
      contextBudgetTokens: '32000',
      allowInsecureHttp: true,
    })
    expect(draft).toEqual({
      name: 'Local',
      kind: 'ollama',
      endpoint: null,
      model: 'qwen2.5',
      requestTimeoutSecs: 120,
      contextBudgetTokens: 32000,
      allowInsecureHttp: false,
    })
    expect(JSON.stringify(draft)).not.toContain('secret')
    expect(
      errors({ ...emptyFormValues('ollama'), model: 'm', contextBudgetTokens: '10' }),
    ).toHaveProperty('contextBudgetTokens')
  })

  it('switching kind replaces only untouched defaults', () => {
    const switched = switchKind(emptyFormValues('anthropic'), 'ollama')
    expect(switched).toMatchObject({ kind: 'ollama', name: 'Ollama', model: '' })
    const custom = switchKind(
      { ...emptyFormValues('anthropic'), name: 'Work', model: 'x' },
      'openai',
    )
    expect(custom).toMatchObject({ kind: 'openai', name: 'Work', model: 'x' })
  })
})
