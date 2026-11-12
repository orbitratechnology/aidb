import type { ValueKind } from '@aidb/contracts'
import { cn } from '@/lib/utils'
import { alignsRight } from './table-view'

const MONO_KINDS: ReadonlySet<ValueKind> = new Set(['json', 'array', 'uuid', 'binary'])

/** One value on a single line. NULL and empty text are spelled out, never shown as blank. */
export function CellValue({
  value,
  kind,
  truncated = false,
  className,
}: {
  value: string | null
  kind: ValueKind
  truncated?: boolean
  className?: string
}) {
  if (value === null) {
    return <span className={cn('text-muted-foreground italic', className)}>NULL</span>
  }
  if (value === '') {
    return <span className={cn('text-muted-foreground italic', className)}>empty</span>
  }
  return (
    <span
      className={cn(
        // `relative` keeps the visually hidden note inside the clipped span; otherwise it sits
        // after the full text and widens the whole grid.
        'relative min-w-0 truncate',
        alignsRight(kind) && 'ml-auto tabular-nums',
        MONO_KINDS.has(kind) && 'font-mono text-xs',
        className,
      )}
    >
      {value}
      {truncated && (
        <>
          <span aria-hidden>…</span>
          <span className="sr-only"> (cut short)</span>
        </>
      )}
    </span>
  )
}
