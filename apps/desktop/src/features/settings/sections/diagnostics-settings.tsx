import { useEffect, useState, type ReactNode } from 'react'
import { CircleAlertIcon, CircleCheckIcon, ExternalLinkIcon } from 'lucide-react'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Item, ItemContent, ItemDescription, ItemGroup, ItemTitle } from '@/components/ui/item'
import { Skeleton } from '@/components/ui/skeleton'
import type { AppInfo, DiagnosticsInfo, LocalStoreStatus } from '@aidb/contracts'
import { getAppInfo, getDiagnostics } from '@/lib/commands'
import { SettingsSection } from '../settings-section'

const ISSUES_URL = 'https://github.com/orbitratechnology/aidb/issues'

type Report = { app: AppInfo; diagnostics: DiagnosticsInfo }

type LoadState =
  { state: 'loading' } | { state: 'ready'; report: Report } | { state: 'error'; message: string }

export function DiagnosticsSettings() {
  const [status, setStatus] = useState<LoadState>({ state: 'loading' })

  useEffect(() => {
    let cancelled = false
    Promise.all([getAppInfo(), getDiagnostics()]).then(
      ([app, diagnostics]) => {
        if (!cancelled) setStatus({ state: 'ready', report: { app, diagnostics } })
      },
      (err: unknown) => {
        if (!cancelled) {
          setStatus({ state: 'error', message: err instanceof Error ? err.message : String(err) })
        }
      },
    )
    return () => {
      cancelled = true
    }
  }, [])

  return (
    <SettingsSection title="Diagnostics" description="Details to include when reporting a problem.">
      {status.state === 'loading' && <Skeleton className="h-64 w-full" />}
      {status.state === 'error' && (
        <Alert variant="destructive">
          <CircleAlertIcon />
          <AlertTitle>Native core unavailable</AlertTitle>
          <AlertDescription>{status.message}</AlertDescription>
        </Alert>
      )}
      {status.state === 'ready' && <ReportDetails report={status.report} />}
      <Button variant="outline" asChild className="min-h-11 self-start sm:min-h-0">
        <a href={ISSUES_URL}>
          Report a problem
          <ExternalLinkIcon data-icon="inline-end" />
        </a>
      </Button>
    </SettingsSection>
  )
}

function ReportDetails({ report: { app, diagnostics } }: { report: Report }) {
  const store = diagnostics.localStore
  return (
    <>
      {store.state === 'unavailable' && (
        <Alert variant="destructive">
          <CircleAlertIcon />
          <AlertTitle>Local database unavailable</AlertTitle>
          <AlertDescription>
            {store.message ?? 'Saved connections and history cannot be used until this is fixed.'}
          </AlertDescription>
        </Alert>
      )}
      <ItemGroup className="rounded-lg border">
        <DetailRow label="Application" value={app.name} />
        <DetailRow label="Version" value={app.version} />
        <DetailRow label="Platform" value={app.platform} />
        <DetailRow label="Local database">
          <StoreBadge store={store} />
        </DetailRow>
        <DetailRow label="Data folder" value={diagnostics.dataDir} />
        <DetailRow label="Log folder" value={diagnostics.logDir} />
      </ItemGroup>
    </>
  )
}

function StoreBadge({ store }: { store: LocalStoreStatus }) {
  if (store.state === 'ready') {
    return (
      <Badge variant="outline">
        <CircleCheckIcon data-icon="inline-start" />
        Ready · schema {store.schemaVersion ?? '?'}
      </Badge>
    )
  }
  return (
    <Badge variant="destructive">
      <CircleAlertIcon data-icon="inline-start" />
      Unavailable
    </Badge>
  )
}

function DetailRow({
  label,
  value,
  children,
}: {
  label: string
  value?: string
  children?: ReactNode
}) {
  return (
    <Item size="sm">
      <ItemContent>
        <ItemTitle>{label}</ItemTitle>
      </ItemContent>
      {children ?? (
        <ItemDescription className="min-w-0 font-mono break-all">{value}</ItemDescription>
      )}
    </Item>
  )
}
