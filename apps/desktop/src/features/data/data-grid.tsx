import { useRef, useState, type KeyboardEvent } from 'react'
import type { PageColumn } from '@aidb/contracts'
import type { ReactTable } from '@tanstack/react-table'
import { useVirtualizer } from '@tanstack/react-virtual'
import {
  ArrowDownIcon,
  ArrowUpIcon,
  ChevronsUpDownIcon,
  PencilIcon,
  Trash2Icon,
} from 'lucide-react'
import { cn } from '@/lib/utils'
import { CellValue } from './cell-value'
import { copyRow } from './copy-actions'
import type { PendingRow } from './edits'
import { columnIds, type GridFeatures, type GridRow } from './grid-table'
import type { ResultSet } from './table-view'

const ROW_HEIGHT = 36
/** Height of the sticky header row, border included. Rows start below it. */
const HEADER_HEIGHT = 49
const ROW_NUMBER_WIDTH = 64
const PAGE_STEP = 10

type GridProps = {
  table: ReactTable<GridFeatures, GridRow>
  page: ResultSet
  /** Keys of cut cells, `row:column`. */
  truncated: Set<string>
  busy: boolean
  /** Opens the row's details, scrolled to `column` when given. */
  onOpenRow: (row: number, column?: string) => void
  /** Unsaved changes to a row, shown in place of its values. */
  pending?: (row: number) => PendingRow | undefined
}

/**
 * The page's rows as a virtualized table: only the rows in view (plus a few) are in the DOM.
 * One row is focusable at a time; arrow keys, Home/End and Page Up/Down move between rows,
 * Enter opens the row's details and Ctrl/Cmd+C copies the row as JSON.
 */
