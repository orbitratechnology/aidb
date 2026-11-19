import { SparklesIcon } from 'lucide-react'
import { PageContainer } from '@/components/shared/page'
import { PlannedFeature, RequireConnection } from '@/components/shared/status-states'

export function AskPage() {
  return (
    <PageContainer>
      <RequireConnection
        icon={SparklesIcon}
        title="Ask about your data"
        description="Connect to a PostgreSQL database, then describe what you need in plain language. AI is optional and stays off until you set up a provider."
      >
        <PlannedFeature
          icon={SparklesIcon}
          title="Agent workspace"
          description="Plans, previews and results for natural-language tasks will appear here."
        />
      </RequireConnection>
    </PageContainer>
  )
}
