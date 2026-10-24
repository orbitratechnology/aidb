import { ShieldCheckIcon } from 'lucide-react'
import { PlannedFeature } from '@/components/shared/status-states'
import { SettingsSection } from '../settings-section'

export function SafetySettings() {
  return (
    <SettingsSection
      title="Safety"
      description="Defaults that apply to new connections and to every write."
    >
      <PlannedFeature
        icon={ShieldCheckIcon}
        title="Read-only by default"
        description="Row limits, timeouts and approval rules will be configurable here. New connections start read-only."
      />
    </SettingsSection>
  )
}
