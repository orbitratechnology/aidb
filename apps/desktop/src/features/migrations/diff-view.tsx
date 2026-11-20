import type { MigrationStep, NoteKind, SchemaDiff, StepAction } from '@aidb/contracts'
import { useMutation } from '@tanstack/react-query'
import {
  CircleCheckIcon,
  CopyIcon,
  DownloadIcon,
  InfoIcon,
  LockIcon,
  MinusIcon,
  PencilIcon,
  PlusIcon,
  TriangleAlertIcon,
  Trash2Icon,
  UnlinkIcon,
  type LucideIcon,
} from 'lucide-react'
import { toast } from 'sonner'
import { EnvironmentBadge } from '@/components/shared/environment-badge'
import { EmptyState } from '@/components/shared/status-states'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { CodeBlock, CodeBlockContent } from '@/components/ui/code-block'
import { Item, ItemContent, ItemMedia, ItemTitle } from '@/components/ui/item'
import { Spinner } from '@/components/ui/spinner'
import type { EnvironmentKind } from '@/features/connections/types'
import { copyText } from '@/lib/clipboard'
import { saveSchemaScript } from '@/lib/commands'
import { errorMessage } from '@/lib/errors'
import { NOTE_LABELS, stepsWith, summaryParts } from './compare'

const ACTION_DISPLAY: Record<StepAction, { icon: LucideIcon; label: string }> = {
  add: { icon: PlusIcon, label: 'Add' },
  change: { icon: PencilIcon, label: 'Change' },
  remove: { icon: MinusIcon, label: 'Remove' },
}

const NOTE_ICONS: Record<NoteKind, LucideIcon> = {
  data_loss: Trash2Icon,
  lock: LockIcon,
  may_fail: TriangleAlertIcon,
  non_transactional: UnlinkIcon,
}

