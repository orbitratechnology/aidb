import { useEffect, useState } from 'react'
import { CircleAlertIcon, ExternalLinkIcon } from 'lucide-react'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Item, ItemContent, ItemDescription, ItemGroup, ItemTitle } from '@/components/ui/item'
import { Skeleton } from '@/components/ui/skeleton'
import type { AppInfo } from '@aidb/contracts'
import { getAppInfo } from '@/lib/commands'
import { SettingsSection } from '../settings-section'

const ISSUES_URL = 'https://github.com/orbitratechnology/aidb/issues'

type NativeStatus =
  { state: 'loading' } | { state: 'ready'; info: AppInfo } | { state: 'error'; message: string }

export function DiagnosticsSettings() {
  const [status, setStatus] = useState<NativeStatus>({ state: 'loading' })

  useEffect(() => {
    let cancelled = false
    getAppInfo().then(
      (info) => {
        if (!cancelled) setStatus({ state: 'ready', info })
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
      {status.state === 'loading' && <Skeleton className="h-32 w-full" />}
      {status.state === 'error' && (
        <Alert variant="destructive">
          <CircleAlertIcon />
          <AlertTitle>Native core unavailable</AlertTitle>
          <AlertDescription>{status.message}</AlertDescription>
        </Alert>
      )}
      {status.state === 'ready' && (
        <ItemGroup className="rounded-lg border">
          <DetailRow label="Application" value={status.info.name} />
          <DetailRow label="Version" value={status.info.version} />
          <DetailRow label="Platform" value={status.info.platform} />
        </ItemGroup>
      )}
      <Button variant="outline" asChild className="min-h-11 self-start sm:min-h-0">
        <a href={ISSUES_URL}>
          Report a problem
          <ExternalLinkIcon data-icon="inline-end" />
        </a>
      </Button>
    </SettingsSection>
  )
}

function DetailRow({ label, value }: { label: string; value: string }) {
  return (
    <Item size="sm">
      <ItemContent>
        <ItemTitle>{label}</ItemTitle>
      </ItemContent>
      <ItemDescription className="font-mono">{value}</ItemDescription>
    </Item>
  )
}
