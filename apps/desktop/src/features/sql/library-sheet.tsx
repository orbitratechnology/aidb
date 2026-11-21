import { useDeferredValue, useState, type ReactNode } from 'react'
import type { ConnectionId, HistoryEntry, SavedQuery } from '@aidb/contracts'
import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import {
  BanIcon,
  BookmarkIcon,
  CircleAlertIcon,
  CircleCheckIcon,
  CircleSlashIcon,
  HistoryIcon,
  SearchIcon,
  SquarePenIcon,
  Trash2Icon,
  type LucideIcon,
} from 'lucide-react'
import { toast } from 'sonner'
import { LoadError } from '@/components/shared/load-error'
import { EmptyState } from '@/components/shared/status-states'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { InputGroup, InputGroupAddon, InputGroupInput } from '@/components/ui/input-group'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { Skeleton } from '@/components/ui/skeleton'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import {
  deleteQueryHistoryEntry,
  deleteSavedQuery,
  listQueryHistory,
  listSavedQueries,
} from '@/lib/commands'
import { HISTORY_KEY, historyOutcome, LIST_LIMIT, SAVED_KEY, sqlPreview } from './library-view'
import { returnFocus } from '@/lib/focus-return'

const timeFormat = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' })

function when(timestamp: string): string {
  const date = new Date(timestamp)
  return Number.isNaN(date.getTime()) ? timestamp : timeFormat.format(date)
}

const STATUS_ICONS: Record<HistoryEntry['status'], LucideIcon> = {
  completed: CircleCheckIcon,
  refused: BanIcon,
  failed: CircleAlertIcon,
  cancelled: CircleSlashIcon,
}

type Props = {
  open: boolean
  onOpenChange: (open: boolean) => void
  connectionId: ConnectionId
  /** Opens SQL in an editor tab; `saved` links the tab to that saved query. */
  onOpenSql: (sql: string, saved?: SavedQuery) => void
}

/** Statements run on this connection, and saved queries, to search and open again. */
export function LibrarySheet({ open, onOpenChange, connectionId, onOpenSql }: Props) {
  const [tab, setTab] = useState<'history' | 'saved'>('history')
  const openSql = (sql: string, saved?: SavedQuery) => {
    onOpenSql(sql, saved)
    onOpenChange(false)
  }

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        {...returnFocus}
        side="right"
        className="gap-0 data-[side=right]:w-full data-[side=right]:sm:max-w-lg"
      >
        <SheetHeader className="border-b">
          <SheetTitle>Query library</SheetTitle>
          <SheetDescription>
            Statements run on this connection, and the queries you saved. Opening one puts it in a
            new tab; nothing runs until you run it.
          </SheetDescription>
        </SheetHeader>
        <Tabs
          value={tab}
          onValueChange={(value) => setTab(value === 'saved' ? 'saved' : 'history')}
          className="min-h-0 flex-1 gap-0"
        >
          <TabsList className="mx-4 mt-3 grid w-auto grid-cols-2 max-sm:h-11!">
            <TabsTrigger value="history">
              <HistoryIcon />
              History
            </TabsTrigger>
            <TabsTrigger value="saved">
              <BookmarkIcon />
              Saved
            </TabsTrigger>
          </TabsList>
          {/* Mounted only while open, so lists are fresh and searches start empty. */}
          {open && (
            <>
              <TabsContent value="history" className="flex min-h-0 flex-col">
                <HistoryPanel connectionId={connectionId} onOpenSql={openSql} />
              </TabsContent>
              <TabsContent value="saved" className="flex min-h-0 flex-col">
                <SavedPanel onOpenSql={openSql} />
              </TabsContent>
            </>
          )}
        </Tabs>
      </SheetContent>
    </Sheet>
  )
}

function SearchBox({
  value,
  onChange,
  label,
}: {
  value: string
  onChange: (value: string) => void
  label: string
}) {
  return (
    <div className="p-4 pb-2">
      <InputGroup className="max-sm:h-11">
        <InputGroupAddon>
          <SearchIcon />
        </InputGroupAddon>
        <InputGroupInput
          type="search"
          value={value}
          onChange={(event) => onChange(event.target.value)}
          placeholder={label}
          aria-label={label}
          autoComplete="off"
          spellCheck={false}
        />
      </InputGroup>
    </div>
  )
}

function ListBody<T>({
  query,
  searching,
  empty,
  failure,
  children,
}: {
  query: {
    data: T[] | undefined
    isPending: boolean
    isError: boolean
    error: Error | null
    isFetching: boolean
    refetch: () => unknown
  }
  searching: boolean
  empty: { icon: LucideIcon; title: string; description: string }
  failure: string
  children: (items: T[]) => ReactNode
}) {
  if (query.isPending) {
    return (
      <div className="flex flex-col gap-2 p-4">
        <Skeleton className="h-20" />
        <Skeleton className="h-20" />
      </div>
    )
  }
  if (query.isError && query.error) {
    return (
      <div className="p-4">
        <LoadError
          title={failure}
          error={query.error}
          retrying={query.isFetching}
          onRetry={() => void query.refetch()}
        />
      </div>
    )
  }
  const items = query.data ?? []
  if (items.length === 0) {
    return (
      <div className="p-4">
        {searching ? (
          <EmptyState icon={SearchIcon} title="No matches" description="Try other words." />
        ) : (
          <EmptyState {...empty} />
        )}
      </div>
    )
  }
  return (
    <div className="min-h-0 flex-1 overflow-y-auto" aria-busy={query.isFetching}>
      <ul className="flex flex-col divide-y">{children(items)}</ul>
      {items.length >= LIST_LIMIT && (
        <p className="p-4 text-xs text-muted-foreground">
          Showing the latest {LIST_LIMIT}. Search to find older ones.
        </p>
      )}
    </div>
  )
}

