import { useRef } from 'react'
import type { TablePage } from '@aidb/contracts'
import { useVirtualizer } from '@tanstack/react-virtual'
import { cn } from '@/lib/utils'
import { CellValue } from './cell-value'

/** Fields shown on each card; the row details show all of them. */
const PREVIEW_FIELDS = 4

/**
 * Compact layout: each row as a card with its first few fields. Cards are virtualized like the
 * grid's rows, and open the row's details.
 */
export function RecordList({
  page,
  truncated,
  busy,
  onOpenRow,
}: {
  page: TablePage
  truncated: Set<string>
  busy: boolean
  onOpenRow: (row: number) => void
}) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const virtualizer = useVirtualizer({
    count: page.rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 140,
    overscan: 4,
  })
  const fields = page.columns.slice(0, PREVIEW_FIELDS)

  return (
    <div
      ref={scrollRef}
      className={cn('min-h-0 flex-1 overflow-y-auto px-4', busy && 'opacity-60')}
      aria-busy={busy}
    >
      <ul
        aria-label="Rows"
        className="relative w-full"
        style={{ height: virtualizer.getTotalSize() }}
      >
        {virtualizer.getVirtualItems().map((item) => {
          const row = page.rows[item.index] ?? []
          return (
            <li
              key={item.key}
              data-index={item.index}
              ref={virtualizer.measureElement}
              className="absolute left-0 w-full py-1.5"
              style={{ transform: `translateY(${item.start}px)` }}
            >
              <button
                type="button"
                onClick={() => onOpenRow(item.index)}
                className="flex w-full flex-col gap-1 rounded-md border p-3 text-left text-sm outline-none hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring"
              >
                <span className="text-xs text-muted-foreground tabular-nums">
                  Row {page.offset + item.index + 1}
                </span>
                {fields.map((column, index) => (
                  <span key={column.name} className="flex min-w-0 items-baseline gap-2">
                    <span className="max-w-[40%] shrink-0 truncate text-muted-foreground">
                      {column.name}
                    </span>
                    <CellValue
                      value={row[index] ?? null}
                      kind={column.kind}
                      truncated={truncated.has(`${item.index}:${index}`)}
                      className="ml-0"
                    />
                  </span>
                ))}
                {page.columns.length > PREVIEW_FIELDS && (
                  <span className="text-xs text-muted-foreground">
                    {page.columns.length - PREVIEW_FIELDS} more fields
                  </span>
                )}
              </button>
            </li>
          )
        })}
      </ul>
    </div>
  )
}
