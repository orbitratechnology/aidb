import type {
  CheckKind,
  CheckOutcome,
  KeyStorage,
  ProviderDetails,
  StructuredOutput,
} from '@aidb/contracts'
import {
  CircleCheckIcon,
  CircleMinusIcon,
  CircleXIcon,
  KeyRoundIcon,
  KeySquareIcon,
  TimerIcon,
  TriangleAlertIcon,
  type LucideIcon,
} from 'lucide-react'
import { isLoopbackUrl } from './provider-form'

export const KEY_DISPLAY: Record<KeyStorage, { label: string; icon: LucideIcon }> = {
  os_store: { label: 'Key saved', icon: KeyRoundIcon },
  session_only: { label: 'Key for this session', icon: TimerIcon },
  not_saved: { label: 'No key', icon: KeySquareIcon },
}

export const STRUCTURED_OUTPUT_LABELS: Record<StructuredOutput, string> = {
  json_schema: 'JSON schema',
  json_object: 'JSON mode',
  instructions: 'JSON by instructions',
}

export const CHECK_LABELS: Record<CheckKind, string> = {
  endpoint: 'Endpoint and key',
  model: 'Model',
  answer: 'Answer',
  tool_calls: 'Tool calls',
  structured_output: 'JSON answers',
}

/** Each outcome has its own icon and words, never color alone. */
export const OUTCOME_DISPLAY: Record<
  CheckOutcome,
  { label: string; icon: LucideIcon; className: string }
> = {
  passed: { label: 'Passed', icon: CircleCheckIcon, className: 'text-foreground' },
  warning: { label: 'Works with a limit', icon: TriangleAlertIcon, className: 'text-foreground' },
  failed: { label: 'Failed', icon: CircleXIcon, className: 'text-destructive' },
  skipped: { label: 'Not checked', icon: CircleMinusIcon, className: 'text-muted-foreground' },
}

/** Whether requests to this provider stay on this computer. */
export function isLocalProvider(provider: Pick<ProviderDetails, 'endpointUrl'>): boolean {
  try {
    return isLoopbackUrl(new URL(provider.endpointUrl))
  } catch {
    return false
  }
}

/** The host requests go to, for messages such as "Prompts go to api.anthropic.com". */
export function providerHost(provider: Pick<ProviderDetails, 'endpointUrl'>): string {
  try {
    return new URL(provider.endpointUrl).host
  } catch {
    return provider.endpointUrl
  }
}

/** Why the agent cannot use this provider yet, or `null` when it can. */
export function providerBlocker(provider: ProviderDetails): string | null {
  if (provider.keyRequired && provider.key === 'not_saved') return 'Save an API key first.'
  return null
}
