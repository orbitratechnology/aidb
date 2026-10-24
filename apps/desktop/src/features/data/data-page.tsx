import { Table2Icon } from 'lucide-react'
import { PageContainer } from '@/components/shared/page'
import { PlannedFeature, RequireConnection } from '@/components/shared/status-states'

export function DataPage() {
  return (
    <PageContainer>
      <RequireConnection
        icon={Table2Icon}
        title="Browse your database"
        description="Connect to a PostgreSQL database to explore its schemas, tables and records."
      >
        <PlannedFeature
          icon={Table2Icon}
          title="Schema browser"
          description="Schemas, tables, relationships and paginated records will appear here."
        />
      </RequireConnection>
    </PageContainer>
  )
}
