import { BotIcon } from 'lucide-react'
import { PlannedFeature } from '@/components/shared/status-states'
import { SettingsSection } from '../settings-section'

export function AiSettings() {
  return (
    <SettingsSection
      title="AI providers"
      description="The agent is optional. Manual SQL and data tools work without it."
    >
      <PlannedFeature
        icon={BotIcon}
        title="AI is off"
        description="You will be able to add a hosted provider or a local Ollama endpoint here. Database credentials are never sent to a model."
      />
    </SettingsSection>
  )
}
