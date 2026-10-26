import { useState } from 'react'
import { useMutation } from '@tanstack/react-query'
import { DatabaseIcon, DownloadIcon, UploadIcon } from 'lucide-react'
import { toast } from 'sonner'
import type { ImportPreview } from '@aidb/contracts'
import { EmptyState } from '@/components/shared/status-states'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { ConnectionList } from '@/features/connections/connection-list'
import { pluralize } from '@/features/connections/format'
import { ImportConnectionsDialog } from '@/features/connections/import-dialog'
import { useConnections } from '@/features/connections/queries'
import { exportConnections, previewConnectionImport } from '@/lib/commands'
import { LoadError } from '../load-error'
import { SettingsSection } from '../settings-section'

export function ConnectionsSettings() {
  const connections = useConnections()
  const [preview, setPreview] = useState<ImportPreview | null>(null)

  const importFile = useMutation({
    mutationFn: previewConnectionImport,
    onSuccess: (result) => setPreview(result),
    onError: (error) => toast.error("Couldn't read the file", { description: error.message }),
  })
  const exportFile = useMutation({
    mutationFn: exportConnections,
    onSuccess: (result) => {
      if (result) {
        toast.success(
          `Exported ${pluralize(result.count, 'connection', 'connections')} to ${result.fileName}`,
        )
      }
    },
    onError: (error) => toast.error("Couldn't export connections", { description: error.message }),
  })

  const count = connections.data?.length ?? 0
  const importButton = (
    <Button
      variant="outline"
      className="min-h-11 sm:min-h-0"
      disabled={importFile.isPending}
      onClick={() => importFile.mutate()}
    >
      {importFile.isPending ? (
        <Spinner data-icon="inline-start" />
      ) : (
        <UploadIcon data-icon="inline-start" />
      )}
      Import…
    </Button>
  )

  return (
    <SettingsSection
      title="Connections"
      description="PostgreSQL connection profiles. Passwords are kept in your operating system's credential store, never in app files or exports."
    >
      {connections.isPending && <Skeleton className="h-48 w-full" />}
      {connections.isError && (
        <LoadError
          title="Connections could not be loaded"
          error={connections.error}
          retrying={connections.isFetching}
          onRetry={() => void connections.refetch()}
        />
      )}
      {connections.isSuccess && count === 0 && (
        <EmptyState
          icon={DatabaseIcon}
          title="No saved connections"
          description="Import connection profiles exported from aidb on another computer."
        >
          {importButton}
        </EmptyState>
      )}
      {connections.isSuccess && count > 0 && (
        <>
          <div className="flex flex-wrap gap-2">
            {importButton}
            <Button
              variant="outline"
              className="min-h-11 sm:min-h-0"
              disabled={exportFile.isPending}
              onClick={() => exportFile.mutate()}
            >
              {exportFile.isPending ? (
                <Spinner data-icon="inline-start" />
              ) : (
                <DownloadIcon data-icon="inline-start" />
              )}
              Export…
            </Button>
          </div>
          <ConnectionList connections={connections.data} />
        </>
      )}
      <ImportConnectionsDialog preview={preview} onClose={() => setPreview(null)} />
    </SettingsSection>
  )
}
