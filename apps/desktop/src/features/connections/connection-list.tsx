import { Fragment } from 'react'
import {
  EllipsisVerticalIcon,
  KeyRoundIcon,
  KeySquareIcon,
  LockIcon,
  PencilIcon,
  PlugIcon,
  PlugZapIcon,
  ShieldOffIcon,
  TimerIcon,
  Trash2Icon,
  UnplugIcon,
  type LucideIcon,
} from 'lucide-react'
import type { ConnectionDetails, PasswordStorage } from '@aidb/contracts'
import { EnvironmentBadge } from '@/components/shared/environment-badge'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemSeparator,
  ItemTitle,
} from '@/components/ui/item'
import { Spinner } from '@/components/ui/spinner'
import { connectionAddress } from './format'

const PASSWORD_DISPLAY: Record<PasswordStorage, { label: string; icon: LucideIcon }> = {
  os_store: { label: 'Password saved', icon: KeyRoundIcon },
  session_only: { label: 'Password for this session', icon: TimerIcon },
  not_saved: { label: 'No password saved', icon: KeySquareIcon },
}

export type ConnectionActions = {
  /** The connection that is open, if any. */
  activeId: string | null
  /** The connection being connected to or disconnected from, if any. */
  busyId: string | undefined
  onConnect: (connection: ConnectionDetails) => void
  onDisconnect: (connection: ConnectionDetails) => void
  onEdit: (connection: ConnectionDetails) => void
  onForget: (connection: ConnectionDetails) => void
}

export function ConnectionList({
  connections,
  ...actions
}: { connections: readonly ConnectionDetails[] } & ConnectionActions) {
  return (
    <ItemGroup className="gap-0 rounded-lg border">
      {connections.map((connection, index) => (
        <Fragment key={connection.id}>
          {index > 0 && <ItemSeparator className="my-0" />}
          <ConnectionRow connection={connection} {...actions} />
        </Fragment>
      ))}
    </ItemGroup>
  )
}

function ConnectionRow({
  connection,
  activeId,
  busyId,
  onConnect,
  onDisconnect,
  onEdit,
  onForget,
}: { connection: ConnectionDetails } & ConnectionActions) {
  const password = PASSWORD_DISPLAY[connection.password]
  const connected = activeId === connection.id
  const busy = busyId === connection.id
  return (
    <Item role="listitem" data-connected={connected} className="data-[connected=true]:bg-muted/50">
      <ItemContent className="min-w-0">
        <ItemTitle className="flex-wrap">
          <span className="truncate">{connection.name}</span>
          <EnvironmentBadge environment={connection.environment} />
          {connected && (
            <Badge>
              <PlugZapIcon data-icon="inline-start" />
              Connected
            </Badge>
          )}
        </ItemTitle>
        <ItemDescription className="font-mono text-xs break-all">
          {connectionAddress(connection)}
        </ItemDescription>
        <div className="flex flex-wrap gap-1.5 pt-1">
          {connection.readOnly ? (
            <Badge variant="outline">
              <LockIcon data-icon="inline-start" />
              Read-only
            </Badge>
          ) : (
            <Badge variant="secondary">
              <PencilIcon data-icon="inline-start" />
              Writes enabled
            </Badge>
          )}
          {connection.unencryptedRemote && (
            <Badge variant="destructive">
              <ShieldOffIcon data-icon="inline-start" />
              TLS off
            </Badge>
          )}
          <Badge variant="outline">
            <password.icon data-icon="inline-start" />
            {password.label}
          </Badge>
        </div>
      </ItemContent>
      <ItemActions className="w-full justify-end sm:w-auto">
        {connected ? (
          <Button
            variant="outline"
            className="min-h-11 sm:min-h-0"
            disabled={busy}
            onClick={() => onDisconnect(connection)}
          >
            {busy ? <Spinner data-icon="inline-start" /> : <UnplugIcon data-icon="inline-start" />}
            Disconnect
          </Button>
        ) : (
          <Button
            className="min-h-11 sm:min-h-0"
            disabled={busy}
            onClick={() => onConnect(connection)}
          >
            {busy ? <Spinner data-icon="inline-start" /> : <PlugIcon data-icon="inline-start" />}
            Connect
          </Button>
        )}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className="size-11 sm:size-9"
              aria-label={`More actions for ${connection.name}`}
            >
              <EllipsisVerticalIcon />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="min-w-48">
            <DropdownMenuItem onSelect={() => onEdit(connection)}>
              <PencilIcon />
              Edit or test
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem variant="destructive" onSelect={() => onForget(connection)}>
              <Trash2Icon />
              Forget connection…
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </ItemActions>
    </Item>
  )
}
