import type { AuditEntry, ConnectionId } from '@aidb/contracts'
import { useInfiniteQuery } from '@tanstack/react-query'
import { ClipboardCheckIcon, FingerprintIcon, HistoryIcon } from 'lucide-react'
import { useState } from 'react'
import { EnvironmentBadge } from '@/components/shared/environment-badge'
import { LoadError } from '@/components/shared/load-error'
import { PageContainer } from '@/components/shared/page'
import { EmptyState } from '@/components/shared/status-states'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Item,
  ItemContent,
  ItemDescription,
  ItemFooter,
  ItemGroup,
  ItemMedia,
  ItemTitle,
} from '@/components/ui/item'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { useConnections } from '@/features/connections/queries'
import { useActiveConnection } from '@/features/connections/sessions'
import { listAuditEvents } from '@/lib/commands'
import { PendingChangeCard } from './pending-change-card'
import { usePendingChanges } from './pending'
import {
  ACTOR_LABELS,
  nextBefore,
  operationLabel,
  PAGE_SIZE,
  shortFingerprint,
  STATUS_DISPLAY,
} from './display'

const timeFormat = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'medium' })

export function ChangesPage() {
  return (
    <PageContainer>
      <header className="flex flex-col gap-1">
        <h1 className="text-lg font-semibold">Changes</h1>
        <p className="text-sm text-muted-foreground">
          Changes waiting for your approval, and a record of what was approved, refused or run. The
          record keeps fingerprints and reasons, never SQL text or data values.
        </p>
      </header>
      <section aria-labelledby="pending-heading" className="flex flex-col gap-3">
        <h2 id="pending-heading" className="text-base font-medium">
          Waiting for review
        </h2>
        <Pending />
      </section>
      <History />
    </PageContainer>
  )
}

/** Changes the core holds for approval, on every connection: the assistant's proposals. */
function Pending() {
  const pending = usePendingChanges(null)
  if (pending.isPending) return <Skeleton className="h-16 w-full" />
  if (pending.isError) {
    return (
      <LoadError
        title="What waits for review could not be loaded"
        error={pending.error}
        retrying={pending.isFetching}
        onRetry={() => void pending.refetch()}
      />
    )
  }
  if (pending.data.length === 0) {
    return (
      <EmptyState
        icon={ClipboardCheckIcon}
        title="Nothing waits for review"
        description="Changes the assistant proposes wait here with a preview until you apply or decline them. Nothing is written before that."
      />
    )
  }
  return (
    <ItemGroup className="gap-2">
      {pending.data.map((change) => (
        <PendingChangeCard key={change.preview.approvalId} change={change} />
      ))}
    </ItemGroup>
  )
}

type Scope = 'connection' | 'all'

function History() {
  const active = useActiveConnection()
  const [chosen, setChosen] = useState<Scope>('connection')
  const scope: Scope = active ? chosen : 'all'
  const connectionId: ConnectionId | null = scope === 'connection' ? (active?.id ?? null) : null
  const connections = useConnections()
  const history = useInfiniteQuery({
    queryKey: ['audit', connectionId],
    queryFn: ({ pageParam }) =>
      listAuditEvents({ connectionId, beforeId: pageParam ?? null, limit: PAGE_SIZE }),
    initialPageParam: undefined as number | undefined,
    getNextPageParam: nextBefore,
  })
  const names = new Map(connections.data?.map((c) => [c.id, c.name]))
  const entries = history.data?.pages.flat() ?? []

  return (
    <section aria-labelledby="history-heading" className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <h2 id="history-heading" className="text-base font-medium">
          History
        </h2>
        {active && (
          <ToggleGroup
            type="single"
            variant="outline"
            size="sm"
            className="ml-auto"
            value={scope}
            onValueChange={(value) => value && setChosen(value as Scope)}
            aria-label="Which connections"
          >
            <ToggleGroupItem value="connection" className="max-sm:min-h-11">
              {active.name}
            </ToggleGroupItem>
            <ToggleGroupItem value="all" className="max-sm:min-h-11">
              All connections
            </ToggleGroupItem>
          </ToggleGroup>
        )}
      </div>

      {history.isPending ? (
        <div className="flex flex-col gap-2">
          <Skeleton className="h-16 w-full" />
          <Skeleton className="h-16 w-full" />
        </div>
      ) : history.isError ? (
        <LoadError
          title="The history could not be loaded"
          error={history.error}
          retrying={history.isFetching}
          onRetry={() => void history.refetch()}
        />
      ) : entries.length === 0 ? (
        <EmptyState
          icon={HistoryIcon}
          title="No history yet"
          description="Approvals, refused requests, assistant tasks and connection changes will be recorded here."
        />
      ) : (
        <>
          <ItemGroup className="gap-2">
            {entries.map((entry) => (
              <HistoryItem
                key={entry.id}
                entry={entry}
                connectionName={
                  entry.connectionId
                    ? (names.get(entry.connectionId) ?? 'A removed connection')
                    : null
                }
              />
            ))}
          </ItemGroup>
          {history.hasNextPage && (
            <Button
              variant="outline"
              className="self-center"
              disabled={history.isFetchingNextPage}
              onClick={() => void history.fetchNextPage()}
            >
              {history.isFetchingNextPage && <Spinner data-icon="inline-start" />}
              Show older
            </Button>
          )}
        </>
      )}
    </section>
  )
}

function HistoryItem({
  entry,
  connectionName,
}: {
  entry: AuditEntry
  connectionName: string | null
}) {
  const status = STATUS_DISPLAY[entry.status]
  const occurred = new Date(entry.occurredAt)
  return (
    <Item variant="outline" size="sm" role="listitem">
      <ItemMedia variant="icon">
        <status.icon />
      </ItemMedia>
      <ItemContent>
        <ItemTitle className="flex flex-wrap items-center gap-2">
          {operationLabel(entry.operation)}
          <Badge variant={status.variant}>{status.label}</Badge>
          {entry.risk && <Badge variant="outline">Risk {entry.risk}</Badge>}
        </ItemTitle>
        {entry.detail && (
          <ItemDescription className="line-clamp-none">{entry.detail}</ItemDescription>
        )}
      </ItemContent>
      <ItemFooter className="flex-wrap justify-start gap-x-3 gap-y-1 text-xs text-muted-foreground">
        <time dateTime={entry.occurredAt} title={entry.occurredAt}>
          {Number.isNaN(occurred.getTime()) ? entry.occurredAt : timeFormat.format(occurred)}
        </time>
        <span>{ACTOR_LABELS[entry.actor]}</span>
        {connectionName && <span>{connectionName}</span>}
        {entry.environment && <EnvironmentBadge environment={entry.environment} />}
        {entry.fingerprint && (
          <span className="inline-flex items-center gap-1 font-mono" title={entry.fingerprint}>
            <FingerprintIcon aria-hidden className="size-3" />
            <span className="sr-only">Fingerprint </span>
            {shortFingerprint(entry.fingerprint)}
          </span>
        )}
      </ItemFooter>
    </Item>
  )
}
