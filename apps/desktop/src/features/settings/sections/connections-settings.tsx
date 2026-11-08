import { useState } from 'react'
import { useMutation } from '@tanstack/react-query'
import { DatabaseIcon, DownloadIcon, PlusIcon, UploadIcon } from 'lucide-react'
import { toast } from 'sonner'
import type { ConnectionDetails, ImportPreview } from '@aidb/contracts'
import { EmptyState } from '@/components/shared/status-states'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { useConnectFlow } from '@/features/connections/connect-flow'
import { ConnectionSheet, type FormMode } from '@/features/connections/connection-form'
import { ConnectionList } from '@/features/connections/connection-list'
import { ForgetConnectionDialog } from '@/features/connections/forget-dialog'
import { pluralize } from '@/features/connections/format'
import { ImportConnectionsDialog } from '@/features/connections/import-dialog'
import { useConnections } from '@/features/connections/queries'
import { useActiveSessions, useDisconnect } from '@/features/connections/sessions'
import { exportConnections, previewConnectionImport } from '@/lib/commands'
import { LoadError } from '../load-error'
import { SettingsSection } from '../settings-section'

export function ConnectionsSettings() {
  const connections = useConnections()
  const sessions = useActiveSessions()
  const [preview, setPreview] = useState<ImportPreview | null>(null)
  const [formMode, setFormMode] = useState<FormMode | null>(null)
  const [forgetting, setForgetting] = useState<ConnectionDetails | null>(null)
  const { connectTo, connectingId, dialog: passwordDialog } = useConnectFlow()
  const disconnect = useDisconnect()
  const activeId = sessions.data?.[0]?.connectionId ?? null

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
  const addButton = (
    <Button className="min-h-11 sm:min-h-0" onClick={() => setFormMode({ kind: 'create' })}>
      <PlusIcon data-icon="inline-start" />
      Add connection
    </Button>
  )
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
          description="Add a PostgreSQL connection, or import profiles exported from aidb on another computer."
        >
          <div className="flex flex-wrap justify-center gap-2">
            {addButton}
            {importButton}
          </div>
        </EmptyState>
      )}
      {connections.isSuccess && count > 0 && (
        <>
          <div className="flex flex-wrap gap-2">
            {addButton}
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
          <ConnectionList
            connections={connections.data}
            activeId={activeId}
            busyId={connectingId ?? (disconnect.isPending ? disconnect.variables : undefined)}
            onConnect={connectTo}
            onDisconnect={(connection) =>
              disconnect.mutate(connection.id, {
                onSuccess: () => toast.success(`Disconnected from ${connection.name}`),
                onError: (error) =>
                  toast.error(`Couldn't disconnect`, { description: error.message }),
              })
            }
            onEdit={(connection) => setFormMode({ kind: 'edit', connection })}
            onForget={setForgetting}
          />
        </>
      )}
      <ImportConnectionsDialog preview={preview} onClose={() => setPreview(null)} />
      <ConnectionSheet mode={formMode} onClose={() => setFormMode(null)} />
      <ForgetConnectionDialog connection={forgetting} onClose={() => setForgetting(null)} />
      {passwordDialog}
    </SettingsSection>
  )
}
