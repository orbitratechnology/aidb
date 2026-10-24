import { Badge } from '@/components/ui/badge'
import { ENVIRONMENT_DISPLAY } from '@/features/connections/environments'
import type { EnvironmentKind } from '@/features/connections/types'

export function EnvironmentBadge({ environment }: { environment: EnvironmentKind }) {
  const { label, icon: Icon, variant } = ENVIRONMENT_DISPLAY[environment]
  return (
    <Badge variant={variant}>
      <Icon data-icon="inline-start" />
      <span className="sr-only">Environment: </span>
      {label}
    </Badge>
  )
}
