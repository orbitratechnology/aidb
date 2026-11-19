import { GitCompareArrowsIcon } from 'lucide-react'
import { PageContainer } from '@/components/shared/page'
import { EmptyState } from '@/components/shared/status-states'

export function ChangesPage() {
  return (
    <PageContainer>
      <EmptyState
        icon={GitCompareArrowsIcon}
        title="Nothing to review"
        description="Data edits, imports and migrations wait here for your review before anything is written. Applied operations are kept in the history."
      />
    </PageContainer>
  )
}
