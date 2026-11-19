import { useState } from 'react'
import {
  EXPORT_LIMITS,
  type ConnectionId,
  type ExportFormat,
  type Filter,
  type ObjectName,
  type SortKey,
} from '@aidb/contracts'
import { DownloadIcon } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Spinner } from '@/components/ui/spinner'
import { pluralize } from '@/features/connections/format'
import { cancelOperation, exportTable } from '@/lib/commands'
import { NativeError } from '@/lib/native'

const maxRows = EXPORT_LIMITS.maxRows.toLocaleString('en-US')

/**
 * Exports every row that matches the filters, in the current order, to a file the user picks.
 * The native core writes the file; the UI only learns its name.
 */
export function ExportMenu({
  connectionId,
  object,
  filters,
  sort,
}: {
  connectionId: ConnectionId
  object: ObjectName
  filters: Filter[]
  sort: SortKey[]
}) {
  const [running, setRunning] = useState<string | null>(null)

  const run = async (format: ExportFormat) => {
    const operationId = crypto.randomUUID()
    setRunning(operationId)
    try {
      const outcome = await exportTable(connectionId, {
        object,
        filters,
        sort,
        format,
        operationId,
      })
      if (!outcome) return
      if (outcome.truncated) {
        toast.warning(
          `Exported the first ${pluralize(outcome.rows, 'row', 'rows')} to ${outcome.fileName}`,
          {
            description: `Exports stop at ${outcome.maxRows.toLocaleString('en-US')} rows and more rows match. Add filters to export the rest in parts.`,
            duration: 15_000,
          },
        )
      } else {
        toast.success(`Exported ${pluralize(outcome.rows, 'row', 'rows')} to ${outcome.fileName}`)
      }
    } catch (error) {
      if (error instanceof NativeError && error.code === 'cancelled') {
        toast('Export cancelled', { description: 'Nothing was saved.' })
      } else {
        toast.error('Could not export', {
          description: error instanceof Error ? error.message : String(error),
        })
      }
    } finally {
      setRunning(null)
    }
  }

  if (running) {
    return (
      <span className="flex items-center gap-2 text-sm text-muted-foreground" role="status">
        <Spinner />
        Exporting…
        <Button
          variant="link"
          size="sm"
          className="h-auto px-0"
          onClick={() => void cancelOperation(connectionId, running).catch(() => undefined)}
        >
          Cancel
        </Button>
      </span>
    )
  }

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm" className="max-sm:min-h-11">
          <DownloadIcon data-icon="inline-start" />
          Export
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-72">
        <DropdownMenuLabel className="font-normal text-muted-foreground">
          Every row that matches the filters, in this order, up to {maxRows} rows. Values are
          complete.
        </DropdownMenuLabel>
        <DropdownMenuItem onSelect={() => void run('csv')}>CSV file…</DropdownMenuItem>
        <DropdownMenuItem onSelect={() => void run('json')}>JSON file…</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
