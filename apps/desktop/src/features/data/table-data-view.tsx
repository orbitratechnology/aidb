import { useMemo, useState, type ReactNode } from 'react'
import type {
  ConnectionId,
  CsvImportDraft,
  Filter,
  ObjectName,
  ObjectSummary,
  PageColumn,
  SortKey,
  TablePage,
} from '@aidb/contracts'
import { TABLE_DATA_LIMITS } from '@aidb/contracts'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import {
  useTable,
  type ColumnVisibilityState,
  type ReactTable,
  type SortingState,
} from '@tanstack/react-table'
import {
  ChevronLeftIcon,
  ChevronRightIcon,
  ChevronsLeftIcon,
  Columns3Icon,
  EyeOffIcon,
  FileUpIcon,
  FilterIcon,
  InfoIcon,
  PencilLineIcon,
  PlusIcon,
  RefreshCwIcon,
  Rows3Icon,
  SearchXIcon,
  XIcon,
} from 'lucide-react'
import { toast } from 'sonner'
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
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { useMediaQuery } from '@/hooks/use-media-query'
import { useActiveConnection } from '@/features/connections/sessions'
import { closeCsvImport, openCsvImport } from '@/lib/commands'
import { NativeError } from '@/lib/native'
import { approximateCount } from './catalog-view'
import { CsvImportDialog } from './csv-import-dialog'
import { DataGrid } from './data-grid'
import { editScope, useEditBuffer } from './edit-store'
import {
  bufferProblem,
  deleteRow,
  describeCounts,
  describeKey,
  editing,
  editRow,
  keyId,
  pendingCounts,
  pendingRow,
  putNewRow,
  removeNewRow,
  revertRow,
  reviewItems,
  rowKey,
  rowValues,
  toChanges,
  type Editing,
  type EditBuffer,
  type NewRow,
  type PendingRow,
  type ReviewItem,
} from './edits'
import { ExportMenu } from './export-menu'
import { FilterSheet } from './filter-sheet'
import {
  EMPTY_ROWS,
  gridColumns,
  gridFeatures,
  type GridFeatures,
  type GridRow,
} from './grid-table'
import { tablePageKey, useObjectDetails, useTablePage, type PageQuery } from './queries'
import { RecordList } from './record-list'
import { ReviewChangesDialog } from './review-changes-dialog'
import { RowDetailSheet, type RowEditActions } from './row-detail-sheet'
import { RowEditorSheet, type EditorColumn, type EditorTarget } from './row-editor-sheet'
import { describeFilter, rowRange, truncatedKeys } from './table-view'
import { returnFocus } from '@/lib/focus-return'

/** Below this width rows are shown as a record list instead of a grid. */
const GRID_QUERY = '(min-width: 640px)'

type Detail = { row: number; column?: string }

