import {
  CircleDashedIcon,
  FlaskConicalIcon,
  LaptopIcon,
  ShieldAlertIcon,
  WrenchIcon,
  type LucideIcon,
} from 'lucide-react'
import type { EnvironmentKind } from './types'

export type EnvironmentDisplay = {
  label: string
  icon: LucideIcon
  variant: 'outline' | 'secondary' | 'destructive'
}

// Each environment has a distinct label and icon, so it never depends on color alone.
export const ENVIRONMENT_DISPLAY: Record<EnvironmentKind, EnvironmentDisplay> = {
  local: { label: 'Local', icon: LaptopIcon, variant: 'outline' },
  development: { label: 'Development', icon: WrenchIcon, variant: 'outline' },
  staging: { label: 'Staging', icon: FlaskConicalIcon, variant: 'secondary' },
  production: { label: 'Production', icon: ShieldAlertIcon, variant: 'destructive' },
  other: { label: 'Other', icon: CircleDashedIcon, variant: 'outline' },
}
