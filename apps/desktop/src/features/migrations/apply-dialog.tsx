import { useEffect, useEffectEvent, useId, useRef, useState } from 'react'
import type { MigrationResult, SchemaDiff } from '@aidb/contracts'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import {
  CircleAlertIcon,
  CircleCheckIcon,
  Trash2Icon,
  TriangleAlertIcon,
  UnlinkIcon,
} from 'lucide-react'
import { EnvironmentBadge } from '@/components/shared/environment-badge'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Field, FieldContent, FieldDescription, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import type { ActiveConnection } from '@/features/connections/types'
import { applyMigration, cancelMigration, prepareMigration } from '@/lib/commands'
import { errorMessage } from '@/lib/errors'
import { MIGRATION_RECORDS_KEY } from './history'
import { returnFocus } from '@/lib/focus-return'

type ApplyProps = {
  active: ActiveConnection
  diff: SchemaDiff
  /** It ran (whatever the outcome): the comparison may no longer describe the schema. */
  onFinished: (result: MigrationResult) => void
}

/**
 * Applies a comparison's migration after the user reviews what it risks: the typed database
 * name, and an explicit acknowledgement when steps delete data.
 */
export function ApplyMigrationDialog({
  open,
  onOpenChange,
  ...props
}: ApplyProps & { open: boolean; onOpenChange: (open: boolean) => void }) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent {...returnFocus} className="flex max-h-[90dvh] flex-col gap-0 p-0 sm:max-w-xl">
        <DialogHeader className="border-b p-4 sm:p-6">
          <DialogTitle>Apply the migration</DialogTitle>
          <DialogDescription>
            Changes {props.diff.target} to match {props.diff.source}. The steps run in one
            transaction: if one fails, nothing changes.
          </DialogDescription>
        </DialogHeader>
        {/* Mounted while open, so every opening prepares afresh. */}
        {open && <Apply {...props} onClose={() => onOpenChange(false)} />}
      </DialogContent>
    </Dialog>
  )
}