/** Rows of one table or view, a bounded page at a time, filtered and sorted by the database. */
export function TableDataView({
  connectionId,
  object,
  summary,
}: {
  connectionId: ConnectionId
  object: ObjectName
  summary: ObjectSummary | undefined
}) {
  const queryClient = useQueryClient()
  const grid = useMediaQuery(GRID_QUERY)
  const [sorting, setSorting] = useState<SortingState>([])
  const [filters, setFilters] = useState<Filter[]>([])
  // Offsets of the pages visited, so Previous returns exactly where it came from even when a
  // page stopped early at the size limit.
  const [offsets, setOffsets] = useState<number[]>([0])
  const [columnVisibility, setColumnVisibility] = useState<ColumnVisibilityState>({})
  const [detail, setDetail] = useState<Detail | null>(null)
  const [filtersOpen, setFiltersOpen] = useState(false)

  const offset = offsets.at(-1) ?? 0
  const sort: SortKey[] = useMemo(
    () => sorting.map(({ id, desc }) => ({ column: id, direction: desc ? 'desc' : 'asc' })),
    [sorting],
  )
  const query: PageQuery = useMemo(() => ({ filters, sort, offset }), [filters, sort, offset])
  const result = useTablePage(connectionId, object, query)
  const page = result.data

  // Rebuilt only when the columns change, not for every page of the same table.
  // The last columns seen, kept while a page fails (say, a bad filter value) so the filters stay
  // editable. Grid columns are rebuilt only when they change, not for every page.
  const [columnsKey, setColumnsKey] = useState('[]')
  const pageColumnsKey = page ? JSON.stringify(page.columns) : columnsKey
  if (pageColumnsKey !== columnsKey) setColumnsKey(pageColumnsKey)
  const knownColumns = useMemo(() => JSON.parse(columnsKey) as PageColumn[], [columnsKey])
  const columns = useMemo(() => gridColumns(knownColumns), [knownColumns])
  const table = useTable({
    features: gridFeatures,
    columns,
    data: page?.rows ?? EMPTY_ROWS,
    getRowId: (_row, index) => String(index),
    manualSorting: true,
    enableMultiSort: true,
    maxMultiSortColCount: TABLE_DATA_LIMITS.maxSortKeys,
    columnResizeMode: 'onChange',
    state: { sorting, columnVisibility },
    onSortingChange: (updater) => {
      setSorting(updater)
      setOffsets([0])
    },
    onColumnVisibilityChange: setColumnVisibility,
  })

  const applyFilters = (next: { filters: Filter[]; sort: SortKey[] }) => {
    setFilters(next.filters)
    setSorting(
      next.sort.map(({ column, direction }) => ({ id: column, desc: direction === 'desc' })),
    )
    setOffsets([0])
  }
  const removeFilter = (index: number) => {
    setFilters((current) => current.filter((_, i) => i !== index))
    setOffsets([0])
  }
  const cancel = () =>
    void queryClient.cancelQueries({ queryKey: tablePageKey(connectionId, object, query) })

  const fetching = result.fetchStatus === 'fetching'
  const stopped = result.fetchStatus === 'idle' && (result.isPending || result.isPlaceholderData)
  const truncated = useMemo(() => (page ? truncatedKeys(page) : new Set<string>()), [page])

  // Editing: what may change here, the unsaved changes, and the rows they touch on this page.
  const active = useActiveConnection()
  const details = useObjectDetails(connectionId, object)
  const scope = editScope(connectionId, object)
  const { buffer, update, clear } = useEditBuffer(scope)
  const readOnly = active?.readOnly ?? true
  const edit: Editing | null = useMemo(
    () =>
      details.data ? editing({ details: details.data, columns: knownColumns, readOnly }) : null,
    [details.data, knownColumns, readOnly],
  )
  const rowKeys = useMemo(
    () =>
      page && edit && !edit.rowsBlocked
        ? page.rows.map((_, row) => rowKey(page, row, edit.keyColumns, truncated))
        : [],
    [page, edit, truncated],
  )
  const pending = (row: number): PendingRow | undefined => pendingRow(buffer, rowKeys[row] ?? null)
  const [editor, setEditor] = useState<EditorTarget | null>(null)
  const [editorRow, setEditorRow] = useState<number | null>(null)
  const [reviewing, setReviewing] = useState<{
    changes: ReturnType<typeof toChanges>
    items: ReviewItem[]
  } | null>(null)
  const [discarding, setDiscarding] = useState(false)
  const [importing, setImporting] = useState<CsvImportDraft | null>(null)
  // The core holds the file until the dialog closes (or the connection changes).
  const closeImport = () => {
    if (importing) closeCsvImport(importing.importId).catch(() => undefined)
    setImporting(null)
  }
  const chooseFile = useMutation({
    mutationFn: () => openCsvImport(connectionId, object),
    onSuccess: (draft) => draft && setImporting(draft),
    onError: (error) => toast.error(error instanceof Error ? error.message : String(error)),
  })
  const counts = pendingCounts(buffer)

  const editorColumns = (row: number | null): EditorColumn[] =>
    knownColumns.map((column, index) => {
      const info = details.data?.columns.find((c) => c.name === column.name)
      const cut = row !== null && truncated.has(`${row}:${index}`)
      return {
        column,
        nullable: info?.nullable ?? true,
        readOnly:
          edit?.readOnlyColumns[column.name] ??
          (cut
            ? `Only the first ${page?.cellCharLimit.toLocaleString() ?? ''} characters were loaded, so it cannot be edited here.`
            : null),
      }
    })

  const rowEdit = (row: number): RowEditActions | undefined => {
    if (!page || !edit) return undefined
    const key = rowKeys[row] ?? null
    const blocked =
      edit.blocked ??
      edit.rowsBlocked ??
      (key ? null : "This row's primary key was not loaded in full, so it cannot be changed here.")
    const loaded = rowValues(page, row, truncated)
    const rowPending = pending(row)
    return {
      blocked,
      pending: rowPending,
      onEdit: () => {
        if (!key) return
        setEditorRow(row)
        setEditor({
          mode: 'edit',
          label: describeKey(key),
          values: { ...loaded, ...(rowPending?.status === 'edited' ? rowPending.values : {}) },
        })
      },
      onDelete: () => key && update((current) => deleteRow(current, key, loaded)),
      onRevert: () => key && update((current) => revertRow(current, keyId(key))),
    }
  }

  const keepEdits = (values: Record<string, string | null>) => {
    if (!editor || !page) return
    if (editor.mode === 'new') {
      const row: NewRow = { id: editor.id, values }
      update((current) => putNewRow(current, row))
    } else if (editorRow !== null) {
      const key = rowKeys[editorRow]
      if (key)
        update((current) => editRow(current, key, rowValues(page, editorRow, truncated), values))
    }
    setEditor(null)
    setEditorRow(null)
  }

  const startReview = () =>
    setReviewing({
      changes: toChanges(buffer, object, knownColumns),
      items: reviewItems(buffer, knownColumns),
    })

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-wrap items-center gap-2 border-b px-4 py-2 sm:px-6">
        <Button
          variant="outline"
          size="sm"
          className="max-sm:min-h-11"
          disabled={knownColumns.length === 0}
          onClick={() => setFiltersOpen(true)}
        >
          <FilterIcon data-icon="inline-start" />
          Filter and sort
          {filters.length > 0 && (
            <Badge variant="secondary" className="ml-1 tabular-nums">
              {filters.length}
              <span className="sr-only">{filters.length === 1 ? ' filter' : ' filters'}</span>
            </Badge>
          )}
        </Button>
        {grid && page && <ColumnsMenu table={table} />}
        <ExportMenu connectionId={connectionId} object={object} filters={filters} sort={sort} />
        <Button
          variant="ghost"
          size="sm"
          className="max-sm:min-h-11"
          disabled={fetching}
          onClick={() => void result.refetch()}
        >
          <RefreshCwIcon data-icon="inline-start" />
          Reload
        </Button>
        {edit && !edit.blocked && (
          <Button
            variant="outline"
            size="sm"
            className="max-sm:min-h-11"
            onClick={() => {
              setEditorRow(null)
              setEditor({ mode: 'new', id: crypto.randomUUID(), label: 'New row', values: {} })
            }}
          >
            <PlusIcon data-icon="inline-start" />
            Add row
          </Button>
        )}
        {edit && !edit.blocked && (
          <Button
            variant="outline"
            size="sm"
            className="max-sm:min-h-11"
            disabled={chooseFile.isPending}
            onClick={() => chooseFile.mutate()}
          >
            {chooseFile.isPending ? (
              <Spinner data-icon="inline-start" />
            ) : (
              <FileUpIcon data-icon="inline-start" />
            )}
            Import CSV
          </Button>
        )}
        {fetching && (
          <span className="flex items-center gap-2 text-sm text-muted-foreground" role="status">
            <Spinner />
            Loading rows…
            <Button variant="link" size="sm" className="h-auto px-0" onClick={cancel}>
              Cancel
            </Button>
          </span>
        )}
        {stopped && !fetching && (
          <span className="flex items-center gap-2 text-sm text-muted-foreground" role="status">
            Loading stopped.
            <Button
              variant="link"
              size="sm"
              className="h-auto px-0"
              onClick={() => void result.refetch()}
            >
              Load again
            </Button>
          </span>
        )}
        {page && (
          <Pager
            page={page}
            estimate={filters.length === 0 ? (summary?.rowEstimate ?? null) : null}
            atStart={offsets.length === 1}
            disabled={fetching}
            onFirst={() => setOffsets([0])}
            onPrevious={() => setOffsets((current) => current.slice(0, -1))}
            onNext={() => setOffsets((current) => [...current, offset + page.rows.length])}
          />
        )}
      </div>

      {filters.length > 0 && (
        <ul aria-label="Active filters" className="flex flex-wrap gap-2 border-b px-4 py-2 sm:px-6">
          {filters.map((filter, index) => (
            <li key={`${index}-${describeFilter(filter)}`}>
              <Badge variant="outline" className="gap-1 py-0.5 pr-0.5 font-normal">
                <span className="max-w-64 truncate">{describeFilter(filter)}</span>
                <Button
                  variant="ghost"
                  size="icon-xs"
                  className="max-sm:size-8"
                  aria-label={`Remove filter: ${describeFilter(filter)}`}
                  onClick={() => removeFilter(index)}
                >
                  <XIcon />
                </Button>
              </Badge>
            </li>
          ))}
        </ul>
      )}

      {counts.total > 0 && (
        <PendingBar
          buffer={buffer}
          problem={bufferProblem(buffer)}
          onEditNew={(row) => {
            setEditorRow(null)
            setEditor({ mode: 'new', id: row.id, label: 'New row', values: row.values })
          }}
          onRemoveNew={(id) => update((current) => removeNewRow(current, id))}
          onReview={startReview}
          onDiscard={() => setDiscarding(true)}
        />
      )}

      {page && <PageNotices page={page} isView={summary?.kind === 'view'} />}

      <Body
        result={result}
        grid={grid}
        table={table}
        truncated={truncated}
        filtered={filters.length > 0}
        onClearFilters={() => applyFilters({ filters: [], sort })}
        onOpenRow={(row, column) => setDetail({ row, column })}
        object={object}
        pending={pending}
      />

      {page && (
        <RowDetailSheet
          source={`${object.schema}.${object.name}`}
          page={page}
          truncated={truncated}
          row={detail?.row ?? null}
          focusColumn={detail?.column}
          onRowChange={(row) => setDetail({ row, column: detail?.column })}
          onClose={() => setDetail(null)}
          edit={detail ? rowEdit(detail.row) : undefined}
        />
      )}
      <RowEditorSheet
        target={editor}
        source={`${object.schema}.${object.name}`}
        columns={editorColumns(editorRow)}
        onKeep={keepEdits}
        onClose={() => {
          setEditor(null)
          setEditorRow(null)
        }}
      />
      {active && (
        <ReviewChangesDialog
          open={reviewing !== null}
          onOpenChange={(open) => !open && setReviewing(null)}
          connectionId={connectionId}
          environment={active.environment}
          changes={reviewing?.changes ?? []}
          items={reviewing?.items ?? []}
          onApplied={(outcome) => {
            clear()
            setReviewing(null)
            const rows = outcome.rowsAffected ?? 0
            toast.success(`Saved. ${rows === 1 ? '1 row' : `${rows} rows`} written.`)
            void queryClient.invalidateQueries({
              queryKey: ['tablePage', connectionId, object.schema, object.name],
            })
          }}
          onReload={() => void result.refetch()}
        />
      )}
      {active && details.data && (
        <CsvImportDialog
          open={importing !== null}
          onOpenChange={(open) => !open && closeImport()}
          connectionId={connectionId}
          environment={active.environment}
          object={object}
          details={details.data}
          draft={importing}
          onImported={(_result, rows) => {
            closeImport()
            toast.success(`Imported ${rows === 1 ? '1 row' : `${rows.toLocaleString()} rows`}.`)
            void queryClient.invalidateQueries({ queryKey: ['audit'] })
            void queryClient.invalidateQueries({
              queryKey: ['tablePage', connectionId, object.schema, object.name],
            })
          }}
          onReload={() => void result.refetch()}
        />
      )}
      <AlertDialog open={discarding} onOpenChange={setDiscarding}>
        <AlertDialogContent {...returnFocus}>
          <AlertDialogHeader>
            <AlertDialogTitle>
              Discard{' '}
              {counts.total === 1 ? 'the unsaved change' : `${counts.total} unsaved changes`}?
            </AlertDialogTitle>
            <AlertDialogDescription>
              {describeCounts(counts)}. {counts.total === 1 ? 'It was' : 'They were'} never saved,
              so the database stays as it is.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep editing</AlertDialogCancel>
            <AlertDialogAction variant="destructive" onClick={clear}>
              Discard
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <FilterSheet
        open={filtersOpen}
        onOpenChange={setFiltersOpen}
        columns={knownColumns}
        filters={filters}
        sort={sort}
        onApply={applyFilters}
      />
    </div>
  )
}