function SqlPreview({ sql }: { sql: string }) {
  return (
    <pre className="overflow-hidden font-mono text-xs break-all whitespace-pre-wrap text-foreground">
      {sqlPreview(sql)}
    </pre>
  )
}

function HistoryPanel({
  connectionId,
  onOpenSql,
}: {
  connectionId: ConnectionId
  onOpenSql: (sql: string) => void
}) {
  const queryClient = useQueryClient()
  const [search, setSearch] = useState('')
  const term = useDeferredValue(search.trim())
  const history = useQuery({
    queryKey: [HISTORY_KEY, connectionId, term],
    queryFn: () => listQueryHistory(connectionId, term),
    placeholderData: keepPreviousData,
  })
  const remove = useMutation({
    mutationFn: deleteQueryHistoryEntry,
    onError: (error) => toast.error("Couldn't remove the entry", { description: error.message }),
    onSettled: () => queryClient.invalidateQueries({ queryKey: [HISTORY_KEY, connectionId] }),
  })

  return (
    <>
      <SearchBox value={search} onChange={setSearch} label="Search history" />
      <ListBody
        query={history}
        searching={term !== ''}
        failure="History could not be read"
        empty={{
          icon: HistoryIcon,
          title: 'No history yet',
          description:
            'Statements you run on this connection are listed here, with passwords and keys removed.',
        }}
      >
        {(entries) =>
          entries.map((entry) => {
            const Icon = STATUS_ICONS[entry.status]
            return (
              <li key={entry.id} className="flex flex-col gap-2 px-4 py-3">
                <SqlPreview sql={entry.sql} />
                <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
                  <span
                    className={
                      entry.status === 'failed'
                        ? 'flex items-center gap-1 text-destructive'
                        : 'flex items-center gap-1'
                    }
                  >
                    <Icon aria-hidden className="size-3.5" />
                    {historyOutcome(entry)}
                  </span>
                  <time dateTime={entry.executedAt}>{when(entry.executedAt)}</time>
                  <span className="ml-auto flex items-center gap-1">
                    <Button
                      variant="outline"
                      size="sm"
                      className="max-sm:min-h-11"
                      onClick={() => onOpenSql(entry.sql)}
                    >
                      <SquarePenIcon data-icon="inline-start" />
                      Open
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      className="max-sm:size-11"
                      aria-label="Remove from history"
                      title="Remove from history"
                      disabled={remove.isPending && remove.variables === entry.id}
                      onClick={() => remove.mutate(entry.id)}
                    >
                      <Trash2Icon />
                    </Button>
                  </span>
                </div>
              </li>
            )
          })
        }
      </ListBody>
    </>
  )
}

function SavedPanel({ onOpenSql }: { onOpenSql: (sql: string, saved: SavedQuery) => void }) {
  const [search, setSearch] = useState('')
  const term = useDeferredValue(search.trim())
  const saved = useQuery({
    queryKey: [SAVED_KEY, term],
    queryFn: () => listSavedQueries(term),
    placeholderData: keepPreviousData,
  })

  return (
    <>
      <SearchBox value={search} onChange={setSearch} label="Search saved queries" />
      <ListBody
        query={saved}
        searching={term !== ''}
        failure="Saved queries could not be read"
        empty={{
          icon: BookmarkIcon,
          title: 'No saved queries',
          description: 'Use Save in the editor to keep a query here.',
        }}
      >
        {(queries) =>
          queries.map((query) => (
            <li key={query.id} className="flex flex-col gap-2 px-4 py-3">
              <p className="truncate text-sm font-medium">{query.name}</p>
              <SqlPreview sql={query.sql} />
              <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
                <span>
                  Updated <time dateTime={query.updatedAt}>{when(query.updatedAt)}</time>
                </span>
                <span className="ml-auto flex items-center gap-1">
                  <Button
                    variant="outline"
                    size="sm"
                    className="max-sm:min-h-11"
                    onClick={() => onOpenSql(query.sql, query)}
                  >
                    <SquarePenIcon data-icon="inline-start" />
                    Open
                  </Button>
                  <DeleteSaved query={query} />
                </span>
              </div>
            </li>
          ))
        }
      </ListBody>
    </>
  )
}

function DeleteSaved({ query }: { query: SavedQuery }) {
  const queryClient = useQueryClient()
  const remove = useMutation({
    mutationFn: deleteSavedQuery,
    onSuccess: () => toast.success(`Deleted ${query.name}`),
    onError: (error) => toast.error("Couldn't delete the query", { description: error.message }),
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: [SAVED_KEY] })
      await queryClient.invalidateQueries({ queryKey: ['storage-summary'] })
    },
  })

  return (
    <AlertDialog>
      <AlertDialogTrigger asChild>
        <Button
          variant="ghost"
          size="icon-sm"
          className="max-sm:size-11"
          aria-label={`Delete ${query.name}`}
          title="Delete"
          disabled={remove.isPending}
        >
          <Trash2Icon />
        </Button>
      </AlertDialogTrigger>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete {query.name}?</AlertDialogTitle>
          <AlertDialogDescription>
            The saved query is removed from this device. Editor tabs that have it open keep their
            text. This cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction variant="destructive" onClick={() => remove.mutate(query.id)}>
            Delete
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
