import { z } from 'zod'
import {
  PROVIDER_LIMITS,
  ProviderPresetSchema,
  type ProviderDetails,
  type ProviderDraft,
  type ProviderPreset,
} from '@aidb/contracts'

// The same rules as `ProviderDraft::validated` in the native core, which checks again on save.
// These give immediate feedback; the core's answer is the one that counts.

/** What each kind of provider needs, and its defaults. */
export const PRESETS: Record<
  ProviderPreset,
  {
    label: string
    description: string
    defaultEndpoint: string | null
    keyRequired: boolean
    defaultModel: string
    modelPlaceholder: string
  }
> = {
  anthropic: {
    label: 'Anthropic',
    description: 'Claude models through the Anthropic API. Needs an API key.',
    defaultEndpoint: 'https://api.anthropic.com',
    keyRequired: true,
    defaultModel: 'claude-opus-5-5',
    modelPlaceholder: 'claude-opus-5-5',
  },
  openai: {
    label: 'OpenAI',
    description: 'Models through the OpenAI API. Needs an API key.',
    defaultEndpoint: 'https://api.openai.com/v1',
    keyRequired: true,
    defaultModel: '',
    modelPlaceholder: 'The model ID from your OpenAI account',
  },
  ollama: {
    label: 'Ollama',
    description: 'Models running on this computer with Ollama. Prompts stay on this computer.',
    defaultEndpoint: 'http://localhost:11434/v1',
    keyRequired: false,
    defaultModel: '',
    modelPlaceholder: 'qwen2.5:7b',
  },
  openai_compatible: {
    label: 'OpenAI-compatible server',
    description: 'LM Studio, llama.cpp, vLLM or a gateway that speaks the OpenAI API.',
    defaultEndpoint: null,
    keyRequired: false,
    defaultModel: '',
    modelPlaceholder: 'The model name the server lists',
  },
}

export const PRESET_ORDER: readonly ProviderPreset[] = [
  'anthropic',
  'openai',
  'ollama',
  'openai_compatible',
]

function hasControlCharacters(text: string): boolean {
  for (const char of text) {
    const code = char.codePointAt(0) ?? 0
    if (code < 0x20 || (code >= 0x7f && code <= 0x9f)) return true
  }
  return false
}

function parseUrl(text: string): URL | null {
  try {
    return new URL(text)
  } catch {
    return null
  }
}

/** Whether `url` names this machine. */
export function isLoopbackUrl(url: URL): boolean {
  const host = url.hostname
    .replace(/^\[|\]$/g, '')
    .replace(/\.$/, '')
    .toLowerCase()
  return (
    host === 'localhost' ||
    host.endsWith('.localhost') ||
    host === '::1' ||
    /^127(\.\d{1,3}){3}$/.test(host)
  )
}

/** The endpoint requests would go to: the one typed, or the kind's default. */
export function effectiveEndpoint(kind: ProviderPreset, endpoint: string): string | null {
  return endpoint.trim() || PRESETS[kind].defaultEndpoint
}

/** Why an endpoint is unusable, or `null` when it is fine. */
export function endpointProblem(text: string): string | null {
  const url = parseUrl(text)
  if (!url || !/^https?:$/.test(url.protocol) || !url.hostname) {
    return 'Enter a full URL starting with https:// or http://.'
  }
  if (url.username || url.password) return 'Remove the credentials; save the API key below.'
  if (url.search || url.hash) return 'Remove the query or fragment.'
  return null
}

/** Plain HTTP to another machine: prompts and the key would cross the network unencrypted. */
export function needsInsecureConsent(kind: ProviderPreset, endpoint: string): boolean {
  const effective = effectiveEndpoint(kind, endpoint)
  const url = effective ? parseUrl(effective) : null
  return url !== null && url.protocol === 'http:' && !isLoopbackUrl(url)
}

const text = (label: string, max: number) =>
  z
    .string()
    .trim()
    .min(1, { error: `Enter a ${label}.` })
    .max(max, { error: `Use at most ${max} characters.` })
    .refine((value) => !hasControlCharacters(value), { error: 'Remove control characters.' })

const optionalWholeNumber = z
  .string()
  .trim()
  .refine((value) => value === '' || /^\d+$/.test(value), { error: 'Enter a whole number.' })

