import {
  GitCompareArrowsIcon,
  SettingsIcon,
  SparklesIcon,
  SquareTerminalIcon,
  Table2Icon,
  type LucideIcon,
} from 'lucide-react'

export type NavItem = {
  to: string
  label: string
  icon: LucideIcon
}

/** Main destinations. These four also form the compact bottom navigation. */
export const PRIMARY_NAV: readonly NavItem[] = [
  { to: '/ask', label: 'Ask', icon: SparklesIcon },
  { to: '/data', label: 'Data', icon: Table2Icon },
  { to: '/sql', label: 'SQL', icon: SquareTerminalIcon },
  { to: '/changes', label: 'Changes', icon: GitCompareArrowsIcon },
]

export const SETTINGS_NAV: NavItem = { to: '/settings', label: 'Settings', icon: SettingsIcon }

export function isActivePath(pathname: string, to: string): boolean {
  return pathname === to || pathname.startsWith(`${to}/`)
}

/** Title of the top-level destination that contains `pathname`. */
export function destinationTitle(pathname: string): string {
  const match = [...PRIMARY_NAV, SETTINGS_NAV].find((item) => isActivePath(pathname, item.to))
  return match?.label ?? 'aidb'
}