function Body({
  result,
  grid,
  table,
  truncated,
  filtered,
  onClearFilters,
  onOpenRow,
  object,
  pending,
}: {
  result: ReturnType<typeof useTablePage>
  grid: boolean
  table: ReactTable<GridFeatures, GridRow>
  truncated: Set<string>
  filtered: boolean
  onClearFilters: () => void
  onOpenRow: (row: number, column?: string) => void
  object: ObjectName
  pending: (row: number) => PendingRow | undefined
}) {
  if (result.isError) {
    const error = result.error
    return (
      <div className="p-4 sm:p-6">
        {error instanceof NativeError && error.code === 'not_found' ? (
          <EmptyState
            icon={SearchXIcon}
            title={`${object.schema}.${object.name} was not found`}
            description="It may have been dropped or renamed. Refresh the schema to see the current list."
          />
        ) : (
          <LoadError
            title={
              error instanceof NativeError && error.code === 'cancelled'
                ? 'Loading was cancelled'
                : 'Could not load rows'
            }
            error={error}
            retrying={result.isFetching}
            onRetry={() => void result.refetch()}
          />
        )}
      </div>
    )
  }
  const page = result.data
  if (!page) {
    return result.fetchStatus === 'fetching' ? (
      <div className="flex flex-col gap-2 p-4 sm:p-6" aria-busy="true" aria-label="Loading rows">
        {Array.from({ length: 8 }, (_, index) => (
          <Skeleton key={index} className="h-8 w-full" />
        ))}
      </div>
    ) : null
  }
  if (page.rows.length === 0) {
    return (
      <div className="p-4 sm:p-6">
        <EmptyState
          icon={Rows3Icon}
          title={filtered ? 'No rows match these filters' : 'No rows'}
          description={
            filtered
              ? 'Change or clear the filters to see more.'
              : page.offset > 0
                ? 'There are no more rows after this point.'
                : 'This table or view is empty.'
          }
        >
          {filtered && (
            <Button variant="outline" className="min-h-11 sm:min-h-0" onClick={onClearFilters}>
              Clear filters
            </Button>
          )}
        </EmptyState>
      </div>
    )
  }
  const busy = result.isPlaceholderData
  return grid ? (
    <DataGrid
      table={table}
      page={page}
      truncated={truncated}
      busy={busy}
      onOpenRow={onOpenRow}
      pending={pending}
    />
  ) : (
    <RecordList
      page={page}
      truncated={truncated}
      busy={busy}
      onOpenRow={onOpenRow}
      pending={pending}
    />
  )
}

