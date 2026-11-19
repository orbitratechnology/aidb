import { Fragment } from 'react'
import type { ProviderDetails } from '@aidb/contracts'
import {
  CheckIcon,
  EllipsisVerticalIcon,
  FlaskConicalIcon,
  HouseIcon,
  PencilIcon,
  ShieldOffIcon,
  Trash2Icon,
  WrenchIcon,
  type LucideIcon,
} from 'lucide-react'
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
import { PRESETS } from './provider-form'
import {
  isLocalProvider,
  KEY_DISPLAY,
  providerBlocker,
  STRUCTURED_OUTPUT_LABELS,
} from './provider-display'

export type ProviderActions = {
  /** The provider the assistant uses, if any. */
  chosenId: string | null
  onTest: (provider: ProviderDetails) => void
  onEdit: (provider: ProviderDetails) => void
  onDelete: (provider: ProviderDetails) => void
}

export function ProviderList({
  providers,
  ...actions
}: { providers: readonly ProviderDetails[] } & ProviderActions) {
  return (
    <ItemGroup className="gap-0 rounded-lg border">
      {providers.map((provider, index) => (
        <Fragment key={provider.id}>
          {index > 0 && <ItemSeparator className="my-0" />}
          <ProviderRow provider={provider} {...actions} />
        </Fragment>
      ))}
    </ItemGroup>
  )
}

function Feature({ icon: Icon, children }: { icon: LucideIcon; children: React.ReactNode }) {
  return (
    <Badge variant="outline">
      <Icon data-icon="inline-start" />
      {children}
    </Badge>
  )
}

function ProviderRow({
  provider,
  chosenId,
  onTest,
  onEdit,
  onDelete,
}: { provider: ProviderDetails } & ProviderActions) {
  const key = KEY_DISPLAY[provider.key]
  const blocker = providerBlocker(provider)
  const chosen = chosenId === provider.id
  return (
    <Item role="listitem" data-chosen={chosen} className="data-[chosen=true]:bg-muted/50">
      <ItemContent className="min-w-0">
        <ItemTitle className="flex-wrap">
          <span className="truncate">{provider.name}</span>
          <Badge variant="secondary">{PRESETS[provider.kind].label}</Badge>
          {chosen && (
            <Badge>
              <CheckIcon data-icon="inline-start" />
              Used by the assistant
            </Badge>
          )}
        </ItemTitle>
        <ItemDescription className="font-mono text-xs break-all">
          {provider.model} · {provider.endpointUrl}
        </ItemDescription>
        <div className="flex flex-wrap gap-1.5 pt-1">
          {isLocalProvider(provider) && <Feature icon={HouseIcon}>On this computer</Feature>}
          {provider.insecureRemote && (
            <Badge variant="destructive">
              <ShieldOffIcon data-icon="inline-start" />
              Plain HTTP
            </Badge>
          )}
          {(provider.keyRequired || provider.key !== 'not_saved') && (
            <Feature icon={key.icon}>{key.label}</Feature>
          )}
          {provider.checkedAt ? (
            <>
              <Feature icon={WrenchIcon}>
                {provider.features.toolCalls ? 'Tool calls' : 'No tool calls'}
              </Feature>
              <Feature icon={CheckIcon}>
                {STRUCTURED_OUTPUT_LABELS[provider.features.structuredOutput]}
              </Feature>
            </>
          ) : (
            <Feature icon={FlaskConicalIcon}>Not tested yet</Feature>
          )}
        </div>
        {blocker && <p className="pt-1 text-sm text-muted-foreground">{blocker}</p>}
      </ItemContent>
      <ItemActions className="w-full justify-end sm:w-auto">
        <Button
          variant="outline"
          className="min-h-11 sm:min-h-0"
          disabled={blocker !== null}
          onClick={() => onTest(provider)}
        >
          <FlaskConicalIcon data-icon="inline-start" />
          Test
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              className="size-11 sm:size-9"
              aria-label={`More actions for ${provider.name}`}
            >
              <EllipsisVerticalIcon />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="min-w-48">
            <DropdownMenuItem onSelect={() => onEdit(provider)}>
              <PencilIcon />
              Edit
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem variant="destructive" onSelect={() => onDelete(provider)}>
              <Trash2Icon />
              Delete provider…
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </ItemActions>
    </Item>
  )
}
