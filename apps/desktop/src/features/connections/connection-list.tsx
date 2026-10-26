import { Fragment } from 'react'
import {
  KeyRoundIcon,
  KeySquareIcon,
  LockIcon,
  PencilIcon,
  ShieldOffIcon,
  TimerIcon,
  type LucideIcon,
} from 'lucide-react'
import type { ConnectionDetails, PasswordStorage } from '@aidb/contracts'
import { EnvironmentBadge } from '@/components/shared/environment-badge'
import { Badge } from '@/components/ui/badge'
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemGroup,
  ItemSeparator,
  ItemTitle,
} from '@/components/ui/item'
import { connectionAddress } from './format'

const PASSWORD_DISPLAY: Record<PasswordStorage, { label: string; icon: LucideIcon }> = {
  os_store: { label: 'Password saved', icon: KeyRoundIcon },
  session_only: { label: 'Password for this session', icon: TimerIcon },
  not_saved: { label: 'No password saved', icon: KeySquareIcon },
}

export function ConnectionList({ connections }: { connections: readonly ConnectionDetails[] }) {
  return (
    <ItemGroup className="gap-0 rounded-lg border">
      {connections.map((connection, index) => (
        <Fragment key={connection.id}>
          {index > 0 && <ItemSeparator className="my-0" />}
          <ConnectionRow connection={connection} />
        </Fragment>
      ))}
    </ItemGroup>
  )
}

function ConnectionRow({ connection }: { connection: ConnectionDetails }) {
  const password = PASSWORD_DISPLAY[connection.password]
  return (
    <Item role="listitem">
      <ItemContent className="min-w-0">
        <ItemTitle className="flex-wrap">
          <span className="truncate">{connection.name}</span>
          <EnvironmentBadge environment={connection.environment} />
        </ItemTitle>
        <ItemDescription className="font-mono text-xs break-all">
          {connectionAddress(connection)}
        </ItemDescription>
      </ItemContent>
      <ItemActions className="w-full flex-wrap sm:w-auto">
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
      </ItemActions>
    </Item>
  )
}
