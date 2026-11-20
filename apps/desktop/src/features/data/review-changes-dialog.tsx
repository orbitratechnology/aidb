import { useEffect, useEffectEvent, useId, useRef, useState } from 'react'
import type {
  ConnectionId,
  DataChange,
  DataChangePreview,
  DataChangeResult,
  Environment,
  ValueSample,
} from '@aidb/contracts'
import { useMutation } from '@tanstack/react-query'
import {
  AlertTriangleIcon,
  ArrowRightIcon,
  ChevronDownIcon,
  CircleAlertIcon,
  PencilIcon,
  PlusIcon,
  Trash2Icon,
} from 'lucide-react'
import { EnvironmentBadge } from '@/components/shared/environment-badge'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { CodeBlock, CodeBlockContent } from '@/components/ui/code-block'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Field, FieldDescription, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Item, ItemContent, ItemGroup, ItemMedia, ItemTitle } from '@/components/ui/item'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { applyDataChange, cancelDataChange, previewDataChange } from '@/lib/commands'
import { CellValue } from './cell-value'
import { failedChange, staleChanges, type ReviewItem } from './edits'

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

const APPROVAL_TEXT: Record<string, string> = {
  apply: 'Saving needs your approval here.',
  elevated: 'An elevated approval: type the database name to confirm.',
}

/**
 * The pending changes as the database sees them now, before anything is written: each change
 * before and after, the rows it writes (counted, not estimated), its risk and SQL. Saving
 * applies them together in one transaction, all or none.
 */
export function ReviewChangesDialog({
  open,
  onOpenChange,
  connectionId,
  environment,
  changes,
  items,
  onApplied,
  onReload,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  connectionId: ConnectionId
  environment: Environment
  changes: DataChange[]
  /** The same changes for display, in the same order. */
  items: ReviewItem[]
  onApplied: (result: DataChangeResult) => void
  /** Reloads the rows, after an uncertain outcome. */
  onReload: () => void
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[90dvh] flex-col gap-0 p-0 sm:max-w-2xl">
        <DialogHeader className="border-b p-4 sm:p-6">
          <DialogTitle>Review changes</DialogTitle>
          <DialogDescription>
            Nothing has been written yet. Saving runs these changes together in one transaction: all
            of them, or none.
          </DialogDescription>
        </DialogHeader>
        {/* Mounted while open, so every review plans the changes afresh. */}
        {open && (
          <Review
            connectionId={connectionId}
            environment={environment}
            changes={changes}
            items={items}
            onClose={() => onOpenChange(false)}
            onApplied={onApplied}
            onReload={onReload}
          />
        )}
      </DialogContent>
    </Dialog>
  )
}

type Outcome = { result: DataChangeResult } | { error: unknown }

