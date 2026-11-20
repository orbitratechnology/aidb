import type { ConnectionId, MigrationRecord, MigrationStatus } from '@aidb/contracts'
import { useQuery } from '@tanstack/react-query'
import {
  CircleCheckIcon,
  CircleDashedIcon,
  CircleHelpIcon,
  CircleXIcon,
  HistoryIcon,
  TriangleAlertIcon,
  type LucideIcon,
} from 'lucide-react'
import { EnvironmentBadge } from '@/components/shared/environment-badge'
import { LoadError } from '@/components/shared/load-error'
import { EmptyState } from '@/components/shared/status-states'
import { Badge } from '@/components/ui/badge'
import {
  Item,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemMedia,
  ItemTitle,
} from '@/components/ui/item'
import { Skeleton } from '@/components/ui/skeleton'
import { listMigrationRecords } from '@/lib/commands'
import {
  MIGRATION_RECORDS_KEY,
  needsAttention,
  STATUS_LABELS,
  stepsText,
  VERIFICATION_LABELS,
} from './history'

const STATUS_ICONS: Record<MigrationStatus, LucideIcon> = {
  running: CircleDashedIcon,
  succeeded: CircleCheckIcon,
  failed: CircleXIcon,
  partial: TriangleAlertIcon,
  uncertain: CircleHelpIcon,
}

const time = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' })

/** Migrations applied on a connection, newest first. */
export function MigrationHistory({ connectionId }: { connectionId: ConnectionId }) {
  const records = useQuery({
    queryKey: [...MIGRATION_RECORDS_KEY, connectionId],
    queryFn: () => listMigrationRecords(connectionId),
  })
  return (
    <section aria-labelledby="migration-history" className="flex flex-col gap-3">
      <h2 id="migration-history" className="text-base font-medium">
        Applied here
      </h2>
      {records.isPending ? (
        <Skeleton className="h-16 w-full" />
      ) : records.isError ? (
        <LoadError
          title="The migration history could not be loaded"
          error={records.error}
          retrying={records.isFetching}
          onRetry={() => void records.refetch()}
        />
      ) : records.data.length === 0 ? (
        <EmptyState
          icon={HistoryIcon}
          title="No migrations yet"
          description="Migrations applied from this page are recorded here: when, how far they got and whether the schemas matched afterwards. The record keeps a digest of the script, not its text."
        />
      ) : (
        <ItemGroup className="gap-2">
          {records.data.map((record) => (
            <RecordItem key={record.id} record={record} />
          ))}
        </ItemGroup>
      )}
    </section>
  )
}

function RecordItem({ record }: { record: MigrationRecord }) {
  const Icon = STATUS_ICONS[record.status]
  const attention = needsAttention(record.status)
  return (
    <Item variant="outline" size="sm" role="listitem" className="items-start">
      <ItemMedia variant="icon">
        <Icon className={record.status === 'succeeded' ? undefined : 'text-destructive'} />
      </ItemMedia>
      <ItemContent className="min-w-0 gap-1.5">
        <ItemTitle className="flex flex-wrap items-center gap-2">
          <span className="break-words">
            {record.targetSchema} to match {record.source}
          </span>
          <Badge variant={record.status === 'succeeded' ? 'secondary' : 'destructive'}>
            {STATUS_LABELS[record.status]}
          </Badge>
          {record.status !== 'failed' && (
            <Badge variant={record.verification === 'failed' ? 'destructive' : 'outline'}>
              {VERIFICATION_LABELS[record.verification]}
            </Badge>
          )}
        </ItemTitle>
        {record.detail && (
          <ItemDescription className="line-clamp-none">{record.detail}</ItemDescription>
        )}
        {attention && (
          <p className="text-sm text-destructive">
            The schema may be between two states. Compare again to see where it stands.
          </p>
        )}
        <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          <span>{time.format(new Date(record.startedAt))}</span>
          {record.environment && <EnvironmentBadge environment={record.environment} />}
          <span>{stepsText(record)}</span>
          <Badge variant="outline">Risk {record.risk}</Badge>
          <span className="font-mono" title="SHA-256 of the script">
            {record.scriptSha256.slice(0, 12)}
          </span>
        </div>
      </ItemContent>
    </Item>
  )
}
