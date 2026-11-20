import { useEffect, useEffectEvent, useId, useRef, useState } from 'react'
import type {
  ConnectionId,
  DataChange,
  DataChangePreview,
  DataChangeResult,
  Environment,
  PendingChange,
  PlannedChange,
  TablePage,
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
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { applyDataChange, cancelDataChange, previewDataChange } from '@/lib/commands'
import { errorMessage } from '@/lib/errors'
import { CellValue } from './cell-value'
import { failedChange, staleChanges, type ReviewItem } from './edits'

const APPROVAL_TEXT: Record<string, string> = {
  apply: 'It needs your approval here.',
  elevated: 'An elevated approval: type the database name to confirm.',
}

/** What is reviewed: grid edits planned on opening, or changes already held by the core. */
type ReviewProps = {
  connectionId: ConnectionId
  environment: Environment
  changes: DataChange[]
  /** The same changes for display, in the same order. */
  items: ReviewItem[]
  /**
   * Changes the core already holds for approval (the assistant's proposal), shown as planned.
   * Closing leaves them waiting; Decline declines them. Without it, the changes are planned on
   * opening, each edit must still match exactly one row, and closing declines them.
   */
  held?: PendingChange | null
  onApplied: (result: DataChangeResult) => void
  /** Reloads the rows, after an uncertain outcome. */
  onReload: () => void
  /** The held changes were declined. */
  onDeclined?: () => void
}

/**
 * Changes as the database sees them now, before anything is written: each change before and
 * after, the rows it writes (counted, not estimated), its risk and SQL. Applying runs them
 * together in one transaction, all or none.
 */
export function ReviewChangesDialog({
  open,
  onOpenChange,
  ...props
}: ReviewProps & { open: boolean; onOpenChange: (open: boolean) => void }) {
  const proposal = props.held?.actor === 'agent'
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[90dvh] flex-col gap-0 p-0 sm:max-w-2xl">
        <DialogHeader className="border-b p-4 sm:p-6">
          <DialogTitle>{proposal ? 'Review the assistant’s change' : 'Review changes'}</DialogTitle>
          <DialogDescription>
            {proposal ? 'The assistant prepared this; it cannot apply it. ' : ''}Nothing has been
            written yet. Applying runs these changes together in one transaction: all of them, or
            none.
          </DialogDescription>
        </DialogHeader>
        {/* Mounted while open, so every review starts afresh. */}
        {open && <Review {...props} onClose={() => onOpenChange(false)} />}
      </DialogContent>
    </Dialog>
  )
}

export type Outcome = { result: DataChangeResult } | { error: unknown }

function Review({
  connectionId,
  environment,
  changes,
  items,
  held: pending = null,
  onClose,
  onApplied,
  onReload,
  onDeclined,
}: ReviewProps & { onClose: () => void }) {
  const confirmId = useId()
  const [confirmation, setConfirmation] = useState('')
  const [outcome, setOutcome] = useState<Outcome | null>(null)
  const preview = useMutation({ mutationFn: () => previewDataChange(connectionId, changes) })
  const apply = useMutation({
    mutationFn: (approvalId: string) =>
      applyDataChange(connectionId, approvalId, confirmation.trim() || null),
  })
  // An approval this dialog asked for, declined when it closes unused. Changes the core held
  // before the dialog opened are not this dialog's to decline on close.
  const held = useRef<string | null>(null)
  const [replanned, setReplanned] = useState(false)
  const decline = useMutation({ mutationFn: (approvalId: string) => cancelDataChange(approvalId) })

  const startPlanning = () =>
    preview.mutate(undefined, {
      onSuccess: (planned) => {
        held.current = planned.approvalId
      },
    })
  const plan = () => {
    setOutcome(null)
    setConfirmation('')
    setReplanned(true)
    startPlanning()
  }
  // Planned once per opening (held changes are already planned); the changes cannot change
  // while the dialog is open.
  const planOnOpen = useEffectEvent(() => {
    if (!pending) startPlanning()
  })
  useEffect(() => {
    planOnOpen()
    return () => {
      // Closed without saving: decline the held approval (best effort; it also expires).
      const approvalId = held.current
      held.current = null
      if (approvalId) cancelDataChange(approvalId).catch(() => undefined)
    }
  }, [])

  const planned = pending && !replanned ? pending.preview : preview.data
  // Grid edits address one row each; the assistant's filters may match any number.
  const stale = planned && !pending ? staleChanges(changes, planned) : []
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
              samples={pending ? planned?.changes[index] : undefined}
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
        ) : done || preview.isError || decline.isError ? (
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
              disabled={apply.isPending || decline.isPending}
              onClick={onClose}
            >
              {pending && !replanned ? 'Later' : 'Cancel'}
            </Button>
            {pending && !replanned && (
              <Button
                variant="outline"
                className="min-h-11 flex-1 sm:min-h-0 sm:flex-none"
                disabled={apply.isPending || decline.isPending}
                onClick={() =>
                  decline.mutate(pending.preview.approvalId, {
                    onSuccess: () => {
                      onDeclined?.()
                      onClose()
                    },
                    onError: (error) => setOutcome({ error }),
                  })
                }
              >
                {decline.isPending && <Spinner data-icon="inline-start" />}
                Decline
              </Button>
            )}
            <Button
              variant={deletes > 0 ? 'destructive' : 'default'}
              className="min-h-11 flex-1 sm:min-h-0 sm:flex-none"
              disabled={!planned || stale.length > 0 || !confirmed || apply.isPending}
              onClick={save}
            >
              {apply.isPending && <Spinner data-icon="inline-start" />}
              {saveLabel(changes.length, deletes, pending ? 'Apply' : 'Save')}
            </Button>
          </>
        )}
      </DialogFooter>
    </>
  )
}

