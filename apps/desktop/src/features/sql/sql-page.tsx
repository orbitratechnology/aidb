import { SquareTerminalIcon } from 'lucide-react'
import { PageContainer } from '@/components/shared/page'
import { PlannedFeature, RequireConnection } from '@/components/shared/status-states'

export function SqlPage() {
  return (
    <PageContainer>
      <RequireConnection
        icon={SquareTerminalIcon}
        title="Write and run SQL"
        description="Connect to a PostgreSQL database to run queries. New connections are read-only until you enable writes."
      >
        <PlannedFeature
          icon={SquareTerminalIcon}
          title="SQL editor"
          description="The editor, results and query plans will appear here."
        />
      </RequireConnection>
    </PageContainer>
  )
}