function Review({
  connectionId,
  environment,
  changes,
  items,
  onClose,
  onApplied,
  onReload,
}: {
  connectionId: ConnectionId
  environment: Environment
  changes: DataChange[]
  items: ReviewItem[]
  onClose: () => void
  onApplied: (result: DataChangeResult) => void
  onReload: () => void
}) {
  const confirmId = useId()
  const [confirmation, setConfirmation] = useState('')
  const [outcome, setOutcome] = useState<Outcome | null>(null)
  const preview = useMutation({ mutationFn: () => previewDataChange(connectionId, changes) })
  const apply = useMutation({
    mutationFn: (approvalId: string) =>
      applyDataChange(connectionId, approvalId, confirmation.trim() || null),
  })
  // The approval held for this review, until it is used or declined.
  const held = useRef<string | null>(null)

  const startPlanning = () =>
    preview.mutate(undefined, {
      onSuccess: (planned) => {
        held.current = planned.approvalId
      },
    })
  const plan = () => {
    setOutcome(null)
    setConfirmation('')
    startPlanning()
  }
  // Planned once per opening; the changes cannot change while the dialog is open.
  const planOnOpen = useEffectEvent(startPlanning)
  useEffect(() => {
    planOnOpen()
    return () => {
      // Closed without saving: decline the held approval (best effort; it also expires).
      const approvalId = held.current
      held.current = null
      if (approvalId) cancelDataChange(approvalId).catch(() => undefined)
    }
  }, [])

  const planned = preview.data
  const stale = planned ? staleChanges(changes, planned) : []
  const failedIndex = preview.error !== null ? failedChange(errorMessage(preview.error)) : null
  const deletes = changes.filter((change) => change.kind === 'delete').length
  const needsName = planned?.confirmation ?? null
  const confirmed = needsName === null || confirmation.trim() === needsName
  const done = outcome !== null

  const save = () => {
    if (!planned) return
    apply.mutate(planned.approvalId, {
      onSuccess: (result) => {
        // Used, whatever the outcome: nothing to decline any more.
        held.current = null
        if (result.status === 'applied') onApplied(result)
        else setOutcome({ result })
      },
      onError: (error) => {
        // A refusal before anything ran: a typed name stays open, anything else is spent.
        if (!(needsName !== null && !confirmed)) held.current = null
        setOutcome({ error })
      },
    })
  }

  return (
    <>
      <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto p-4 sm:p-6">
        {preview.isPending && <Planning />}
        {preview.isError && (
          <Alert variant="destructive">
            <CircleAlertIcon />
            <AlertTitle>These changes cannot be saved</AlertTitle>
            <AlertDescription>{errorMessage(preview.error)}</AlertDescription>
          </Alert>
        )}
        {planned && <Assessment preview={planned} environment={environment} />}
        {stale.length > 0 && (
          <Alert variant="destructive">
            <AlertTriangleIcon />
            <AlertTitle>
              {stale.length === 1
                ? 'A row changed after it was loaded'
                : `${stale.length} rows changed after they were loaded`}
            </AlertTitle>
            <AlertDescription>
              {stale.length === 1 ? 'It was' : 'They were'} changed or deleted in the database
              since. Reload the rows and make these edits again; nothing was saved.
            </AlertDescription>
          </Alert>
        )}
        {outcome && <OutcomeAlert outcome={outcome} />}

        <ItemGroup className="gap-2" aria-label="Changes">
          {items.map((item, index) => (
            <ChangeItem
              key={`${item.kind}:${item.id}`}
              item={item}
              rows={planned?.changes[index]?.rows}
              stored={storedValues(planned?.changes[index]?.after ?? null)}
              stale={stale.includes(index)}
              failed={failedIndex === index}
            />
          ))}
        </ItemGroup>

        {planned && (
          <Collapsible>
            <CollapsibleTrigger asChild>
              <Button variant="ghost" size="sm" className="group self-start">
                <ChevronDownIcon
                  data-icon="inline-start"
                  className="transition-transform group-data-[state=open]:rotate-180"
                />
                Show the SQL
              </Button>
            </CollapsibleTrigger>
            <CollapsibleContent className="flex flex-col gap-2 pt-2">
              <p className="text-sm text-muted-foreground">
                Values are sent separately as parameters ($1, $2, …), never inside the SQL.
              </p>
              {planned.changes.map((change, index) => (
                <CodeBlock key={index} size="sm">
                  <CodeBlockContent wrap className="max-h-40 text-xs">
                    {change.statement}
                  </CodeBlockContent>
                </CodeBlock>
              ))}
            </CollapsibleContent>
          </Collapsible>
        )}

        {planned && needsName !== null && !done && (
          <Field>
            <FieldLabel htmlFor={confirmId}>Type {needsName} to confirm</FieldLabel>
            <Input
              id={confirmId}
              value={confirmation}
              autoComplete="off"
              spellCheck={false}
              className="min-h-11 sm:min-h-0"
              onChange={(event) => setConfirmation(event.target.value)}
            />
            <FieldDescription>
              This change is elevated, so it needs the database name typed.
            </FieldDescription>
          </Field>
        )}
      </div>

      <DialogFooter className="flex-row flex-wrap border-t p-4 sm:p-6">
        {outcome && 'result' in outcome && outcome.result.status === 'uncertain' ? (
          <Button
            className="min-h-11 flex-1 sm:min-h-0 sm:flex-none"
            onClick={() => {
              onReload()
              onClose()
            }}
          >
            Close and reload rows
          </Button>
        ) : done || preview.isError ? (
          <>
            <Button
              variant="outline"
              className="min-h-11 flex-1 sm:min-h-0 sm:flex-none"
              onClick={onClose}
            >
              Close
            </Button>
            <Button
              className="min-h-11 flex-1 sm:min-h-0 sm:flex-none"
              disabled={preview.isPending}
              onClick={plan}
            >
              Review again
            </Button>
          </>
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
              variant={deletes > 0 ? 'destructive' : 'default'}
              className="min-h-11 flex-1 sm:min-h-0 sm:flex-none"
              disabled={!planned || stale.length > 0 || !confirmed || apply.isPending}
              onClick={save}
            >
              {apply.isPending && <Spinner data-icon="inline-start" />}
              {saveLabel(changes.length, deletes)}
            </Button>
          </>
        )}
      </DialogFooter>
    </>
  )
}

function saveLabel(total: number, deletes: number): string {
  const changes = `${total} ${total === 1 ? 'change' : 'changes'}`
  if (deletes === 0) return `Save ${changes}`
  const rows = `${deletes} ${deletes === 1 ? 'row' : 'rows'}`
  return deletes === total ? `Delete ${rows}` : `Save ${changes}, deleting ${rows}`
}

function Planning() {
  return (
    <div className="flex flex-col gap-2" role="status">
      <span className="flex items-center gap-2 text-sm text-muted-foreground">
        <Spinner />
        Checking the changes against the database…
      </span>
      <Skeleton className="h-12 w-full" />
    </div>
  )
}