function Apply({ active, diff, onFinished, onClose }: ApplyProps & { onClose: () => void }) {
  const formId = useId()
  const queryClient = useQueryClient()
  const [confirmation, setConfirmation] = useState('')
  const [accepted, setAccepted] = useState(false)
  const [result, setResult] = useState<MigrationResult | null>(null)
  // The approval this dialog holds, declined when it closes unused.
  const held = useRef<string | null>(null)

  const prepare = useMutation({
    mutationFn: () => prepareMigration(active.id, diff.diffId),
    onSuccess: (preview) => {
      held.current = preview.approvalId
    },
  })
  const prepareOnOpen = useEffectEvent(() => prepare.mutate())
  useEffect(() => {
    prepareOnOpen()
    return () => {
      const approvalId = held.current
      held.current = null
      if (approvalId) cancelMigration(approvalId).catch(() => undefined)
    }
  }, [])

  const apply = useMutation({
    mutationFn: (approvalId: string) =>
      applyMigration(active.id, approvalId, confirmation.trim() || null, accepted),
    onSuccess: (outcome) => {
      held.current = null
      setResult(outcome)
      onFinished(outcome)
      void queryClient.invalidateQueries({ queryKey: MIGRATION_RECORDS_KEY })
      void queryClient.invalidateQueries({ queryKey: ['audit'] })
      void queryClient.invalidateQueries({ queryKey: ['catalog', active.id] })
      void queryClient.invalidateQueries({ queryKey: ['tablePage', active.id] })
    },
  })

  const preview = prepare.data
  const needsName = preview?.confirmation ?? null
  const confirmed = needsName === null || confirmation.trim() === needsName
  const losing = preview?.dataLoss ?? []
  const ready = preview && confirmed && (losing.length === 0 || accepted)

  return (
    <>
      <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-4 sm:p-6">
        {prepare.isPending && (
          <div className="flex flex-col gap-2" role="status">
            <span className="flex items-center gap-2 text-sm text-muted-foreground">
              <Spinner />
              Checking the migration against the policy…
            </span>
            <Skeleton className="h-12 w-full" />
          </div>
        )}
        {prepare.isError && (
          <Alert variant="destructive">
            <CircleAlertIcon />
            <AlertTitle>This migration cannot run here</AlertTitle>
            <AlertDescription>{errorMessage(prepare.error)}</AlertDescription>
          </Alert>
        )}
        {result ? (
          <Outcome result={result} />
        ) : (
          preview && (
            <>
              <section aria-label="Risk" className="flex flex-col gap-2">
                <div className="flex flex-wrap items-center gap-2 text-sm text-muted-foreground">
                  <EnvironmentBadge environment={active.environment} />
                  <Badge variant={preview.risk >= 'R3' ? 'destructive' : 'secondary'}>
                    Risk {preview.risk}
                  </Badge>
                  <span>
                    {preview.steps === 1 ? '1 step' : `${preview.steps} steps`}
                    {preview.outsideTransaction > 0
                      ? `, ${preview.outsideTransaction} after the transaction`
                      : ''}
                    .
                  </span>
                </div>
                {preview.reasons.length > 0 && (
                  <ul className="list-disc pl-5 text-sm text-muted-foreground">
                    {preview.reasons.map((reason) => (
                      <li key={reason}>{reason}</li>
                    ))}
                  </ul>
                )}
              </section>
              {preview.outsideTransaction > 0 && (
                <Alert>
                  <UnlinkIcon />
                  <AlertTitle>Some steps run after the transaction</AlertTitle>
                  <AlertDescription>
                    Index builds run once the rest has committed and are not rolled back if one
                    fails.
                  </AlertDescription>
                </Alert>
              )}
              {losing.length > 0 && (
                <Alert variant="destructive">
                  <Trash2Icon />
                  <AlertTitle>
                    {losing.length === 1
                      ? '1 step deletes data'
                      : `${losing.length} steps delete data`}
                  </AlertTitle>
                  <AlertDescription>
                    <ul className="list-disc pl-5">
                      {losing.map((summary) => (
                        <li key={summary}>{summary}</li>
                      ))}
                    </ul>
                    The data cannot be brought back from here. A renamed column or table shows as
                    one removed and another added.
                  </AlertDescription>
                </Alert>
              )}
              {losing.length > 0 && (
                <Field orientation="horizontal">
                  <Checkbox
                    id={`${formId}-accept`}
                    checked={accepted}
                    onCheckedChange={(checked) => setAccepted(checked === true)}
                  />
                  <FieldContent>
                    <FieldLabel htmlFor={`${formId}-accept`}>
                      I accept that these steps delete data
                    </FieldLabel>
                  </FieldContent>
                </Field>
              )}
              {needsName !== null && (
                <Field>
                  <FieldLabel htmlFor={`${formId}-confirm`}>Type {needsName} to confirm</FieldLabel>
                  <Input
                    id={`${formId}-confirm`}
                    value={confirmation}
                    autoComplete="off"
                    spellCheck={false}
                    className="min-h-11 sm:min-h-0"
                    onChange={(event) => setConfirmation(event.target.value)}
                  />
                  <FieldDescription>
                    A schema change needs the database name typed.
                  </FieldDescription>
                </Field>
              )}
              {apply.isError && (
                <Alert variant="destructive">
                  <CircleAlertIcon />
                  <AlertTitle>Not applied</AlertTitle>
                  <AlertDescription>{errorMessage(apply.error)}</AlertDescription>
                </Alert>
              )}
            </>
          )
        )}
      </div>

      <DialogFooter className="flex-row flex-wrap border-t p-4 sm:p-6">
        {result ? (
          <Button className="min-h-11 flex-1 sm:min-h-0 sm:flex-none" onClick={onClose}>
            Close
          </Button>
        ) : (
          <>
            <Button
              variant="outline"
              className="min-h-11 flex-1 sm:min-h-0 sm:flex-none"
              disabled={apply.isPending}
              onClick={onClose}
            >
              Cancel
            </Button>
            <Button
              variant={losing.length > 0 ? 'destructive' : 'default'}
              className="min-h-11 flex-1 sm:min-h-0 sm:flex-none"
              disabled={!ready || apply.isPending}
              onClick={() => preview && apply.mutate(preview.approvalId)}
            >
              {apply.isPending && <Spinner data-icon="inline-start" />}
              {apply.isPending ? 'Applying…' : 'Apply migration'}
            </Button>
          </>
        )}
      </DialogFooter>
    </>
  )
}

function Outcome({ result }: { result: MigrationResult }) {
  if (result.status === 'succeeded') {
    return (
      <Alert>
        {result.verification === 'failed' ? <TriangleAlertIcon /> : <CircleCheckIcon />}
        <AlertTitle>
          {result.verification === 'failed' ? 'Applied, but differences remain' : 'Applied'}
        </AlertTitle>
        <AlertDescription>{result.message}</AlertDescription>
      </Alert>
    )
  }
  const titles = {
    failed: 'Rolled back: nothing changed',
    partial: 'Partly applied',
    uncertain: 'The outcome is unknown',
    running: 'Still running',
  } as const
  return (
    <Alert variant="destructive">
      {result.status === 'failed' ? <CircleAlertIcon /> : <TriangleAlertIcon />}
      <AlertTitle>{titles[result.status]}</AlertTitle>
      <AlertDescription>
        {result.failedStep && <p className="font-medium">“{result.failedStep.summary}” failed.</p>}
        <p>{result.message}</p>
      </AlertDescription>
    </Alert>
  )
}
