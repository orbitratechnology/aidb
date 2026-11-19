import { useMemo, useState } from 'react'
import type { QueryRun, StatementKind } from '@aidb/contracts'
import { useTable } from '@tanstack/react-table'
import {
  BanIcon,
  ChevronLeftIcon,
  ChevronRightIcon,
  CircleAlertIcon,
  InfoIcon,
  Rows3Icon,
  TextCursorInputIcon,
} from 'lucide-react'
import { EmptyState } from '@/components/shared/status-states'
import { Alert, AlertAction, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { useMediaQuery } from '@/hooks/use-media-query'
import { DataGrid } from '@/features/data/data-grid'
import { EMPTY_ROWS, gridColumns, gridFeatures } from '@/features/data/grid-table'
import { RecordList } from '@/features/data/record-list'
import { RowDetailSheet } from '@/features/data/row-detail-sheet'
import { truncatedKeys } from '@/features/data/table-view'
import { rowSummary } from './paging'
import type { DraftPosition } from './run-store'

const GRID_QUERY = '(min-width: 640px)'

const KIND_LABELS: Record<StatementKind, string> = {
  read: 'Read',
  write: 'Changes data',
  schema: 'Changes structure',
  session: 'Session or transaction command',
  privileged: 'Privileged',
  unknown: 'Not understood',
}

function where(position: DraftPosition | null): string {
  return position ? ` (line ${position.line}, column ${position.column})` : ''
}

/** Moving between pages of a result. Each page runs the statement again. */
export type Pager = {
  page: number
  onPrevious: (() => void) | null
  onNext: (() => void) | null
}

type Props = {
  outcome: QueryRun
  start: DraftPosition
  problemAt: DraftPosition | null
  /** Another page is loading: the rows shown are the page being left. */
  busy: boolean
  pager: Pager
  onShowProblem: () => void
}

function ShowInEditor({ onClick }: { onClick: () => void }) {
  return (
    <AlertAction>
      <Button variant="outline" size="sm" className="max-sm:min-h-11" onClick={onClick}>
        <TextCursorInputIcon data-icon="inline-start" />
        Show in editor
      </Button>
    </AlertAction>
  )
}

/**
 * The outcome of the last run: rows, a refusal, or the database's error. Positions shown are in
 * the draft, not in the text that ran.
 */
export function QueryResultView({ outcome, start, problemAt, busy, pager, onShowProblem }: Props) {
  if (outcome.status === 'refused') {
    return (
      <div className="p-4">
        <Alert>
          <BanIcon />
          <AlertTitle>
            Not run{outcome.kind ? `: ${KIND_LABELS[outcome.kind].toLowerCase()}` : ''}
          </AlertTitle>
          <AlertDescription>
            {outcome.message}
            {where(problemAt)}
          </AlertDescription>
          {problemAt && <ShowInEditor onClick={onShowProblem} />}
        </Alert>
      </div>
    )
  }
  if (outcome.status === 'failed' && outcome.code === 'cancelled') {
    return (
      <div className="p-4">
        <Alert>
          <BanIcon />
          <AlertTitle>Cancelled</AlertTitle>
          <AlertDescription>The statement was stopped before it finished.</AlertDescription>
        </Alert>
      </div>
    )
  }
  if (outcome.status === 'failed') {
    return (
      <div className="p-4">
        <Alert variant="destructive">
          <CircleAlertIcon />
          <AlertTitle>
            {outcome.code === 'timeout' ? 'Timed out' : 'The database returned an error'}
            {outcome.sqlstate ? ` (${outcome.sqlstate})` : ''}
          </AlertTitle>
          <AlertDescription>
            {outcome.message}
            {where(problemAt)}
          </AlertDescription>
          {problemAt && <ShowInEditor onClick={onShowProblem} />}
        </Alert>
      </div>
    )
  }
  return (
    <Rows key={outcome.offset} result={outcome} startLine={start.line} busy={busy} pager={pager} />
  )
}

function Rows({
  result,
  startLine,
  busy,
  pager,
}: {
  result: Extract<QueryRun, { status: 'completed' }>
  startLine: number
  busy: boolean
  pager: Pager
}) {
  const grid = useMediaQuery(GRID_QUERY)
  const [detail, setDetail] = useState<{ row: number; column?: string } | null>(null)
  const columns = useMemo(() => gridColumns(result.columns, { sortable: false }), [result.columns])
  const truncated = useMemo(() => truncatedKeys(result), [result])
  const table = useTable({
    features: gridFeatures,
    columns,
    data: result.rows.length > 0 ? result.rows : EMPTY_ROWS,
    getRowId: (_row, index) => String(index),
    columnResizeMode: 'onChange',
  })

  const summary = `${rowSummary(result)} · ${result.elapsedMs.toLocaleString()} ms`
  const paged = pager.onPrevious !== null || pager.onNext !== null

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 border-b px-4 py-2 text-sm">
        <span className="font-medium tabular-nums" aria-live="polite">
          {summary}
        </span>
        <span className="text-muted-foreground">Statement at line {startLine}</span>
        {result.hasMore && (
          <span className="flex items-center gap-1.5 text-muted-foreground">
            <InfoIcon aria-hidden className="size-4" />
            {result.byteLimitReached
              ? 'More rows matched; this page stopped at the result size limit.'
              : 'More rows matched than one page holds.'}
          </span>
        )}
        {paged && (
          <nav aria-label="Result pages" className="ml-auto flex items-center gap-1">
            <Button
              variant="outline"
              size="sm"
              className="max-sm:min-h-11"
              disabled={busy || !pager.onPrevious}
              onClick={pager.onPrevious ?? undefined}
            >
              <ChevronLeftIcon data-icon="inline-start" />
              Previous
            </Button>
            <span className="px-2 text-muted-foreground tabular-nums">Page {pager.page}</span>
            <Button
              variant="outline"
              size="sm"
              className="max-sm:min-h-11"
              disabled={busy || !pager.onNext}
              onClick={pager.onNext ?? undefined}
            >
              Next
              <ChevronRightIcon data-icon="inline-end" />
            </Button>
          </nav>
        )}
      </div>
      {result.columns.length === 0 || result.rows.length === 0 ? (
        <div className="p-4">
          <EmptyState
            icon={Rows3Icon}
            title="No rows"
            description={
              result.columns.length === 0
                ? 'The statement returned no columns.'
                : 'The statement ran and returned no rows.'
            }
          />
        </div>
      ) : grid ? (
        <DataGrid
          table={table}
          page={result}
          truncated={truncated}
          busy={busy}
          onOpenRow={(row, column) => setDetail({ row, column })}
        />
      ) : (
        <RecordList
          page={result}
          truncated={truncated}
          busy={busy}
          onOpenRow={(row) => setDetail({ row })}
        />
      )}
      <RowDetailSheet
        source="Query result"
        page={result}
        truncated={truncated}
        row={detail?.row ?? null}
        focusColumn={detail?.column}
        onRowChange={(row) => setDetail({ row, column: detail?.column })}
        onClose={() => setDetail(null)}
      />
    </div>
  )
}
