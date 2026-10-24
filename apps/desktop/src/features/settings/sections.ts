import type { ComponentType } from 'react'
import {
  ActivityIcon,
  BotIcon,
  DatabaseIcon,
  HardDriveIcon,
  PaletteIcon,
  ShieldCheckIcon,
  type LucideIcon,
} from 'lucide-react'
import { AiSettings } from './sections/ai-settings'
import { AppearanceSettings } from './sections/appearance-settings'
import { ConnectionsSettings } from './sections/connections-settings'
import { DiagnosticsSettings } from './sections/diagnostics-settings'
import { SafetySettings } from './sections/safety-settings'
import { StorageSettings } from './sections/storage-settings'

export type SettingsSectionDef = {
  id: string
  title: string
  summary: string
  icon: LucideIcon
  component: ComponentType
}

export const SETTINGS_SECTIONS: readonly SettingsSectionDef[] = [
  {
    id: 'connections',
    title: 'Connections',
    summary: 'PostgreSQL profiles and credentials',
    icon: DatabaseIcon,
    component: ConnectionsSettings,
  },
  {
    id: 'ai',
    title: 'AI providers',
    summary: 'Optional hosted or local models',
    icon: BotIcon,
    component: AiSettings,
  },
  {
    id: 'safety',
    title: 'Safety',
    summary: 'Read-only defaults, limits and approvals',
    icon: ShieldCheckIcon,
    component: SafetySettings,
  },
  {
    id: 'appearance',
    title: 'Appearance',
    summary: 'Theme',
    icon: PaletteIcon,
    component: AppearanceSettings,
  },
  {
    id: 'storage',
    title: 'Storage',
    summary: 'Local history, caches and deletion',
    icon: HardDriveIcon,
    component: StorageSettings,
  },
  {
    id: 'diagnostics',
    title: 'Diagnostics',
    summary: 'Version and platform details',
    icon: ActivityIcon,
    component: DiagnosticsSettings,
  },
]

export function findSettingsSection(id: string | undefined): SettingsSectionDef | undefined {
  return SETTINGS_SECTIONS.find((section) => section.id === id)
}
