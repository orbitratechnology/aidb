import { useId, useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { BotIcon, InfoIcon, PlusIcon } from 'lucide-react'
import { toast } from 'sonner'
import type { AiSettings, ProviderDetails } from '@aidb/contracts'
import { LoadError } from '@/components/shared/load-error'
import { EmptyState } from '@/components/shared/status-states'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { Field, FieldContent, FieldDescription, FieldLabel } from '@/components/ui/field'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { Switch } from '@/components/ui/switch'
import { WhatIsSent } from '@/features/ai/disclosure'
import { isLocalProvider, providerBlocker, providerHost } from '@/features/ai/provider-display'
import { ProviderList } from '@/features/ai/provider-list'
import { ProviderSheet, type ProviderFormMode } from '@/features/ai/provider-sheet'
import { PROVIDERS_QUERY_KEY, useProviders } from '@/features/ai/queries'
import { ProviderTestDialog } from '@/features/ai/test-dialog'
import { SETTINGS_QUERY_KEY, useSettings, useUpdateSettings } from '@/hooks/use-settings'
import { deleteProvider } from '@/lib/commands'
import { SettingsSection } from '../settings-section'

export function AiSettings() {
  const providers = useProviders()
  const settings = useSettings()
  const [formMode, setFormMode] = useState<ProviderFormMode | null>(null)
  const [testing, setTesting] = useState<ProviderDetails | null>(null)
  const [deleting, setDeleting] = useState<ProviderDetails | null>(null)
  const ai = settings.data?.ai
  const list = providers.data ?? []

  const addButton = (
    <Button className="min-h-11 sm:min-h-0" onClick={() => setFormMode({ kind: 'create' })}>
      <PlusIcon data-icon="inline-start" />
      Add provider
    </Button>
  )

  return (
    <SettingsSection
      title="AI providers"
      description="The assistant is optional and off until you turn it on. Manual SQL and data tools never need it."
    >
      {(providers.isPending || settings.isPending) && <Skeleton className="h-48 w-full" />}
      {providers.isError && (
        <LoadError
          title="Providers could not be loaded"
          error={providers.error}
          retrying={providers.isFetching}
          onRetry={() => void providers.refetch()}
        />
      )}
      {providers.isSuccess && ai && (
        <>
          <AssistantSwitch ai={ai} providers={list} />
          {list.length === 0 ? (
            <EmptyState
              icon={BotIcon}
              title="No providers yet"
              description="Add a hosted provider such as Anthropic or OpenAI, or a model running on this computer with Ollama."
            >
              {addButton}
            </EmptyState>
          ) : (
            <>
              <div className="flex flex-wrap gap-2">{addButton}</div>
              <ProviderList
                providers={list}
                chosenId={ai.providerId}
                onTest={setTesting}
                onEdit={(provider) => setFormMode({ kind: 'edit', provider })}
                onDelete={setDeleting}
              />
            </>
          )}
          <section
            aria-labelledby="ai-disclosure-title"
            className="flex flex-col gap-3 rounded-lg border p-4"
          >
            <h3 id="ai-disclosure-title" className="flex items-center gap-2 font-medium">
              <InfoIcon aria-hidden className="size-4" />
              What the assistant sends
            </h3>
            <WhatIsSent />
          </section>
        </>
      )}
      <ProviderSheet mode={formMode} onClose={() => setFormMode(null)} onTest={setTesting} />
      <ProviderTestDialog provider={testing} onClose={() => setTesting(null)} />
      <DeleteProviderDialog
        provider={deleting}
        chosen={deleting !== null && deleting.id === ai?.providerId}
        onClose={() => setDeleting(null)}
      />
    </SettingsSection>
  )
}

/** On or off, and which provider: changes that send data elsewhere are confirmed first. */
function AssistantSwitch({
  ai,
  providers,
}: {
  ai: AiSettings
  providers: readonly ProviderDetails[]
}) {
  const id = useId()
  const update = useUpdateSettings()
  const [confirming, setConfirming] = useState<AiSettings | null>(null)
  const chosen = providers.find((provider) => provider.id === ai.providerId) ?? null
  const usable = providers.filter((provider) => providerBlocker(provider) === null)

  const save = (next: AiSettings) =>
    update.mutate(
      { ai: next },
      {
        onSuccess: (saved) => {
          const provider = providers.find((p) => p.id === saved.ai.providerId)
          toast.success(
            saved.ai.enabled
              ? `The assistant uses ${provider?.name ?? 'the provider'}`
              : 'AI is off',
          )
        },
        onError: (error) =>
          toast.error("Couldn't change the AI setting", { description: error.message }),
      },
    )

  /** Asks first when the change starts sending data to another computer. */
  const request = (next: AiSettings) => {
    const target = providers.find((provider) => provider.id === next.providerId)
    const startsSending =
      next.enabled &&
      target !== undefined &&
      !isLocalProvider(target) &&
      (!ai.enabled || ai.providerId !== next.providerId)
    if (startsSending) setConfirming(next)
    else save(next)
  }

  const pickId = ai.providerId ?? usable[0]?.id ?? null
  const status = ai.enabled && chosen ? `On · uses ${chosen.name}` : 'Off'
  const hint =
    providers.length === 0
      ? 'Add a provider below to turn the assistant on.'
      : usable.length === 0
        ? 'Save an API key for a provider to turn the assistant on.'
        : chosen && !isLocalProvider(chosen) && ai.enabled
          ? `Questions and schema details go to ${providerHost(chosen)}.`
          : chosen && ai.enabled
            ? 'Everything stays on this computer.'
            : 'Turn on to ask questions about your data in plain language.'
  const target = confirming ? providers.find((p) => p.id === confirming.providerId) : undefined

  return (
    <div className="flex flex-col gap-4 rounded-lg border p-4">
      <Field orientation="horizontal">
        <FieldContent>
          <FieldLabel htmlFor={`${id}-on`}>Use the AI assistant</FieldLabel>
          <FieldDescription>
            <span className="font-medium text-foreground">{status}.</span> {hint}
          </FieldDescription>
        </FieldContent>
        {update.isPending ? (
          <Spinner aria-label="Saving" />
        ) : (
          <Switch
            id={`${id}-on`}
            checked={ai.enabled}
            disabled={pickId === null || (!ai.enabled && usable.length === 0)}
            onCheckedChange={(checked) =>
              pickId && request({ enabled: checked, providerId: pickId })
            }
          />
        )}
      </Field>
      {providers.length > 0 && (
        <Field>
          <FieldLabel htmlFor={`${id}-provider`}>Provider</FieldLabel>
          <Select
            value={pickId ?? undefined}
            disabled={update.isPending}
            onValueChange={(providerId) => request({ enabled: ai.enabled, providerId })}
          >
            <SelectTrigger id={`${id}-provider`} className="min-h-11 w-full sm:min-h-0 sm:max-w-sm">
              <SelectValue placeholder="Choose a provider" />
            </SelectTrigger>
            <SelectContent>
              {providers.map((provider) => {
                const blocker = providerBlocker(provider)
                return (
                  <SelectItem key={provider.id} value={provider.id} disabled={blocker !== null}>
                    {provider.name}
                    <span className="text-muted-foreground">
                      {' '}
                      ·{' '}
                      {blocker ??
                        (isLocalProvider(provider) ? 'on this computer' : providerHost(provider))}
                    </span>
                  </SelectItem>
                )
              })}
            </SelectContent>
          </Select>
        </Field>
      )}
      <AlertDialog open={confirming !== null} onOpenChange={(open) => !open && setConfirming(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              Send questions to {target?.name ?? 'this provider'}?
            </AlertDialogTitle>
            <AlertDialogDescription>
              The assistant will use a hosted provider. Review what it sends first.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <WhatIsSent
            destination={target ? { local: false, host: providerHost(target) } : undefined}
          />
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (confirming) save(confirming)
                setConfirming(null)
              }}
            >
              {ai.enabled ? `Use ${target?.name ?? 'this provider'}` : 'Turn on'}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

function DeleteProviderDialog({
  provider,
  chosen,
  onClose,
}: {
  provider: ProviderDetails | null
  chosen: boolean
  onClose: () => void
}) {
  const queryClient = useQueryClient()
  const remove = useMutation({
    // The dialog closes before this finishes, so the provider travels with the request.
    mutationFn: (target: ProviderDetails) => deleteProvider(target.id),
    onSuccess: (_, target) => toast.success(`Deleted ${target.name}`),
    onError: (error) => toast.error("Couldn't delete the provider", { description: error.message }),
    onSettled: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: PROVIDERS_QUERY_KEY }),
        queryClient.invalidateQueries({ queryKey: SETTINGS_QUERY_KEY }),
      ]),
  })
  return (
    <AlertDialog open={provider !== null} onOpenChange={(open) => !open && onClose()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete {provider?.name}?</AlertDialogTitle>
          <AlertDialogDescription>
            The provider{provider?.key === 'not_saved' ? ' is' : ' and its API key are'} removed
            from this device.
            {chosen ? ' The assistant uses it, so AI will be turned off.' : ''} This cannot be
            undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            onClick={() => provider && remove.mutate(provider)}
          >
            Delete
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