function Pager({
  page,
  estimate,
  atStart,
  disabled,
  onFirst,
  onPrevious,
  onNext,
}: {
  page: TablePage
  estimate: number | null
  atStart: boolean
  disabled: boolean
  onFirst: () => void
  onPrevious: () => void
  onNext: () => void
}) {
  const approx = approximateCount(estimate)
  return (
    <div className="ml-auto flex items-center gap-1">
      <span className="mr-1 text-sm text-muted-foreground tabular-nums" aria-live="polite">
        {rowRange(page.offset, page.rows.length)}
        {approx && page.rows.length > 0 && ` of ${approx}`}
      </span>
      <Button
        variant="outline"
        size="icon-sm"
        className="max-sm:size-11"
        aria-label="First page"
        disabled={disabled || atStart}
        onClick={onFirst}
      >
        <ChevronsLeftIcon />
      </Button>
      <Button
        variant="outline"
        size="icon-sm"
        className="max-sm:size-11"
        aria-label="Previous page"
        disabled={disabled || atStart}
        onClick={onPrevious}
      >
        <ChevronLeftIcon />
      </Button>
      <Button
        variant="outline"
        size="icon-sm"
        className="max-sm:size-11"
        aria-label="Next page"
        disabled={disabled || !page.hasMore}
        onClick={onNext}
      >
        <ChevronRightIcon />
      </Button>
    </div>
  )
}

