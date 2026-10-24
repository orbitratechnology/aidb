import { DatabaseIcon } from 'lucide-react'
import { PlannedFeature } from '@/components/shared/status-states'
import { SettingsSection } from '../settings-section'

export function ConnectionsSettings() {
  return (
    <SettingsSection
      title="Connections"
      description="PostgreSQL connection profiles. Passwords are kept in your operating system's credential store, never in app files."
    >
      <PlannedFeature
        icon={DatabaseIcon}
        title="No saved connections"
        description="Saved connections will be listed here, each with its environment and read-only setting."
      />
    </SettingsSection>
  )
}