export const ProviderFormSchema = z
  .object({
    kind: ProviderPresetSchema,
    name: text('name', PROVIDER_LIMITS.nameMax),
    endpoint: z
      .string()
      .trim()
      .max(PROVIDER_LIMITS.endpointMax, {
        error: `Use at most ${PROVIDER_LIMITS.endpointMax} characters.`,
      }),
    model: text('model', PROVIDER_LIMITS.modelMax),
    apiKey: z
      .string()
      .trim()
      .refine((value) => !/\s/.test(value), { error: 'API keys have no spaces.' }),
    requestTimeoutSecs: z
      .number({ error: 'Enter a whole number.' })
      .int({ error: 'Enter a whole number.' })
      .min(PROVIDER_LIMITS.requestTimeoutSecs.min, {
        error: `Must be between ${PROVIDER_LIMITS.requestTimeoutSecs.min} and ${PROVIDER_LIMITS.requestTimeoutSecs.max}.`,
      })
      .max(PROVIDER_LIMITS.requestTimeoutSecs.max, {
        error: `Must be between ${PROVIDER_LIMITS.requestTimeoutSecs.min} and ${PROVIDER_LIMITS.requestTimeoutSecs.max}.`,
      }),
    contextBudgetTokens: optionalWholeNumber,
    allowInsecureHttp: z.boolean(),
    /** A key is already saved for this profile. */
    hasSavedKey: z.boolean(),
  })
  .superRefine((values, context) => {
    const endpoint = values.endpoint.trim()
    if (!endpoint && PRESETS[values.kind].defaultEndpoint === null) {
      context.addIssue({ code: 'custom', path: ['endpoint'], message: 'Enter the server address.' })
    } else if (endpoint) {
      const problem = endpointProblem(endpoint)
      if (problem) context.addIssue({ code: 'custom', path: ['endpoint'], message: problem })
    }
    if (needsInsecureConsent(values.kind, values.endpoint) && !values.allowInsecureHttp) {
      context.addIssue({
        code: 'custom',
        path: ['allowInsecureHttp'],
        message: 'Confirm that prompts and the key may cross the network unencrypted.',
      })
    }
    if (PRESETS[values.kind].keyRequired && !values.apiKey.trim() && !values.hasSavedKey) {
      context.addIssue({ code: 'custom', path: ['apiKey'], message: 'Enter the API key.' })
    }
    const budget = values.contextBudgetTokens.trim()
    if (budget && /^\d+$/.test(budget)) {
      const { min, max } = PROVIDER_LIMITS.contextBudgetTokens
      const tokens = Number(budget)
      if (tokens < min || tokens > max) {
        context.addIssue({
          code: 'custom',
          path: ['contextBudgetTokens'],
          message: `Must be between ${min.toLocaleString('en-US')} and ${max.toLocaleString('en-US')}.`,
        })
      }
    }
  })

export type ProviderFormValues = z.infer<typeof ProviderFormSchema>

export function emptyFormValues(kind: ProviderPreset = 'anthropic'): ProviderFormValues {
  return {
    kind,
    name: PRESETS[kind].label,
    endpoint: '',
    model: PRESETS[kind].defaultModel,
    apiKey: '',
    requestTimeoutSecs: 120,
    contextBudgetTokens: '',
    allowInsecureHttp: false,
    hasSavedKey: false,
  }
}

/** Form values for editing a saved profile. The key field starts empty: blank keeps it. */
export function formValuesFrom(provider: ProviderDetails): ProviderFormValues {
  return {
    kind: provider.kind,
    name: provider.name,
    endpoint: provider.endpoint ?? '',
    model: provider.model,
    apiKey: '',
    requestTimeoutSecs: provider.requestTimeoutSecs,
    contextBudgetTokens: provider.contextBudgetTokens?.toString() ?? '',
    allowInsecureHttp: provider.allowInsecureHttp,
    hasSavedKey: provider.key !== 'not_saved',
  }
}

/** The profile to send to the core; never includes the key, which is saved separately. */
export function toDraft(values: ProviderFormValues): ProviderDraft {
  const budget = values.contextBudgetTokens.trim()
  return {
    name: values.name.trim(),
    kind: values.kind,
    endpoint: values.endpoint.trim() || null,
    model: values.model.trim(),
    requestTimeoutSecs: values.requestTimeoutSecs,
    contextBudgetTokens: budget ? Number(budget) : null,
    allowInsecureHttp: needsInsecureConsent(values.kind, values.endpoint)
      ? values.allowInsecureHttp
      : false,
  }
}

/**
 * Changing the kind swaps the name and model for the new kind's defaults, but only where the
 * user kept the old kind's defaults.
 */
export function switchKind(values: ProviderFormValues, kind: ProviderPreset): ProviderFormValues {
  const previous = PRESETS[values.kind]
  return {
    ...values,
    kind,
    name: values.name.trim() === previous.label ? PRESETS[kind].label : values.name,
    model:
      values.model.trim() === previous.defaultModel ? PRESETS[kind].defaultModel : values.model,
  }
}