function ColumnsMenu({ table }: { table: ReactTable<GridFeatures, GridRow> }) {
  const all = table.getAllLeafColumns()
  const hidden = all.filter((column) => !column.getIsVisible()).length
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm">
          <Columns3Icon data-icon="inline-start" />
          Columns
          {hidden > 0 && (
            <Badge variant="secondary" className="ml-1 tabular-nums">
              {hidden} hidden
            </Badge>
          )}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="max-h-96 w-64 overflow-y-auto">
        <DropdownMenuLabel>Shown columns</DropdownMenuLabel>
        {all.map((column) => (
          <DropdownMenuCheckboxItem
            key={column.id}
            checked={column.getIsVisible()}
            onCheckedChange={(value) => column.toggleVisibility(value)}
            onSelect={(event) => event.preventDefault()}
          >
            <span className="truncate">{column.id}</span>
          </DropdownMenuCheckboxItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuItem disabled={hidden === 0} onSelect={() => table.resetColumnVisibility()}>
          Show all
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => table.resetColumnSizing()}>Reset widths</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/** What the user should know about this page: early stops, unstable order, hidden columns. */
function PageNotices({ page, isView }: { page: TablePage; isView: boolean }) {
  const notices: Array<{ key: string; icon: ReactNode; text: string }> = []
  if (page.byteLimitReached) {
    notices.push({
      key: 'bytes',
      icon: <InfoIcon />,
      text: 'This page stopped early at the result size limit (Settings › Safety). Use Next for the rest.',
    })
  }
  if (!page.stableOrder && page.rows.length > 0) {
    notices.push({
      key: 'order',
      icon: <InfoIcon />,
      text: `This ${isView ? 'view' : 'table'} has no primary key, so rows can repeat or be skipped between pages. Sort by a unique column for a stable order.`,
    })
  }
  if (page.omittedColumns.length > 0) {
    notices.push({
      key: 'omitted',
      icon: <EyeOffIcon />,
      text: `Not shown, because your database role cannot read them: ${page.omittedColumns.join(', ')}.`,
    })
  }
  if (notices.length === 0) return null
  return (
    <ul className="flex flex-col gap-1 border-b px-4 py-2 text-sm text-muted-foreground sm:px-6">
      {notices.map((notice) => (
        <li
          key={notice.key}
          className="flex items-start gap-2 [&_svg]:mt-0.5 [&_svg]:size-4 [&_svg]:shrink-0"
        >
          {notice.icon}
          <span>{notice.text}</span>
        </li>
      ))}
    </ul>
  )
}

/** Unsaved changes to this table, with the way to review and save or discard them. */
function PendingBar({
  buffer,
  problem,
  onEditNew,
  onRemoveNew,
  onReview,
  onDiscard,
}: {
  buffer: EditBuffer
  problem: string | null
  onEditNew: (row: NewRow) => void
  onRemoveNew: (id: string) => void
  onReview: () => void
  onDiscard: () => void
}) {
  const counts = pendingCounts(buffer)
  return (
    <section
      aria-label="Unsaved changes"
      className="flex flex-col gap-2 border-b bg-muted/50 px-4 py-2 sm:px-6"
    >
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
        <p className="flex min-w-0 flex-1 basis-64 items-start gap-2 text-sm">
          <PencilLineIcon aria-hidden className="mt-0.5 size-4 shrink-0" />
          <span>
            <span className="font-medium">
              {counts.total === 1 ? '1 unsaved change' : `${counts.total} unsaved changes`}
            </span>
            <span className="text-muted-foreground">
              : {describeCounts(counts)}. Nothing is written until you save.
            </span>
          </span>
        </p>
        <div className="ml-auto flex gap-2">
          <Button variant="outline" size="sm" className="max-sm:min-h-11" onClick={onDiscard}>
            Discard
          </Button>
          <Button
            size="sm"
            className="max-sm:min-h-11"
            disabled={problem !== null}
            onClick={onReview}
          >
            Review and save
          </Button>
        </div>
      </div>
      {problem && <p className="text-sm text-destructive">{problem}</p>}
      {buffer.inserts.length > 0 && (
        <ul aria-label="New rows" className="flex flex-wrap gap-2">
          {buffer.inserts.map((row, index) => (
            <li key={row.id}>
              <Badge variant="outline" className="gap-1 py-0.5 pr-0.5 font-normal">
                <PlusIcon aria-hidden />
                <Button
                  variant="link"
                  size="sm"
                  className="h-auto px-0"
                  onClick={() => onEditNew(row)}
                >
                  New row {buffer.inserts.length > 1 ? index + 1 : ''}
                  <span className="sr-only">, edit</span>
                </Button>
                <Button
                  variant="ghost"
                  size="icon-xs"
                  className="max-sm:size-8"
                  aria-label={`Remove new row ${index + 1}`}
                  onClick={() => onRemoveNew(row.id)}
                >
                  <XIcon />
                </Button>
              </Badge>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}
