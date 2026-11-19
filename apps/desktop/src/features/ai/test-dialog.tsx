import { useEffect, useRef } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import type { ProviderDetails, ProviderTestReport } from '@aidb/contracts'
import { CircleAlertIcon } from 'lucide-react'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Spinner } from '@/components/ui/spinner'
import { testProvider } from '@/lib/commands'
import {
  CHECK_LABELS,
  isLocalProvider,
  OUTCOME_DISPLAY,
  STRUCTURED_OUTPUT_LABELS,
} from './provider-display'
import { PROVIDERS_QUERY_KEY } from './queries'

/** Runs "Test provider" when opened and shows each check. */
export function ProviderTestDialog({
  provider,
  onClose,
}: {
  provider: ProviderDetails | null
  onClose: () => void
}) {
  return (
    <Dialog open={provider !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-lg">
        {provider && <TestRun key={provider.id} provider={provider} onClose={onClose} />}
      </DialogContent>
    </Dialog>
  )
}

function TestRun({ provider, onClose }: { provider: ProviderDetails; onClose: () => void }) {
  const queryClient = useQueryClient()
  const test = useMutation({
    mutationFn: () => testProvider(provider.id),
    onSettled: () => queryClient.invalidateQueries({ queryKey: PROVIDERS_QUERY_KEY }),
  })
  // One run per opening; the dialog remounts for each provider.
  const started = useRef(false)
  useEffect(() => {
    if (started.current) return
    started.current = true
    test.mutate()
  }, [test])

  const local = isLocalProvider(provider)
  return (
    <>
      <DialogHeader>
        <DialogTitle>Test {provider.name}</DialogTitle>
        <DialogDescription>
          Sends a few short, fixed prompts to <span className="font-mono">{provider.model}</span>.
          No database content is sent.
          {local ? '' : ' Hosted providers may charge for these requests.'}
        </DialogDescription>
      </DialogHeader>
      {test.isPending && (
        <p className="flex items-center gap-2 text-sm text-muted-foreground" role="status">
          <Spinner />
          Checking the endpoint, the model and what it supports…
        </p>
      )}
      {test.isError && (
        <Alert variant="destructive">
          <CircleAlertIcon />
          <AlertTitle>The test could not run</AlertTitle>
          <AlertDescription>{test.error.message}</AlertDescription>
        </Alert>
      )}
      {test.isSuccess && <Report report={test.data} />}
      <DialogFooter>
        <Button
          variant="outline"
          className="max-sm:min-h-11"
          disabled={test.isPending}
          onClick={() => test.mutate()}
        >
          Test again
        </Button>
        <Button className="max-sm:min-h-11" onClick={onClose}>
          Done
        </Button>
      </DialogFooter>
    </>
  )
}

function Report({ report }: { report: ProviderTestReport }) {
  return (
    <div className="flex flex-col gap-4" role="status">
      <ul className="flex flex-col gap-3">
        {report.checks.map((check) => {
          const outcome = OUTCOME_DISPLAY[check.outcome]
          return (
            <li key={check.kind} className="flex gap-3 text-sm">
              <outcome.icon aria-hidden className={`mt-0.5 size-4 shrink-0 ${outcome.className}`} />
              <div className="min-w-0">
                <p className="font-medium">
                  {CHECK_LABELS[check.kind]}
                  <span className={`font-normal ${outcome.className}`}> · {outcome.label}</span>
                </p>
                <p className="break-words text-muted-foreground">{check.detail}</p>
              </div>
            </li>
          )
        })}
      </ul>
      {report.features && (
        <p className="text-sm text-muted-foreground">
          Saved with the provider: {report.features.toolCalls ? 'tool calls' : 'no tool calls'},{' '}
          {STRUCTURED_OUTPUT_LABELS[report.features.structuredOutput]}.
        </p>
      )}
      {report.models.length > 0 && (
        <Collapsible>
          <CollapsibleTrigger asChild>
            <Button variant="link" size="sm" className="h-auto px-0">
              Models the endpoint lists ({report.models.length})
            </Button>
          </CollapsibleTrigger>
          <CollapsibleContent>
            <ul className="mt-2 max-h-40 overflow-y-auto rounded-md border p-2 font-mono text-xs">
              {report.models.map((model) => (
                <li key={model.id} className="truncate py-0.5">
                  {model.id}
                </li>
              ))}
            </ul>
          </CollapsibleContent>
        </Collapsible>
      )}
    </div>
  )
}