/** A comparison: what differs, the steps that make the target match, and the whole script. */
export function DiffView({
  diff,
  environment,
}: {
  diff: SchemaDiff
  environment: EnvironmentKind
}) {
  const save = useMutation({
    mutationFn: () => saveSchemaScript(diff.diffId),
    onSuccess: (name) => name && toast.success(`Saved ${name}.`),
    onError: (error) => toast.error(errorMessage(error)),
  })
  const losing = stepsWith(diff.steps, 'data_loss')
  const outside = diff.steps.filter((step) => !step.transactional)

  return (
    <section aria-labelledby="diff-heading" className="flex flex-col gap-4">
      <div className="flex flex-col gap-1">
        <h2 id="diff-heading" className="text-base font-medium">
          {diff.target} compared with {diff.source}
        </h2>
        {diff.steps.length > 0 && (
          <div className="flex flex-wrap items-center gap-2 text-sm text-muted-foreground">
            <EnvironmentBadge environment={environment} />
            <Badge variant={diff.risk >= 'R3' ? 'destructive' : 'secondary'}>
              Risk {diff.risk}
            </Badge>
            <span>
              {diff.steps.length === 1 ? '1 step' : `${diff.steps.length} steps`}:{' '}
              {summaryParts(diff.summary).join(', ')}.
            </span>
          </div>
        )}
      </div>

      {diff.steps.length === 0 ? (
        <EmptyState
          icon={CircleCheckIcon}
          title="They match"
          description={`Nothing to change: ${diff.target} already has what ${diff.source} has, as far as aidb compares.`}
        />
      ) : (
        <>
          {losing.length > 0 && (
            <Alert variant="destructive">
              <Trash2Icon />
              <AlertTitle>
                {losing.length === 1 ? '1 step deletes data' : `${losing.length} steps delete data`}
              </AlertTitle>
              <AlertDescription>
                <ul className="list-disc pl-5">
                  {losing.map((step) => (
                    <li key={step.sql}>{step.summary}</li>
                  ))}
                </ul>
                A renamed column or table shows as one removed and another added: check before
                running.
              </AlertDescription>
            </Alert>
          )}
          {outside.length > 0 && (
            <Alert>
              <UnlinkIcon />
              <AlertTitle>
                {outside.length === 1
                  ? '1 step runs outside the transaction'
                  : `${outside.length} steps run outside the transaction`}
              </AlertTitle>
              <AlertDescription>
                They come last, one by one, and are not rolled back if something fails.
              </AlertDescription>
            </Alert>
          )}
        </>
      )}

      {diff.notes.length > 0 && (
        <Alert>
          <InfoIcon />
          <AlertTitle>Not compared</AlertTitle>
          <AlertDescription>
            <ul className="list-disc pl-5">
              {diff.notes.map((note) => (
                <li key={note}>{note}</li>
              ))}
            </ul>
          </AlertDescription>
        </Alert>
      )}

      {diff.steps.length > 0 && (
        <>
          <ol aria-label="Steps" className="flex flex-col gap-2">
            {diff.steps.map((step, index) => (
              <li key={`${index}:${step.sql}`}>
                <StepItem step={step} number={index + 1} />
              </li>
            ))}
          </ol>
          <div className="flex flex-col gap-3 rounded-lg border p-4">
            <p className="text-sm text-muted-foreground">
              The script runs these steps in this order in one transaction, then any that cannot run
              in one. aidb does not run it here: review it, then run it with your usual tool, such
              as psql.
            </p>
            <div className="flex flex-wrap gap-2">
              <Button
                variant="outline"
                className="min-h-11 sm:min-h-0"
                onClick={() =>
                  void copyText(diff.script).then(
                    () => toast.success('Script copied.'),
                    (error: unknown) => toast.error(errorMessage(error)),
                  )
                }
              >
                <CopyIcon data-icon="inline-start" />
                Copy script
              </Button>
              <Button
                variant="outline"
                className="min-h-11 sm:min-h-0"
                disabled={save.isPending}
                onClick={() => save.mutate()}
              >
                {save.isPending ? (
                  <Spinner data-icon="inline-start" />
                ) : (
                  <DownloadIcon data-icon="inline-start" />
                )}
                Save script…
              </Button>
            </div>
          </div>
        </>
      )}
    </section>
  )
}

function StepItem({ step, number }: { step: MigrationStep; number: number }) {
  const { icon: Icon, label } = ACTION_DISPLAY[step.action]
  return (
    <Item variant="outline" size="sm" className="items-start">
      <ItemMedia variant="icon">
        <Icon />
      </ItemMedia>
      <ItemContent className="min-w-0 gap-2">
        <ItemTitle className="flex flex-wrap items-center gap-2">
          <span className="text-muted-foreground tabular-nums">{number}.</span>
          <span className="break-words">{step.summary}</span>
          <Badge variant={step.action === 'remove' ? 'destructive' : 'outline'}>{label}</Badge>
          <Badge variant={step.risk >= 'R3' ? 'destructive' : 'secondary'}>Risk {step.risk}</Badge>
        </ItemTitle>
        {step.notes.length > 0 && (
          <ul className="flex flex-col gap-1 text-sm text-muted-foreground">
            {step.notes.map((note) => {
              const NoteIcon = NOTE_ICONS[note.kind]
              return (
                <li key={`${note.kind}:${note.text}`} className="flex items-start gap-2">
                  <NoteIcon
                    aria-hidden
                    className={
                      note.kind === 'data_loss'
                        ? 'mt-0.5 size-4 shrink-0 text-destructive'
                        : 'mt-0.5 size-4 shrink-0'
                    }
                  />
                  <span>
                    <span className="font-medium text-foreground">{NOTE_LABELS[note.kind]}:</span>{' '}
                    {note.text}
                  </span>
                </li>
              )
            })}
          </ul>
        )}
        <CodeBlock size="sm">
          <CodeBlockContent wrap className="max-h-40 text-xs">
            {step.sql}
          </CodeBlockContent>
        </CodeBlock>
      </ItemContent>
    </Item>
  )
}