function saveLabel(total: number, deletes: number, verb: 'Save' | 'Apply'): string {
  const changes = `${total} ${total === 1 ? 'change' : 'changes'}`
  if (deletes === 0) return `${verb} ${changes}`
  const deleted = `${deletes} ${deletes === 1 ? 'delete' : 'deletes'}`
  return deletes === total ? `${verb} ${deleted}` : `${verb} ${changes}, with ${deleted}`
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

/** The environment, risk, rows written and why, for changes planned and held for approval. */
export function Assessment({
  preview,
  environment,
}: {
  preview: Pick<DataChangePreview, 'risk' | 'approval' | 'reasons' | 'rows'>
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

export function OutcomeAlert({
  outcome,
  rolledBack = 'The transaction was rolled back; your changes are still pending.',
}: {
  outcome: Outcome
  /** What a refused apply leaves behind. */
  rolledBack?: string
}) {
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
        {result.message} {rolledBack}
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
  samples,
  stale,
  failed,
}: {
  item: ReviewItem
  rows: number | undefined
  /** How the database will store the values, once planned. */
  stored: Map<string, string | null>
  /** The rows it matches now and the values it stores, for changes not made in the grid. */
  samples?: PlannedChange
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
        {samples && rows !== undefined && (
          <p className="text-sm">
            {item.kind === 'insert' ? 'Adds' : item.kind === 'delete' ? 'Deletes' : 'Changes'}{' '}
            {rows === 1 ? '1 row' : `${rows} rows`}.
          </p>
        )}
        {samples?.before && samples.before.rows.length > 0 && (
          <SampleTable
            caption={
              item.kind === 'delete'
                ? 'Rows it deletes, as they are now'
                : 'Rows it changes, as they are now'
            }
            columns={samples.before.columns.map((column) => column.name)}
            rows={samples.before.rows}
            more={samples.before.hasMore}
          />
        )}
        {samples?.after && item.kind === 'insert' && (
          <SampleTable
            caption="Rows it adds, as they will be stored"
            columns={samples.after.columns}
            rows={samples.after.rows}
            more={samples.after.totalRows > samples.after.rows.length}
          />
        )}
      </ItemContent>
    </Item>
  )
}

/** A few rows, scrolling sideways when wide. */
export function SampleTable({
  caption,
  columns,
  rows,
  more,
}: {
  caption: string
  columns: string[]
  rows: TablePage['rows']
  more: boolean
}) {
  return (
    <figure className="flex min-w-0 flex-col gap-1">
      <figcaption className="text-xs text-muted-foreground">
        {caption}
        {more ? ' (the first ones)' : ''}
      </figcaption>
      <div className="max-h-56 overflow-auto rounded-md border">
        <Table className="text-xs">
          <TableHeader>
            <TableRow>
              {columns.map((column) => (
                <TableHead key={column} className="h-8">
                  {column}
                </TableHead>
              ))}
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row, index) => (
              <TableRow key={index}>
                {row.map((value, column) => (
                  <TableCell key={column} className="max-w-48 py-1.5">
                    <CellValue value={value} kind="text" />
                  </TableCell>
                ))}
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </div>
    </figure>
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
              {before === undefined ? (
                <span>set to</span>
              ) : (
                <>
                  <span className="sr-only">from</span>
                  <CellValue value={before} kind="text" className="line-through" />
                  <ArrowRightIcon aria-hidden className="size-3.5 shrink-0 self-center" />
                  <span className="sr-only">to</span>
                </>
              )}
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
