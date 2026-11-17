import { useMemo, useState } from 'react'
import type { QueryRun, StatementKind } from '@aidb/contracts'
import { useTable } from '@tanstack/react-table'
import { BanIcon, CircleAlertIcon, InfoIcon, Rows3Icon } from 'lucide-react'
import { EmptyState } from '@/components/shared/status-states'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { useMediaQuery } from '@/hooks/use-media-query'
import { pluralize } from '@/features/connections/format'
import { DataGrid } from '@/features/data/data-grid'
import { EMPTY_ROWS, gridColumns, gridFeatures } from '@/features/data/grid-table'
import { RecordList } from '@/features/data/record-list'
import { RowDetailSheet } from '@/features/data/row-detail-sheet'
import { truncatedKeys } from '@/features/data/table-view'
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

/**
 * The outcome of the last run: rows, a refusal, or the database's error. Positions shown are in
 * the draft, not in the text that ran.
 */
export function QueryResultView({
  outcome,
  start,
  problemAt,
}: {
  outcome: QueryRun
  start: DraftPosition
  problemAt: DraftPosition | null
}) {
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
        </Alert>
      </div>
    )
  }
  return <Rows result={outcome} startLine={start.line} />
}

function Rows({
  result,
  startLine,
}: {
  result: Extract<QueryRun, { status: 'completed' }>
  startLine: number
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

  const summary = [
    result.hasMore
      ? `First ${pluralize(result.rows.length, 'row', 'rows')}`
      : pluralize(result.rows.length, 'row', 'rows'),
    `${result.elapsedMs.toLocaleString()} ms`,
  ].join(' · ')

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 border-b px-4 py-2 text-sm">
        <span className="font-medium tabular-nums" aria-live="polite">
          {summary}
        </span>
        <span className="text-muted-foreground">Statement at line {startLine}</span>
        {(result.hasMore || result.byteLimitReached) && (
          <span className="flex items-center gap-1.5 text-muted-foreground">
            <InfoIcon aria-hidden className="size-4" />
            {result.byteLimitReached
              ? 'More rows matched; this page stopped at the result size limit.'
              : 'More rows matched than one page holds.'}
          </span>
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
          busy={false}
          onOpenRow={(row, column) => setDetail({ row, column })}
        />
      ) : (
        <RecordList
          page={result}
          truncated={truncated}
          busy={false}
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
