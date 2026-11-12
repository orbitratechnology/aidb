import { useMemo, useState, type ReactNode } from 'react'
import type {
  ConnectionId,
  Filter,
  ObjectName,
  ObjectSummary,
  PageColumn,
  SortKey,
  TablePage,
} from '@aidb/contracts'
import { TABLE_DATA_LIMITS } from '@aidb/contracts'
import { useQueryClient } from '@tanstack/react-query'
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
  FilterIcon,
  InfoIcon,
  RefreshCwIcon,
  Rows3Icon,
  SearchXIcon,
  XIcon,
} from 'lucide-react'
import { LoadError } from '@/components/shared/load-error'
import { EmptyState } from '@/components/shared/status-states'
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
import { NativeError } from '@/lib/native'
import { approximateCount } from './catalog-view'
import { DataGrid } from './data-grid'
import { FilterSheet } from './filter-sheet'
import {
  EMPTY_ROWS,
  gridColumns,
  gridFeatures,
  type GridFeatures,
  type GridRow,
} from './grid-table'
import { tablePageKey, useTablePage, type PageQuery } from './queries'
import { RecordList } from './record-list'
import { RowDetailSheet } from './row-detail-sheet'
import { describeFilter, rowRange, truncatedKeys } from './table-view'

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
      />

      {page && (
        <RowDetailSheet
          object={object}
          page={page}
          truncated={truncated}
          row={detail?.row ?? null}
          focusColumn={detail?.column}
          onRowChange={(row) => setDetail({ row, column: detail?.column })}
          onClose={() => setDetail(null)}
        />
      )}
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
}: {
  result: ReturnType<typeof useTablePage>
  grid: boolean
  table: ReactTable<GridFeatures, GridRow>
  truncated: Set<string>
  filtered: boolean
  onClearFilters: () => void
  onOpenRow: (row: number, column?: string) => void
  object: ObjectName
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
    <DataGrid table={table} page={page} truncated={truncated} busy={busy} onOpenRow={onOpenRow} />
  ) : (
    <RecordList page={page} truncated={truncated} busy={busy} onOpenRow={onOpenRow} />
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
