import { useEffect, useRef } from 'react'
import {
  ChevronDownIcon,
  ChevronUpIcon,
  CopyIcon,
  InfoIcon,
  PencilIcon,
  Trash2Icon,
  Undo2Icon,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { cn } from '@/lib/utils'
import { copyCell, copyRow } from './copy-actions'
import type { PendingRow } from './edits'
import { formatForDetail, type ResultSet } from './table-view'

const PREFORMATTED = new Set(['json', 'array', 'binary', 'other'])

/** What the sheet offers for changing the shown row. */
export type RowEditActions = {
  /** Why this row cannot be changed, or `null`. */
  blocked: string | null
  pending: PendingRow | undefined
  onEdit: () => void
  onDelete: () => void
  onRevert: () => void
}

/** Every field of one row, with complex values laid out in full. */
export function RowDetailSheet({
  source,
  page,
  truncated,
  row,
  focusColumn,
  onRowChange,
  onClose,
  edit,
}: {
  /** What the row comes from, such as `public.orders`. */
  source: string
  page: ResultSet
  truncated: Set<string>
  row: number | null
  focusColumn: string | undefined
  onRowChange: (row: number) => void
  onClose: () => void
  /** Editing actions, for tables that can be changed here. */
  edit?: RowEditActions
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
                <SheetDescription className="truncate">{source}</SheetDescription>
              </div>
              <Button
                variant="outline"
                size="sm"
                className="max-sm:min-h-11"
                onClick={() => void copyRow(page, row, truncated)}
              >
                <CopyIcon data-icon="inline-start" />
                Copy row
              </Button>
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
            {edit && <EditBar edit={edit} />}
            <dl className="flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto p-2">
              {page.columns.map((column, index) => {
                const pending = edit?.pending
                const edited = pending?.status === 'edited' && column.name in pending.values
                const loaded = values[index] ?? null
                const value = edited ? (pending.values[column.name] ?? null) : loaded
                const cut = !edited && truncated.has(`${row}:${index}`)
                const focused = column.name === focusColumn
                return (
                  <div
                    key={column.name}
                    ref={focused ? focusRef : undefined}
                    className={cn('flex flex-col gap-1 rounded-md p-2', focused && 'bg-muted')}
                  >
                    <dt className="flex flex-wrap items-center gap-x-2">
                      <span className="font-medium break-all">{column.name}</span>
                      <span className="font-mono text-xs text-muted-foreground">
                        {column.dataType}
                      </span>
                      {edited && (
                        <span className="text-xs text-muted-foreground">
                          (edited, not saved; was {loaded === null ? 'NULL' : `“${loaded}”`})
                        </span>
                      )}
                      {value !== null && (
                        <Button
                          variant="ghost"
                          size="icon-xs"
                          className="ml-auto max-sm:size-9"
                          aria-label={`Copy ${column.name}`}
                          title={`Copy ${column.name}`}
                          onClick={() => void copyCell(page, row, index, cut)}
                        >
                          <CopyIcon />
                        </Button>
                      )}
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

function EditBar({ edit }: { edit: RowEditActions }) {
  if (edit.blocked) {
    return (
      <p className="flex items-start gap-2 border-b px-4 py-2 text-sm text-muted-foreground">
        <InfoIcon aria-hidden className="mt-0.5 size-4 shrink-0" />
        {edit.blocked}
      </p>
    )
  }
  const status = edit.pending?.status
  return (
    <div className="flex flex-wrap items-center gap-2 border-b px-4 py-2">
      {status === 'deleted' ? (
        <p className="flex-1 text-sm">Marked for deletion. It is deleted when you save.</p>
      ) : (
        <>
          <Button variant="outline" size="sm" className="max-sm:min-h-11" onClick={edit.onEdit}>
            <PencilIcon data-icon="inline-start" />
            Edit
          </Button>
          <Button variant="outline" size="sm" className="max-sm:min-h-11" onClick={edit.onDelete}>
            <Trash2Icon data-icon="inline-start" />
            Delete
          </Button>
        </>
      )}
      {status && (
        <Button variant="ghost" size="sm" className="max-sm:min-h-11" onClick={edit.onRevert}>
          <Undo2Icon data-icon="inline-start" />
          {status === 'deleted' ? 'Keep this row' : 'Undo edits'}
        </Button>
      )}
    </div>
  )
}
