import { useEffect, useId, useRef, useState } from 'react'
import {
  CSV_IMPORT_LIMITS,
  type ConnectionId,
  type CsvImportDraft,
  type CsvImportReady,
  type CsvOptions,
  type DataChangeResult,
  type Environment,
  type ObjectDetails,
  type ObjectName,
} from '@aidb/contracts'
import { useMutation } from '@tanstack/react-query'
import { ChevronDownIcon, CircleAlertIcon, FileTextIcon, TriangleAlertIcon } from 'lucide-react'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
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
import {
  Field,
  FieldDescription,
  FieldGroup,
  FieldLabel,
  FieldLegend,
  FieldSet,
} from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Spinner } from '@/components/ui/spinner'
import {
  applyDataChange,
  cancelDataChange,
  previewCsvImport,
  rereadCsvImport,
} from '@/lib/commands'
import { errorMessage } from '@/lib/errors'
import {
  delimiterLabel,
  encodingLabel,
  importRequest,
  initialMapping,
  INSERT_ONLY,
  keyOptions,
  required,
  rowCount,
  sameOptions,
  unfilled,
  unmappable,
  type Mapping,
} from './csv-import'
import { Assessment, OutcomeAlert, SampleTable, type Outcome } from './review-changes-dialog'
import { returnFocus } from '@/lib/focus-return'

/** The select value for a table column no file column fills. */
const NOT_IMPORTED = 'none'

type ImportProps = {
  connectionId: ConnectionId
  environment: Environment
  object: ObjectName
  details: ObjectDetails
  /** The file as the core read it. */
  draft: CsvImportDraft
  onImported: (result: DataChangeResult, rows: number) => void
  /** Reloads the rows, after an uncertain outcome. */
  onReload: () => void
}

/**
 * Imports a CSV file the core has read: how it is read, which column fills which, and whether
 * rows are only added or also update rows with the same key. The import is checked and planned
 * against the live table, then applied in one transaction: every row, or none.
 */
export function CsvImportDialog({
  open,
  onOpenChange,
  ...props
}: Omit<ImportProps, 'draft'> & {
  draft: CsvImportDraft | null
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { draft } = props
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        {...returnFocus}
        className="flex max-h-[90dvh] flex-col gap-0 p-0 sm:max-w-3xl"
      >
        <DialogHeader className="border-b p-4 sm:p-6">
          <DialogTitle>
            Import into {props.object.schema}.{props.object.name}
          </DialogTitle>
          <DialogDescription>
            The file was read on this computer. Nothing is written until you check the import and
            apply it; it then runs in one transaction, so every row is imported, or none.
          </DialogDescription>
        </DialogHeader>
        {/* Mounted while open, so every file starts afresh. */}
        {open && draft && <Import {...props} draft={draft} onClose={() => onOpenChange(false)} />}
      </DialogContent>
    </Dialog>
  )
}

