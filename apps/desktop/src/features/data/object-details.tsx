import type { ReactNode } from 'react'
import type { ConnectionId, ForeignKey, ObjectDetails, ObjectName } from '@aidb/contracts'
import { ArrowRightIcon, KeyRoundIcon, LinkIcon, SearchXIcon } from 'lucide-react'
import { Link } from 'react-router'
import { LoadError } from '@/components/shared/load-error'
import { EmptyState } from '@/components/shared/status-states'
import { Badge } from '@/components/ui/badge'
import { Skeleton } from '@/components/ui/skeleton'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { NativeError } from '@/lib/native'
import { ACTION_LABELS, columnKeys, objectPath, sameObject } from './catalog-view'
import { useObjectDetails } from './queries'

/** Definition of one table or view: columns, keys, relationships and indexes. */
export function ObjectDetailsView({
  connectionId,
  object,
}: {
  connectionId: ConnectionId
  object: ObjectName
}) {
  const details = useObjectDetails(connectionId, object)

  if (details.isPending) {
    return <DetailsSkeleton />
  }
  if (details.isError) {
    if (details.error instanceof NativeError && details.error.code === 'not_found') {
      return (
        <EmptyState
          icon={SearchXIcon}
          title={`${object.schema}.${object.name} was not found`}
          description="It may have been dropped or renamed. The list now shows the current schema."
        />
      )
    }
    return (
      <LoadError
        title={`Could not load ${object.schema}.${object.name}`}
        error={details.error}
        retrying={details.isFetching}
        onRetry={() => void details.refetch()}
      />
    )
  }
  return <Definition details={details.data} />
}

function Definition({ details }: { details: ObjectDetails }) {
  const { summary } = details
  const isView = summary.kind === 'view'
  const hasKeys = !isView && summary.kind !== 'materialized_view'

  return (
    <div className="flex flex-col gap-8">
      <Section title="Columns" count={details.columns.length}>
        <ColumnsTable details={details} />
      </Section>

      {hasKeys && <KeysSection details={details} />}

      {hasKeys && (
        <Section title="References" count={details.foreignKeys.length}>
          <Relationships keys={details.foreignKeys} direction="outgoing" self={summary} />
        </Section>
      )}

      {hasKeys && (
        <Section title="Referenced by" count={details.referencedBy.length}>
          <Relationships keys={details.referencedBy} direction="incoming" self={summary} />
        </Section>
      )}

      {!isView && (
        <Section title="Indexes" count={details.indexes.length}>
          <IndexesTable details={details} />
        </Section>
      )}

      {details.definition !== null && (
        <Section title="Query">
          <pre className="overflow-x-auto rounded-md border bg-muted p-3 font-mono text-sm">
            {details.definition}
          </pre>
        </Section>
      )}
    </div>
  )
}

function Section({
  title,
  count,
  children,
}: {
  title: string
  count?: number
  children: ReactNode
}) {
  return (
    <section className="flex flex-col gap-3">
      <h3 className="flex items-baseline gap-2 text-lg font-semibold">
        {title}
        {count !== undefined && (
          <span className="text-sm font-normal text-muted-foreground">{count}</span>
        )}
      </h3>
      {children}
    </section>
  )
}

function None({ children }: { children: ReactNode }) {
  return <p className="text-sm text-muted-foreground">{children}</p>
}

function ColumnsTable({ details }: { details: ObjectDetails }) {
  const keys = columnKeys(details)

  if (details.columns.length === 0) {
    return <None>No columns.</None>
  }

  return (
    <div className="rounded-md border">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Name</TableHead>
            <TableHead>Type</TableHead>
            <TableHead>Null</TableHead>
            <TableHead>Default</TableHead>
            <TableHead>Comment</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {details.columns.map((column) => {
            const key = keys.get(column.name)
            return (
              <TableRow key={column.name}>
                <TableCell>
                  <div className="flex items-center gap-2">
                    <span className="font-medium">{column.name}</span>
                    {key?.primary && <KeyBadge label="PK" title="Primary key" />}
                    {key?.foreign && <KeyBadge label="FK" title="Foreign key" />}
                    {key?.unique && <KeyBadge label="UQ" title="Unique" />}
                  </div>
                </TableCell>
                <TableCell className="font-mono text-xs">{column.dataType}</TableCell>
                <TableCell>{column.nullable ? 'null' : 'not null'}</TableCell>
                <TableCell className="min-w-40 font-mono text-xs break-all whitespace-normal">
                  {columnDefault(column)}
                </TableCell>
                <TableCell className="min-w-40 whitespace-normal text-muted-foreground">
                  {column.comment}
                </TableCell>
              </TableRow>
            )
          })}
        </TableBody>
      </Table>
    </div>
  )
}

