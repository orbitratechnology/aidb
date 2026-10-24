import { DatabaseIcon, LockIcon, UnplugIcon } from 'lucide-react'
import { Link } from 'react-router'
import { EnvironmentBadge } from '@/components/shared/environment-badge'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { ENVIRONMENT_DISPLAY } from '@/features/connections/environments'
import { useConnectionStore } from '@/stores/connection-store'

const CONNECTIONS_PATH = '/settings/connections'

/** Top-bar identity of the active connection: name, database, environment and access mode. */
export function ConnectionStatus() {
  const active = useConnectionStore((state) => state.active)

  if (!active) {
    return (
      <Button variant="outline" size="sm" asChild className="min-h-11 sm:min-h-0">
        <Link to={CONNECTIONS_PATH}>
          <UnplugIcon data-icon="inline-start" />
          No connection
        </Link>
      </Button>
    )
  }

  const description = [
    `Connected to ${active.name}`,
    `database ${active.database}`,
    `${ENVIRONMENT_DISPLAY[active.environment].label} environment`,
    active.readOnly ? 'read-only' : 'writes enabled',
  ].join(', ')

  return (
    <Button variant="ghost" asChild className="h-auto min-h-11 min-w-0 py-1 sm:min-h-9">
      <Link to={CONNECTIONS_PATH} aria-label={description} title={description}>
        <DatabaseIcon data-icon="inline-start" className="hidden sm:block" />
        <span className="flex min-w-0 flex-col items-start leading-tight">
          <span className="max-w-32 truncate sm:max-w-48">{active.name}</span>
          <span className="max-w-32 truncate font-mono text-xs font-normal text-muted-foreground sm:max-w-48">
            {active.database}
          </span>
        </span>
        <EnvironmentBadge environment={active.environment} />
        {active.readOnly && (
          <Badge variant="outline" className="hidden sm:inline-flex">
            <LockIcon data-icon="inline-start" />
            Read-only
          </Badge>
        )}
      </Link>
    </Button>
  )
}
