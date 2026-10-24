import { HardDriveIcon } from 'lucide-react'
import { PlannedFeature } from '@/components/shared/status-states'
import { SettingsSection } from '../settings-section'

export function StorageSettings() {
  return (
    <SettingsSection
      title="Storage"
      description="History, cached schema details and audit records stay on this device."
    >
      <PlannedFeature
        icon={HardDriveIcon}
        title="Local data controls"
        description="Controls to review and clear local history and caches will appear here."
      />
    </SettingsSection>
  )
}
