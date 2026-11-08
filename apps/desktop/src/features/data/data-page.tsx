import { useState, type ReactNode } from 'react'
import type { CatalogSnapshot, ConnectionId, ObjectName } from '@aidb/contracts'
import { ListTreeIcon, MousePointerClickIcon, RefreshCwIcon, Table2Icon } from 'lucide-react'
import { useParams } from 'react-router'
import { toast } from 'sonner'
import { LoadError } from '@/components/shared/load-error'
import { PageContainer } from '@/components/shared/page'
import { ConnectionRequired, EmptyState } from '@/components/shared/status-states'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from '@/components/ui/sheet'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { useActiveConnection } from '@/features/connections/sessions'
import { useMediaQuery } from '@/hooks/use-media-query'
import { ObjectDetailsView } from './object-details'
import { ObjectTree } from './object-tree'
import { useCatalog, useRefreshCatalog } from './queries'

const WIDE_QUERY = '(min-width: 1024px)'

export function DataPage() {
  const active = useActiveConnection()

  if (!active) {
    return (
      <PageContainer>
        <ConnectionRequired
          icon={Table2Icon}
          title="Browse your database"
          description="Connect to a PostgreSQL database to explore its schemas, tables and records."
        />
      </PageContainer>
    )
  }
  // Keyed by connection, so switching databases starts from a fresh tree and search.
  return <SchemaBrowser key={active.id} connectionId={active.id} />
}

/** The object named in the URL (`/data/:schema/:name`), if any. */
function useSelectedObject(): ObjectName | null {
  const { schema, name } = useParams()
  return schema !== undefined && name !== undefined ? { schema, name } : null
}

function SchemaBrowser({ connectionId }: { connectionId: ConnectionId }) {
  const catalog = useCatalog(connectionId)
  const selected = useSelectedObject()
  const wide = useMediaQuery(WIDE_QUERY)

  if (catalog.isPending) {
    return (
      <PageContainer aria-busy="true" aria-label="Loading schema">
        <Skeleton className="h-9 w-full max-w-sm" />
        <Skeleton className="h-64 w-full" />
      </PageContainer>
    )
  }
  if (catalog.isError) {
    return (
      <PageContainer>
        <LoadError
          title="Could not load the schema"
          error={catalog.error}
          retrying={catalog.isFetching}
          onRetry={() => void catalog.refetch()}
        />
      </PageContainer>
    )
  }

  const details = selected ? (
    <ObjectDetailsView connectionId={connectionId} object={selected} />
  ) : (
    <EmptyState
      icon={MousePointerClickIcon}
      title="Pick a table or view"
      description={
        wide
          ? 'Choose an object from the list to see its columns, keys and relationships.'
          : 'Browse the objects to see a table’s columns, keys and relationships.'
      }
    />
  )

  if (wide) {
    return (
      <div className="flex min-h-0 flex-1">
        <aside
          aria-label="Schema"
          className="flex w-80 shrink-0 flex-col border-r bg-sidebar text-sidebar-foreground"
        >
          <CatalogToolbar connectionId={connectionId} catalog={catalog.data} />
          <ObjectTree catalog={catalog.data} selected={selected} />
        </aside>
        {/* Keyed by the selection, so each object opens scrolled to its top. */}
        <div key={JSON.stringify(selected)} className="min-w-0 flex-1 overflow-y-auto">
          <PageContainer>{details}</PageContainer>
        </div>
      </div>
    )
  }

  return (
    <PageContainer>
      <div className="rounded-md border">
        <CatalogToolbar connectionId={connectionId} catalog={catalog.data}>
          <ObjectPickerSheet catalog={catalog.data} selected={selected} />
        </CatalogToolbar>
      </div>
      {details}
    </PageContainer>
  )
}

const timeFormat = new Intl.DateTimeFormat(undefined, { timeStyle: 'short' })

function CatalogToolbar({
  connectionId,
  catalog,
  children,
}: {
  connectionId: ConnectionId
  catalog: CatalogSnapshot
  children?: ReactNode
}) {
  const refresh = useRefreshCatalog(connectionId)
  const loadedAt = new Date(catalog.loadedAt)

  const onRefresh = () =>
    refresh.mutate(undefined, {
      onError: (error) =>
        toast.error('Could not refresh the schema', { description: error.message }),
    })

  return (
    <div className="flex flex-col gap-2 border-b p-3 last:border-b-0">
      <div className="flex items-center gap-2">
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium">{catalog.database}</p>
          <p className="text-xs text-muted-foreground">
            Schema loaded at <time dateTime={catalog.loadedAt}>{timeFormat.format(loadedAt)}</time>
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          className="max-sm:min-h-11"
          disabled={refresh.isPending}
          onClick={onRefresh}
        >
          {refresh.isPending ? (
            <Spinner data-icon="inline-start" />
          ) : (
            <RefreshCwIcon data-icon="inline-start" />
          )}
          Refresh
        </Button>
      </div>
      {catalog.truncated && (
        <Alert>
          <AlertDescription>
            This database has more tables and views than aidb loads at once. Only the first{' '}
            {catalog.objects.length.toLocaleString()} are listed and searchable.
          </AlertDescription>
        </Alert>
      )}
      {children}
    </div>
  )
}

function ObjectPickerSheet({
  catalog,
  selected,
}: {
  catalog: CatalogSnapshot
  selected: ObjectName | null
}) {
  const [open, setOpen] = useState(false)

  return (
    <Sheet open={open} onOpenChange={setOpen}>
      <SheetTrigger asChild>
        <Button variant="secondary" className="min-h-11 w-full justify-start">
          <ListTreeIcon data-icon="inline-start" />
          {selected ? `${selected.schema}.${selected.name}` : 'Browse tables and views'}
        </Button>
      </SheetTrigger>
      <SheetContent
        side="left"
        className="gap-0 data-[side=left]:w-full data-[side=left]:sm:max-w-sm"
      >
        <SheetHeader className="border-b">
          <SheetTitle>Tables and views</SheetTitle>
          <SheetDescription>{catalog.database}</SheetDescription>
        </SheetHeader>
        <ObjectTree catalog={catalog} selected={selected} onPick={() => setOpen(false)} />
      </SheetContent>
    </Sheet>
  )
}
