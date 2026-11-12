import { useEffect, useRef } from 'react'
import type { ObjectName, TablePage } from '@aidb/contracts'
import { ChevronDownIcon, ChevronUpIcon } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { cn } from '@/lib/utils'
import { formatForDetail } from './table-view'

const PREFORMATTED = new Set(['json', 'array', 'binary', 'other'])

/** Every field of one row, with complex values laid out in full. */
export function RowDetailSheet({
  object,
  page,
  truncated,
  row,
  focusColumn,
  onRowChange,
  onClose,
}: {
  object: ObjectName
  page: TablePage
  truncated: Set<string>
  row: number | null
  focusColumn: string | undefined
  onRowChange: (row: number) => void
  onClose: () => void
}) {
  const values = row === null ? undefined : page.rows[row]
  const focusRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    // Wait for the sheet to lay out (it slides in) before scrolling to the clicked field.
    const frame = requestAnimationFrame(() =>
      requestAnimationFrame(() => focusRef.current?.scrollIntoView({ block: 'center' })),
    )
    return () => cancelAnimationFrame(frame)
  }, [row, focusColumn])

  return (
    <Sheet open={values !== undefined} onOpenChange={(open) => !open && onClose()}>
      <SheetContent
        side="right"
        className="gap-0 data-[side=right]:w-full data-[side=right]:sm:max-w-lg"
      >
        {row !== null && values && (
          <>
            <SheetHeader className="flex-row items-center gap-2 border-b pr-12">
              <div className="flex min-w-0 flex-1 flex-col gap-1">
                <SheetTitle>Row {page.offset + row + 1}</SheetTitle>
                <SheetDescription className="truncate">
                  {object.schema}.{object.name}
                </SheetDescription>
              </div>
              <Button
                variant="outline"
                size="icon-sm"
                className="max-sm:size-11"
                aria-label="Previous row"
                disabled={row === 0}
                onClick={() => onRowChange(row - 1)}
              >
                <ChevronUpIcon />
              </Button>
              <Button
                variant="outline"
                size="icon-sm"
                className="max-sm:size-11"
                aria-label="Next row"
                disabled={row >= page.rows.length - 1}
                onClick={() => onRowChange(row + 1)}
              >
                <ChevronDownIcon />
              </Button>
            </SheetHeader>
            <dl className="flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto p-2">
              {page.columns.map((column, index) => {
                const value = values[index] ?? null
                const cut = truncated.has(`${row}:${index}`)
                const focused = column.name === focusColumn
                return (
                  <div
                    key={column.name}
                    ref={focused ? focusRef : undefined}
                    className={cn('flex flex-col gap-1 rounded-md p-2', focused && 'bg-muted')}
                  >
                    <dt className="flex flex-wrap items-baseline gap-x-2">
                      <span className="font-medium break-all">{column.name}</span>
                      <span className="font-mono text-xs text-muted-foreground">
                        {column.dataType}
                      </span>
                    </dt>
                    <dd className="min-w-0">
                      {value === null ? (
                        <span className="text-muted-foreground italic">NULL</span>
                      ) : value === '' ? (
                        <span className="text-muted-foreground italic">empty</span>
                      ) : PREFORMATTED.has(column.kind) ? (
                        <pre className="max-h-96 overflow-auto rounded-md border bg-muted/50 p-2 font-mono text-xs break-all whitespace-pre-wrap">
                          {formatForDetail(column.kind, value, cut)}
                        </pre>
                      ) : (
                        <p className="text-sm break-words whitespace-pre-wrap">{value}</p>
                      )}
                      {cut && (
                        <p className="mt-1 text-xs text-muted-foreground">
                          Only the first {page.cellCharLimit.toLocaleString()} characters were
                          loaded.
                        </p>
                      )}
                    </dd>
                  </div>
                )
              })}
            </dl>
          </>
        )}
      </SheetContent>
    </Sheet>
  )
}