function Assessment({
  preview,
  environment,
}: {
  preview: DataChangePreview
  environment: Environment
}) {
  return (
    <section aria-label="Risk" className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <EnvironmentBadge environment={environment} />
        <Badge variant={preview.risk >= 'R3' ? 'destructive' : 'secondary'}>
          Risk {preview.risk}
        </Badge>
        <span className="text-sm text-muted-foreground">
          {preview.rows === 1 ? '1 row' : `${preview.rows} rows`} in all.{' '}
          {APPROVAL_TEXT[preview.approval] ?? ''}
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
  )
}

function OutcomeAlert({ outcome }: { outcome: Outcome }) {
  if ('error' in outcome) {
    return (
      <Alert variant="destructive">
        <CircleAlertIcon />
        <AlertTitle>Not saved</AlertTitle>
        <AlertDescription>{errorMessage(outcome.error)}</AlertDescription>
      </Alert>
    )
  }
  const { result } = outcome
  return result.status === 'uncertain' ? (
    <Alert variant="destructive">
      <AlertTriangleIcon />
      <AlertTitle>The outcome is unknown</AlertTitle>
      <AlertDescription>{result.message}</AlertDescription>
    </Alert>
  ) : (
    <Alert variant="destructive">
      <CircleAlertIcon />
      <AlertTitle>Nothing was saved</AlertTitle>
      <AlertDescription>
        {result.message} The transaction was rolled back; your changes are still pending.
      </AlertDescription>
    </Alert>
  )
}

const ITEM_DISPLAY = {
  update: { icon: PencilIcon, badge: 'Edit' },
  delete: { icon: Trash2Icon, badge: 'Delete' },
  insert: { icon: PlusIcon, badge: 'New' },
} as const

/** The first row of values as the database will store them, by column. */
function storedValues(sample: ValueSample | null): Map<string, string | null> {
  const row = sample?.rows[0] ?? []
  return new Map(sample?.columns.map((column, index) => [column, row[index] ?? null]) ?? [])
}

function ChangeItem({
  item,
  rows,
  stored,
  stale,
  failed,
}: {
  item: ReviewItem
  rows: number | undefined
  /** How the database will store the values, once planned. */
  stored: Map<string, string | null>
  stale: boolean
  failed: boolean
}) {
  const { icon: Icon, badge } = ITEM_DISPLAY[item.kind]
  return (
    <Item variant="outline" size="sm" className="items-start">
      <ItemMedia variant="icon">
        <Icon />
      </ItemMedia>
      <ItemContent className="min-w-0">
        <ItemTitle className="flex flex-wrap items-center gap-2">
          <Badge variant={item.kind === 'delete' ? 'destructive' : 'outline'}>{badge}</Badge>
          <span className="break-all">{item.label}</span>
          {stale && (
            <Badge variant="destructive">
              {rows === 0 ? 'Changed or deleted since loaded' : `Matches ${rows} rows`}
            </Badge>
          )}
          {failed && <Badge variant="destructive">Refused</Badge>}
        </ItemTitle>
        <div className="text-sm text-muted-foreground">
          <ChangeDetail item={item} stored={stored} />
        </div>
      </ItemContent>
    </Item>
  )
}

/** "stored as 25.00", when the database stores a value differently than it was typed. */
function StoredAs({ typed, stored }: { typed: string | null; stored: string | null | undefined }) {
  if (stored === undefined || stored === typed) return null
  return (
    <span>
      (stored as <CellValue value={stored} kind="text" className="text-foreground" />)
    </span>
  )
}

function ChangeDetail({ item, stored }: { item: ReviewItem; stored: Map<string, string | null> }) {
  if (item.kind === 'update') {
    return (
      <dl className="flex flex-col gap-1 pt-1">
        {item.cells.map(({ column, before, after }) => (
          <div key={column} className="flex min-w-0 flex-wrap items-baseline gap-x-2">
            <dt className="font-medium text-foreground">{column}</dt>
            <dd className="flex min-w-0 flex-wrap items-baseline gap-x-2">
              <span className="sr-only">from</span>
              {before === undefined ? (
                <span className="italic">not loaded</span>
              ) : (
                <CellValue value={before} kind="text" className="line-through" />
              )}
              <ArrowRightIcon aria-hidden className="size-3.5 shrink-0 self-center" />
              <span className="sr-only">to</span>
              <CellValue value={after} kind="text" className="text-foreground" />
              <StoredAs typed={after} stored={stored.get(column)} />
            </dd>
          </div>
        ))}
      </dl>
    )
  }
  const values = (
    <dl className="flex flex-col gap-1 pt-1">
      {item.values.map(({ column, value }) => (
        <div key={column} className="flex min-w-0 flex-wrap items-baseline gap-x-2">
          <dt className="font-medium text-foreground">{column}</dt>
          <dd className="flex min-w-0 flex-wrap items-baseline gap-x-2">
            <CellValue value={value} kind="text" />
            {item.kind === 'insert' && <StoredAs typed={value} stored={stored.get(column)} />}
          </dd>
        </div>
      ))}
    </dl>
  )
  if (item.kind === 'delete') return values
  return (
    <>
      {values}
      {item.defaults.length > 0 && (
        <p className="pt-1">Defaults for: {item.defaults.join(', ')}.</p>
      )}
    </>
  )
}
