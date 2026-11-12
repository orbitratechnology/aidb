import type { ConnectionId, ObjectName, ObjectSummary } from '@aidb/contracts'
import { LockIcon } from 'lucide-react'
import { useSearchParams } from 'react-router'
import { PageContainer } from '@/components/shared/page'
import { Badge } from '@/components/ui/badge'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { formatRowEstimate, KIND_LABELS } from './catalog-view'
import { KIND_ICONS } from './kind-display'
import { ObjectDetailsView } from './object-details'
import { TableDataView } from './table-data-view'

type View = 'data' | 'definition'

/** The tab lives in the URL (`?view=definition`), so back and forward restore it. */
function useView(): [View, (view: View) => void] {
  const [params, setParams] = useSearchParams()
  const view: View = params.get('view') === 'definition' ? 'definition' : 'data'
  const setView = (next: View) =>
    setParams(next === 'data' ? {} : { view: next }, { replace: true })
  return [view, setView]
}

/** A table or view: its rows and its definition, under one header. */
export function ObjectView({
  connectionId,
  object,
  summary,
}: {
  connectionId: ConnectionId
  object: ObjectName
  /** From the catalog; missing if the object is no longer listed. */
  summary: ObjectSummary | undefined
}) {
  const [view, setView] = useView()

  return (
    <Tabs
      value={view}
      onValueChange={(next) => setView(next as View)}
      className="min-h-0 flex-1 gap-0"
    >
      <header className="flex flex-col gap-2 border-b px-4 pt-3 pb-3 sm:px-6">
        <div className="flex flex-wrap items-end justify-between gap-x-4 gap-y-2">
          <div className="min-w-0">
            <p className="text-sm text-muted-foreground">{object.schema}</p>
            <h2 className="text-xl font-semibold break-all">{object.name}</h2>
          </div>
          <TabsList>
            <TabsTrigger value="data" className="max-sm:min-h-11">
              Data
            </TabsTrigger>
            <TabsTrigger value="definition" className="max-sm:min-h-11">
              Definition
            </TabsTrigger>
          </TabsList>
        </div>
        {summary && <SummaryBadges summary={summary} />}
      </header>
      <TabsContent value="data" className="flex min-h-0 flex-1 flex-col">
        <TableDataView connectionId={connectionId} object={object} summary={summary} />
      </TabsContent>
      <TabsContent value="definition" className="min-h-0 flex-1 overflow-y-auto">
        <PageContainer>
          <ObjectDetailsView connectionId={connectionId} object={object} />
        </PageContainer>
      </TabsContent>
    </Tabs>
  )
}

function SummaryBadges({ summary }: { summary: ObjectSummary }) {
  const Icon = KIND_ICONS[summary.kind]
  const rows = formatRowEstimate(summary.rowEstimate, { precise: true })

  return (
    <>
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant="secondary">
          <Icon data-icon="inline-start" />
          {KIND_LABELS[summary.kind]}
        </Badge>
        {rows && (
          <Badge variant="outline" title="Estimate from the database statistics">
            {rows}
          </Badge>
        )}
        {!summary.readable && (
          <Badge variant="outline">
            <LockIcon data-icon="inline-start" />
            No read access
          </Badge>
        )}
      </div>
      {summary.comment && (
        <p className="line-clamp-2 text-sm text-muted-foreground max-sm:hidden">
          {summary.comment}
        </p>
      )}
    </>
  )
}
