import type { ReactNode } from 'react'
import { PlugIcon, type LucideIcon } from 'lucide-react'
import { Link } from 'react-router'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { useConnectionStore } from '@/stores/connection-store'

type StateProps = {
  icon: LucideIcon
  title: string
  description: string
  children?: ReactNode
}

export function EmptyState({ icon: Icon, title, description, children }: StateProps) {
  return (
    <Empty>
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <Icon />
        </EmptyMedia>
        <EmptyTitle>{title}</EmptyTitle>
        <EmptyDescription>{description}</EmptyDescription>
      </EmptyHeader>
      {children && <EmptyContent>{children}</EmptyContent>}
    </Empty>
  )
}

/** Shown for features that are designed but not built in this version of the app. */
export function PlannedFeature(props: Omit<StateProps, 'children'>) {
  return (
    <EmptyState {...props}>
      <Badge variant="secondary">Not available in this version yet</Badge>
    </EmptyState>
  )
}

/** Renders `children` only while a database connection is active. */
export function RequireConnection({
  children,
  ...empty
}: Omit<StateProps, 'children'> & { children: ReactNode }) {
  const active = useConnectionStore((state) => state.active)

  if (active) {
    return children
  }

  return (
    <EmptyState {...empty}>
      <Button asChild className="min-h-11 sm:min-h-0">
        <Link to="/settings/connections">
          <PlugIcon data-icon="inline-start" />
          Set up a connection
        </Link>
      </Button>
    </EmptyState>
  )
}