function columnDefault(column: ObjectDetails['columns'][number]): string | null {
  if (column.identity === 'always') return 'generated always as identity'
  if (column.identity === 'by_default') return 'generated by default as identity'
  if (column.generated !== null) return `generated always as (${column.generated}) stored`
  return column.default
}

function KeyBadge({ label, title }: { label: string; title: string }) {
  return (
    <Badge variant="outline" className="px-1.5 font-mono text-[0.625rem]" title={title}>
      {label}
      <span className="sr-only"> ({title})</span>
    </Badge>
  )
}

function KeysSection({ details }: { details: ObjectDetails }) {
  const keys = [
    ...(details.primaryKey ? [{ ...details.primaryKey, label: 'Primary key' }] : []),
    ...details.uniqueConstraints.map((unique) => ({ ...unique, label: 'Unique' })),
  ]

  return (
    <Section title="Keys" count={keys.length}>
      {keys.length === 0 ? (
        <None>No primary key or unique constraints.</None>
      ) : (
        <ul className="flex flex-col gap-2">
          {keys.map((key) => (
            <li key={key.name} className="flex flex-wrap items-center gap-2 text-sm">
              <KeyRoundIcon aria-hidden className="size-4 text-muted-foreground" />
              <span className="font-medium">{key.label}</span>
              <span className="font-mono text-xs">({key.columns.join(', ')})</span>
              <span className="text-muted-foreground">{key.name}</span>
            </li>
          ))}
        </ul>
      )}
    </Section>
  )
}

function Relationships({
  keys,
  direction,
  self,
}: {
  keys: ForeignKey[]
  direction: 'outgoing' | 'incoming'
  self: ObjectName
}) {
  if (keys.length === 0) {
    return (
      <None>
        {direction === 'outgoing'
          ? 'This table does not reference other tables.'
          : 'No other table references this one.'}
      </None>
    )
  }

  return (
    <ul className="flex flex-col gap-3">
      {keys.map((key) => {
        const other = direction === 'outgoing' ? key.to : key.from
        const [ownColumns, otherColumns] =
          direction === 'outgoing'
            ? [key.columns, key.referencedColumns]
            : [key.referencedColumns, key.columns]
        return (
          <li
            key={`${key.from.schema}.${key.from.name}.${key.name}`}
            className="flex flex-col gap-1 rounded-md border p-3 text-sm"
          >
            <div className="flex flex-wrap items-center gap-2">
              <span className="font-mono text-xs">({ownColumns.join(', ')})</span>
              <ArrowRightIcon aria-hidden className="size-4 text-muted-foreground" />
              <span className="sr-only">
                {direction === 'outgoing' ? 'references' : 'is referenced by'}
              </span>
              <ObjectLink object={other} self={self} />
              <span className="font-mono text-xs">({otherColumns.join(', ')})</span>
            </div>
            <p className="text-xs text-muted-foreground">
              {key.name} · on update {ACTION_LABELS[key.onUpdate]} · on delete{' '}
              {ACTION_LABELS[key.onDelete]}
            </p>
          </li>
        )
      })}
    </ul>
  )
}

function ObjectLink({ object, self }: { object: ObjectName; self: ObjectName }) {
  const label = `${object.schema}.${object.name}`
  if (sameObject(object, self)) {
    return <span className="font-medium">{label} (itself)</span>
  }
  return (
    <Link
      to={objectPath(object)}
      className="inline-flex items-center gap-1 font-medium underline-offset-4 hover:underline"
    >
      <LinkIcon aria-hidden className="size-3.5" />
      {label}
    </Link>
  )
}

function IndexesTable({ details }: { details: ObjectDetails }) {
  if (details.indexes.length === 0) {
    return <None>No indexes.</None>
  }

  return (
    <div className="rounded-md border">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Name</TableHead>
            <TableHead>Method</TableHead>
            <TableHead>Definition</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {details.indexes.map((index) => (
            <TableRow key={index.name}>
              <TableCell>
                <div className="flex items-center gap-2">
                  <span className="font-medium">{index.name}</span>
                  {index.primary ? (
                    <Badge variant="outline">Primary</Badge>
                  ) : (
                    index.unique && <Badge variant="outline">Unique</Badge>
                  )}
                </div>
              </TableCell>
              <TableCell>{index.method}</TableCell>
              <TableCell className="font-mono text-xs whitespace-normal">
                {index.definition}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  )
}

function DetailsSkeleton() {
  return (
    <div className="flex flex-col gap-3" aria-busy="true" aria-label="Loading definition">
      <Skeleton className="h-6 w-32" />
      <Skeleton className="h-64 w-full" />
    </div>
  )
}