export function DataGrid({ table, page, truncated, busy, onOpenRow, pending }: GridProps) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const rows = table.getRowModel().rows
  const [active, setActive] = useState(0)
  const columnIndex = new Map(columnIds(page.columns).map((id, index) => [id, index]))
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    getItemKey: (index) => rows[index]?.id ?? index,
    overscan: 10,
    // The rows start below the header, which stays on top of them while scrolling.
    scrollMargin: HEADER_HEIGHT,
    scrollPaddingStart: HEADER_HEIGHT,
  })
  const activeRow = Math.min(active, Math.max(rows.length - 1, 0))

  const rowElement = (index: number) =>
    scrollRef.current?.querySelector<HTMLElement>(`tr[data-index="${index}"]`)

  const focusRow = (index: number) => {
    const target = Math.max(0, Math.min(index, rows.length - 1))
    setActive(target)
    virtualizer.scrollToIndex(target, { align: 'auto' })
    const rendered = rowElement(target)
    if (rendered) {
      // Focus at once, so fast repeated keys each move one row.
      rendered.focus({ preventScroll: true })
    } else {
      // Far rows are rendered only after the scroll; focus them on the next frames.
      requestAnimationFrame(() =>
        requestAnimationFrame(() => rowElement(target)?.focus({ preventScroll: true })),
      )
    }
  }

  const onRowKeyDown = (event: KeyboardEvent, index: number) => {
    const moves: Record<string, number> = {
      ArrowDown: index + 1,
      ArrowUp: index - 1,
      PageDown: index + PAGE_STEP,
      PageUp: index - PAGE_STEP,
      Home: 0,
      End: rows.length - 1,
    }
    if (event.key in moves) {
      event.preventDefault()
      focusRow(moves[event.key] ?? index)
    } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'c') {
      event.preventDefault()
      void copyRow(page, index, truncated)
    } else if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      onOpenRow(index)
    }
  }

  const width = table.getTotalSize() + ROW_NUMBER_WIDTH

  return (
    <div
      ref={scrollRef}
      className={cn('relative min-h-0 flex-1 overflow-auto', busy && 'opacity-60')}
      aria-busy={busy}
    >
      <table
        className="grid min-w-full text-sm"
        style={{ width }}
        aria-rowcount={rows.length + 1}
        aria-colcount={page.columns.length + 1}
      >
        <thead className="sticky top-0 z-20 grid bg-background">
          {table.getHeaderGroups().map((group) => (
            <tr
              key={group.id}
              className="flex w-full border-b"
              style={{ height: HEADER_HEIGHT }}
              aria-rowindex={1}
            >
              <th
                scope="col"
                className="sticky left-0 z-10 flex shrink-0 items-center border-r bg-background px-2 text-xs font-medium text-muted-foreground"
                style={{ width: ROW_NUMBER_WIDTH }}
              >
                #
              </th>
              {group.headers.map((header) => {
                const index = columnIndex.get(header.column.id)
                const column = index === undefined ? undefined : page.columns[index]
                return (
                  <th
                    key={header.id}
                    scope="col"
                    className="relative flex shrink-0 border-r text-left"
                    style={{ width: header.getSize() }}
                    aria-sort={ariaSort(header.column.getIsSorted())}
                  >
                    <HeaderLabel
                      column={column}
                      name={column?.name ?? header.column.id}
                      sorted={header.column.getIsSorted()}
                      sortIndex={table.state.sorting.length > 1 ? header.column.getSortIndex() : -1}
                      // v9 returns a handler even for columns that cannot sort.
                      onSort={
                        header.column.getCanSort()
                          ? header.column.getToggleSortingHandler()
                          : undefined
                      }
                    />
                    <div
                      aria-hidden
                      onMouseDown={header.getResizeHandler()}
                      onTouchStart={header.getResizeHandler()}
                      onDoubleClick={() => header.column.resetSize()}
                      data-resizing={header.column.getIsResizing()}
                      className="absolute top-0 right-0 h-full w-1.5 cursor-col-resize touch-none select-none hover:bg-ring/60 data-[resizing=true]:bg-ring"
                    />
                  </th>
                )
              })}
            </tr>
          ))}
        </thead>
        <tbody className="relative grid" style={{ height: virtualizer.getTotalSize() }}>
          {virtualizer.getVirtualItems().map((item) => {
            const row = rows[item.index]
            if (!row) return null
            const change = pending?.(item.index)
            const deleted = change?.status === 'deleted'
            return (
              <tr
                key={row.id}
                data-index={item.index}
                aria-rowindex={item.index + 2}
                tabIndex={item.index === activeRow ? 0 : -1}
                onKeyDown={(event) => onRowKeyDown(event, item.index)}
                onFocus={() => setActive(item.index)}
                className={cn(
                  'absolute left-0 flex w-full border-b outline-none hover:bg-muted/50 focus:bg-muted focus:ring-2 focus:ring-ring focus:ring-inset',
                  deleted && 'text-muted-foreground line-through',
                )}
                style={{
                  height: ROW_HEIGHT,
                  transform: `translateY(${item.start - virtualizer.options.scrollMargin}px)`,
                }}
              >
                <td
                  className="sticky left-0 z-10 flex shrink-0 items-center gap-1 border-r bg-background px-2 text-xs text-muted-foreground tabular-nums no-underline"
                  style={{ width: ROW_NUMBER_WIDTH }}
                >
                  {page.offset + item.index + 1}
                  {change && <PendingMark status={change.status} />}
                </td>
                {row.getVisibleCells().map((cell) => {
                  const index = columnIndex.get(cell.column.id) ?? 0
                  const column = page.columns[index]
                  const edited =
                    change?.status === 'edited' &&
                    column !== undefined &&
                    column.name in change.values
                  return (
                    <td
                      key={cell.id}
                      className="flex shrink-0 cursor-pointer items-center overflow-hidden border-r px-2 whitespace-nowrap"
                      style={{ width: cell.column.getSize() }}
                      onClick={() => {
                        setActive(item.index)
                        onOpenRow(item.index, column?.name)
                      }}
                    >
                      {edited ? (
                        <EditedValue
                          value={change.values[column.name] ?? null}
                          kind={column.kind}
                        />
                      ) : (
                        <CellValue
                          value={row.original[index] ?? null}
                          kind={column?.kind ?? 'other'}
                          truncated={truncated.has(`${item.index}:${index}`)}
                        />
                      )}
                    </td>
                  )
                })}
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}

function ariaSort(sorted: false | 'asc' | 'desc') {
  if (sorted === 'asc') return 'ascending'
  if (sorted === 'desc') return 'descending'
  return undefined
}

function HeaderLabel({
  column,
  name,
  sorted,
  sortIndex,
  onSort,
}: {
  column: PageColumn | undefined
  name: string
  sorted: false | 'asc' | 'desc'
  sortIndex: number
  onSort: ((event: unknown) => void) | undefined
}) {
  const label = (
    <span className="flex min-w-0 flex-col">
      <span className="truncate font-medium">{name}</span>
      {column && (
        <span className="truncate font-mono text-xs font-normal text-muted-foreground">
          {column.dataType}
        </span>
      )}
    </span>
  )
  if (!onSort) {
    return (
      <div className="flex min-w-0 items-center px-2" title={column?.dataType}>
        {label}
      </div>
    )
  }
  const Icon =
    sorted === 'asc' ? ArrowUpIcon : sorted === 'desc' ? ArrowDownIcon : ChevronsUpDownIcon
  return (
    <button
      type="button"
      onClick={onSort}
      title="Sort. Shift-click to sort by more than one column."
      className="flex min-w-0 flex-1 items-center gap-1 px-2 text-left outline-none hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
    >
      {label}
      <Icon
        aria-hidden
        className={cn('ml-auto size-4 shrink-0', !sorted && 'text-muted-foreground')}
      />
      {sortIndex >= 0 && (
        <span className="text-xs text-muted-foreground tabular-nums">{sortIndex + 1}</span>
      )}
    </button>
  )
}

/** Marks a row with unsaved changes; the words are for screen readers, the icon for sight. */
export function PendingMark({ status }: { status: PendingRow['status'] }) {
  const Icon = status === 'deleted' ? Trash2Icon : PencilIcon
  return (
    <span className="ml-auto inline-flex text-foreground" title={PENDING_LABELS[status]}>
      <Icon aria-hidden className="size-3.5" />
      <span className="sr-only">{PENDING_LABELS[status]}</span>
    </span>
  )
}

const PENDING_LABELS: Record<PendingRow['status'], string> = {
  edited: 'Edited, not saved',
  deleted: 'Marked for deletion, not saved',
}

/** An unsaved value: underlined with a dotted line and labelled, so it never relies on color. */
function EditedValue({ value, kind }: { value: string | null; kind: PageColumn['kind'] }) {
  return (
    <span className="flex min-w-0 flex-1 items-center gap-1 underline decoration-dotted underline-offset-4">
      <CellValue value={value} kind={kind} />
      <span className="sr-only"> (edited, not saved)</span>
    </span>
  )
}
