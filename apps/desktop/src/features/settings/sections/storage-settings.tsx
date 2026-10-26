import { Fragment } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Trash2Icon } from 'lucide-react'
import { toast } from 'sonner'
import type { ClearTarget, StorageSummary } from '@aidb/contracts'
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
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemSeparator,
  ItemTitle,
} from '@/components/ui/item'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { clearLocalData, getStorageSummary } from '@/lib/commands'
import { LoadError } from '../load-error'
import { SettingsSection } from '../settings-section'

const STORAGE_QUERY_KEY = ['storage-summary'] as const

type Noun = { one: string; other: string }

type ClearSpec = {
  target: ClearTarget
  action: string
  confirmTitle: string
  consequence: string
}

type Row = {
  key: keyof StorageSummary
  title: string
  description: string
  noun: Noun
  clear?: ClearSpec
}

const ROWS: readonly Row[] = [
  {
    key: 'connections',
    title: 'Connection profiles',
    description: 'Managed under Connections. Passwords stay in the system keychain, not here.',
    noun: { one: 'profile', other: 'profiles' },
  },
  {
    key: 'savedQueries',
    title: 'Saved queries',
    description: 'Kept until you delete them.',
    noun: { one: 'query', other: 'queries' },
  },
  {
    key: 'agentTasks',
    title: 'Agent task history',
    description: 'Past agent requests with their plans and outcomes.',
    noun: { one: 'task', other: 'tasks' },
    clear: {
      target: 'task_history',
      action: 'Clear history',
      confirmTitle: 'Clear agent task history?',
      consequence: 'Their plans and outcomes are removed from this device.',
    },
  },
  {
    key: 'auditRecords',
    title: 'Audit log',
    description: 'A record of the queries and changes run through aidb.',
    noun: { one: 'record', other: 'records' },
    clear: {
      target: 'audit_log',
      action: 'Clear log',
      confirmTitle: 'Clear the audit log?',
      consequence: 'They are removed from this device. New activity is still recorded.',
    },
  },
  {
    key: 'schemaCacheEntries',
    title: 'Cached schema details',
    description: 'Database structure kept so browsing opens quickly. Never row data.',
    noun: { one: 'connection', other: 'connections' },
    clear: {
      target: 'schema_cache',
      action: 'Clear cache',
      confirmTitle: 'Clear cached schema details?',
      consequence: 'They are fetched again on the next connection. Your databases are not changed.',
    },
  },
]

const formatNumber = new Intl.NumberFormat('en-US')

function describeCount(count: number, noun: Noun): string {
  return `${formatNumber.format(count)} ${count === 1 ? noun.one : noun.other}`
}

export function StorageSettings() {
  const summary = useQuery({ queryKey: STORAGE_QUERY_KEY, queryFn: getStorageSummary })

  return (
    <SettingsSection
      title="Storage"
      description="History, cached schema details and audit records stay on this device."
    >
      {summary.isPending && <Skeleton className="h-80 w-full" />}
      {summary.isError && (
        <LoadError
          title="Local data could not be read"
          error={summary.error}
          retrying={summary.isFetching}
          onRetry={() => void summary.refetch()}
        />
      )}
      {summary.isSuccess && (
        <ItemGroup className="gap-0 rounded-lg border">
          {ROWS.map((row, index) => (
            <Fragment key={row.key}>
              {index > 0 && <ItemSeparator className="my-0" />}
              <StorageRow row={row} count={summary.data[row.key]} />
            </Fragment>
          ))}
        </ItemGroup>
      )}
    </SettingsSection>
  )
}

function StorageRow({ row, count }: { row: Row; count: number }) {
  return (
    <Item role="listitem">
      <ItemContent>
        <ItemTitle>{row.title}</ItemTitle>
        <ItemDescription>{row.description}</ItemDescription>
      </ItemContent>
      <ItemActions className="w-full justify-between sm:w-auto">
        <span className="text-sm text-muted-foreground tabular-nums">
          {describeCount(count, row.noun)}
        </span>
        {row.clear && <ClearButton spec={row.clear} count={count} noun={row.noun} />}
      </ItemActions>
    </Item>
  )
}

function ClearButton({ spec, count, noun }: { spec: ClearSpec; count: number; noun: Noun }) {
  const queryClient = useQueryClient()
  const { mutate, isPending } = useMutation({
    mutationFn: clearLocalData,
    onSuccess: ({ removed }) => toast.success(`Removed ${describeCount(removed, noun)}`),
    onError: (error) =>
      toast.error(`Couldn't ${spec.action.toLowerCase()}`, { description: error.message }),
    onSettled: () => queryClient.invalidateQueries({ queryKey: STORAGE_QUERY_KEY }),
  })

  return (
    <AlertDialog>
      <AlertDialogTrigger asChild>
        <Button
          variant="outline"
          size="sm"
          disabled={count === 0 || isPending}
          className="min-h-11 sm:min-h-0"
        >
          {isPending ? (
            <Spinner data-icon="inline-start" />
          ) : (
            <Trash2Icon data-icon="inline-start" />
          )}
          {spec.action}
        </Button>
      </AlertDialogTrigger>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{spec.confirmTitle}</AlertDialogTitle>
          <AlertDialogDescription>
            {describeCount(count, noun)}. {spec.consequence} This cannot be undone.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>Cancel</AlertDialogCancel>
          <AlertDialogAction variant="destructive" onClick={() => mutate(spec.target)}>
            {spec.action}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