function Import({
  connectionId,
  environment,
  details,
  draft: opened,
  onImported,
  onReload,
  onClose,
}: ImportProps & { onClose: () => void }) {
  const formId = useId()
  const [draft, setDraft] = useState(opened)
  const [mapping, setMapping] = useState<Mapping>(() => initialMapping(opened, details))
  const [mode, setMode] = useState(INSERT_ONLY)
  const [confirmation, setConfirmation] = useState('')
  const [outcome, setOutcome] = useState<Outcome | null>(null)
  const keys = keyOptions(details)
  const missing = unfilled(mapping, details)

  // The approval this dialog holds, declined whenever the import changes or the dialog closes.
  const held = useRef<string | null>(null)
  const release = () => {
    const approvalId = held.current
    held.current = null
    if (approvalId) cancelDataChange(approvalId).catch(() => undefined)
  }
  // Closed without importing: decline the approval (best effort; it also expires). The file
  // itself is forgotten by the dialog's close, not here, since an effect may run twice.
  useEffect(
    () => () => {
      const approvalId = held.current
      held.current = null
      if (approvalId) cancelDataChange(approvalId).catch(() => undefined)
    },
    [],
  )

  const check = useMutation({
    mutationFn: () =>
      previewCsvImport(connectionId, draft.importId, importRequest(mapping, details, mode)),
    onSuccess: (preview) => {
      held.current = preview.ready?.approvalId ?? null
    },
  })
  const reread = useMutation({
    mutationFn: (options: CsvOptions) => rereadCsvImport(draft.importId, options),
    onSuccess: (next) => {
      // The same columns keep their mapping; other columns start from the suggestions.
      if (JSON.stringify(next.columns) !== JSON.stringify(draft.columns)) {
        setMapping(initialMapping(next, details))
      }
      setDraft(next)
    },
  })
  const apply = useMutation({
    mutationFn: (approvalId: string) =>
      applyDataChange(connectionId, approvalId, confirmation.trim() || null),
  })

  /** Anything changed: what was checked no longer holds. */
  const changed = () => {
    release()
    check.reset()
    apply.reset()
    setOutcome(null)
    setConfirmation('')
  }
  const readWith = (patch: Partial<CsvOptions>) => {
    const options = { ...draft.options, ...patch }
    if (sameOptions(options, draft.options)) return
    changed()
    reread.mutate(options)
  }

  // What a check or an apply found, brought into view below the form.
  const result = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (check.data || check.error || outcome) {
      result.current?.scrollIntoView({ behavior: 'smooth', block: 'start' })
    }
  }, [check.data, check.error, outcome])

  const ready: CsvImportReady | null = outcome ? null : (check.data?.ready ?? null)
  const problems = check.data?.problems ?? []
  const needsName = ready?.confirmation ?? null
  const confirmed = needsName === null || confirmation.trim() === needsName
  const busy = check.isPending || reread.isPending || apply.isPending
  const upsert = mode !== INSERT_ONLY

  const importNow = () => {
    if (!ready) return
    const rows = ready.rows
    apply.mutate(ready.approvalId, {
      onSuccess: (result) => {
        held.current = null
        if (result.status === 'applied') onImported(result, rows)
        else setOutcome({ result })
      },
      onError: (error) => {
        // A wrong name leaves the approval open; anything else spent it.
        if (confirmed) held.current = null
        setOutcome({ error })
      },
    })
  }

  return (
    <>
      <div className="flex min-h-0 flex-1 flex-col gap-6 overflow-y-auto p-4 sm:p-6">
        <section aria-label="File" className="flex flex-col gap-4">
          <div className="flex min-w-0 items-center gap-2 text-sm">
            <FileTextIcon aria-hidden className="size-4 shrink-0 text-muted-foreground" />
            <span className="truncate font-medium">{draft.fileName}</span>
            <span className="shrink-0 text-muted-foreground">
              {rowCount(draft.totalRows)}, {draft.columns.length}{' '}
              {draft.columns.length === 1 ? 'column' : 'columns'}
            </span>
          </div>
          <div className="grid gap-4 sm:grid-cols-[1fr_1fr_auto] sm:items-end">
            <Field>
              <FieldLabel htmlFor={`${formId}-encoding`}>Encoding</FieldLabel>
              <Select
                value={draft.options.encoding}
                disabled={busy}
                onValueChange={(encoding) =>
                  readWith({ encoding: encoding as CsvOptions['encoding'] })
                }
              >
                <SelectTrigger id={`${formId}-encoding`} className="min-h-11 w-full sm:min-h-0">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {CSV_IMPORT_LIMITS.encodings.map((encoding) => (
                    <SelectItem key={encoding} value={encoding}>
                      {encodingLabel(encoding)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </Field>
            <Field>
              <FieldLabel htmlFor={`${formId}-delimiter`}>Separator</FieldLabel>
              <Select
                value={draft.options.delimiter}
                disabled={busy}
                onValueChange={(delimiter) =>
                  readWith({ delimiter: delimiter as CsvOptions['delimiter'] })
                }
              >
                <SelectTrigger id={`${formId}-delimiter`} className="min-h-11 w-full sm:min-h-0">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {CSV_IMPORT_LIMITS.delimiters.map((delimiter) => (
                    <SelectItem key={delimiter} value={delimiter}>
                      {delimiterLabel(delimiter)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </Field>
            <Field orientation="horizontal" className="sm:pb-2">
              <Checkbox
                id={`${formId}-header`}
                checked={draft.options.hasHeader}
                disabled={busy}
                onCheckedChange={(checked) => readWith({ hasHeader: checked === true })}
              />
              <FieldLabel htmlFor={`${formId}-header`}>First row names the columns</FieldLabel>
            </Field>
          </div>
          {reread.isPending && (
            <span className="flex items-center gap-2 text-sm text-muted-foreground" role="status">
              <Spinner />
              Reading the file again…
            </span>
          )}
          {reread.isError && (
            <Alert variant="destructive">
              <CircleAlertIcon />
              <AlertTitle>The file cannot be read that way</AlertTitle>
              <AlertDescription>{errorMessage(reread.error)}</AlertDescription>
            </Alert>
          )}
          {draft.warnings.length > 0 && (
            <Alert>
              <TriangleAlertIcon />
              <AlertTitle>Check how the file was read</AlertTitle>
              <AlertDescription>
                <ul className="list-disc pl-5">
                  {draft.warnings.map((warning) => (
                    <li key={warning}>{warning}</li>
                  ))}
                </ul>
              </AlertDescription>
            </Alert>
          )}
          {draft.sample.length > 0 && (
            <SampleTable
              caption="The file's first rows"
              columns={draft.columns}
              rows={draft.sample}
              more={draft.totalRows > draft.sample.length}
            />
          )}
        </section>

        <FieldSet>
          <FieldLegend>Columns</FieldLegend>
          <FieldDescription>
            Choose the file column that fills each table column. Columns left out get their default;
            an empty value in the file is stored as NULL.
          </FieldDescription>
          <FieldGroup className="gap-3">
            {details.columns.map((column) => {
              const reason = unmappable(column)
              const id = `${formId}-column-${column.position}`
              const value = mapping[column.name]
              return (
                <Field
                  key={column.name}
                  className="grid gap-1.5 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)] sm:items-start sm:gap-x-4"
                >
                  <div className="flex min-w-0 flex-col gap-0.5">
                    <FieldLabel htmlFor={id} className="flex flex-wrap items-center gap-2">
                      <span className="break-all">{column.name}</span>
                      {required(column) && <Badge variant="outline">Required</Badge>}
                    </FieldLabel>
                    <span className="text-xs break-all text-muted-foreground">
                      {column.dataType}
                      {column.default !== null ? `, default ${column.default}` : ''}
                    </span>
                  </div>
                  <div className="flex min-w-0 flex-col gap-1">
                    <Select
                      value={value === undefined ? NOT_IMPORTED : String(value)}
                      disabled={reason !== null || busy}
                      onValueChange={(next) => {
                        changed()
                        setMapping((current) => {
                          const { [column.name]: _old, ...rest } = current
                          return next === NOT_IMPORTED
                            ? rest
                            : { ...rest, [column.name]: Number(next) }
                        })
                      }}
                    >
                      <SelectTrigger id={id} className="min-h-11 w-full sm:min-h-0">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value={NOT_IMPORTED}>Not imported</SelectItem>
                        {draft.columns.map((name, index) => (
                          <SelectItem key={index} value={String(index)}>
                            {name}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    {reason && <FieldDescription>{reason}</FieldDescription>}
                  </div>
                </Field>
              )
            })}
          </FieldGroup>
          {missing.length > 0 && (
            <p className="text-sm text-muted-foreground">
              Every row needs a value for {missing.join(', ')}; choose a file column for{' '}
              {missing.length === 1 ? 'it' : 'them'}.
            </p>
          )}
        </FieldSet>

        <Field>
          <FieldLabel htmlFor={`${formId}-mode`}>Rows already in the table</FieldLabel>
          <Select
            value={mode}
            disabled={busy}
            onValueChange={(next) => {
              changed()
              setMode(next)
            }}
          >
            <SelectTrigger id={`${formId}-mode`} className="min-h-11 w-full sm:min-h-0">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={INSERT_ONLY}>Add rows only</SelectItem>
              {keys.map((key) => (
                <SelectItem key={key.id} value={key.id}>
                  {key.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <FieldDescription>
            {upsert
              ? 'A row with the same key replaces the values of the mapped columns; other rows are added.'
              : 'Every row is added. If one clashes with a row already there, nothing is imported.'}
          </FieldDescription>
        </Field>

        <div ref={result} className="flex scroll-mt-4 flex-col gap-4 empty:hidden">
          {check.isPending && (
            <span className="flex items-center gap-2 text-sm text-muted-foreground" role="status">
              <Spinner />
              Checking every row against the database…
            </span>
          )}
          {check.isError && (
            <Alert variant="destructive">
              <CircleAlertIcon />
              <AlertTitle>The import cannot be checked</AlertTitle>
              <AlertDescription>{errorMessage(check.error)}</AlertDescription>
            </Alert>
          )}
          {problems.length > 0 && (
            <Alert variant="destructive">
              <CircleAlertIcon />
              <AlertTitle>Fix these before importing</AlertTitle>
              <AlertDescription>
                <ul className="list-disc pl-5">
                  {problems.map((problem) => (
                    <li key={problem}>{problem}</li>
                  ))}
                </ul>
                Nothing was written.
              </AlertDescription>
            </Alert>
          )}
          {outcome && (
            <OutcomeAlert
              outcome={outcome}
              rolledBack="The transaction was rolled back, so nothing was imported."
            />
          )}
          {ready && (
            <Ready
              ready={ready}
              environment={environment}
              upsert={upsert}
              needsName={needsName}
              confirmation={confirmation}
              onConfirmation={setConfirmation}
              confirmId={`${formId}-confirm`}
            />
          )}
        </div>
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
        ) : (
          <>
            <Button
              variant="outline"
              className="min-h-11 flex-1 sm:min-h-0 sm:flex-none"
              disabled={apply.isPending}
              onClick={onClose}
            >
              {outcome ? 'Close' : 'Cancel'}
            </Button>
            {ready ? (
              <Button
                className="min-h-11 flex-1 sm:min-h-0 sm:flex-none"
                disabled={!confirmed || apply.isPending}
                onClick={importNow}
              >
                {apply.isPending && <Spinner data-icon="inline-start" />}
                Import {rowCount(ready.rows)}
              </Button>
            ) : (
              <Button
                className="min-h-11 flex-1 sm:min-h-0 sm:flex-none"
                disabled={busy || draft.totalRows === 0}
                onClick={() => {
                  changed()
                  check.mutate()
                }}
              >
                {check.isPending && <Spinner data-icon="inline-start" />}
                {outcome || check.data ? 'Check again' : 'Check the import'}
              </Button>
            )}
          </>
        )}
      </DialogFooter>
    </>
  )
}

/** A checked import, held for approval: what it writes and how it is judged. */
function Ready({
  ready,
  environment,
  upsert,
  needsName,
  confirmation,
  onConfirmation,
  confirmId,
}: {
  ready: CsvImportReady
  environment: Environment
  upsert: boolean
  needsName: string | null
  confirmation: string
  onConfirmation: (value: string) => void
  confirmId: string
}) {
  return (
    <section aria-label="Checked import" className="flex flex-col gap-4">
      <Assessment preview={ready} environment={environment} />
      <p className="text-sm">
        Every row was checked against the table. {rowCount(ready.rows)}{' '}
        {upsert ? 'will be added or updated' : 'will be added'}, in{' '}
        {ready.batches === 1 ? 'one statement' : `${ready.batches} statements`} run in one
        transaction.
      </p>
      {ready.after.rows.length > 0 && (
        <SampleTable
          caption="The first rows, as they will be stored"
          columns={ready.after.columns}
          rows={ready.after.rows}
          more={ready.after.totalRows > ready.after.rows.length}
        />
      )}
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
            {ready.batches === 1 ? 'The statement' : `The first of ${ready.batches} statements`}.
            Values are sent separately as parameters ($1, $2, …), never inside the SQL.
          </p>
          <CodeBlock size="sm">
            <CodeBlockContent wrap className="max-h-40 text-xs">
              {ready.statement}
            </CodeBlockContent>
          </CodeBlock>
        </CollapsibleContent>
      </Collapsible>
      {needsName !== null && (
        <Field>
          <FieldLabel htmlFor={confirmId}>Type {needsName} to confirm</FieldLabel>
          <Input
            id={confirmId}
            value={confirmation}
            autoComplete="off"
            spellCheck={false}
            className="min-h-11 sm:min-h-0"
            onChange={(event) => onConfirmation(event.target.value)}
          />
          <FieldDescription>
            This import is elevated, so it needs the database name typed.
          </FieldDescription>
        </Field>
      )}
    </section>
  )
}
