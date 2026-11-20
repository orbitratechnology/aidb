import { useState } from 'react'
import type { DataChangeResult, PendingChange } from '@aidb/contracts'
import { FilePenLineIcon } from 'lucide-react'
import { toast } from 'sonner'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Item,
  ItemActions,
  ItemContent,
  ItemFooter,
  ItemMedia,
  ItemTitle,
} from '@/components/ui/item'
import { useConnections } from '@/features/connections/queries'
import { proposalItems } from '@/features/data/edits'
import { ReviewChangesDialog } from '@/features/data/review-changes-dialog'
import { usePendingChanges, useRefreshAfterDecision } from './pending'

const KIND_LABELS = { update: 'Update', delete: 'Delete', insert: 'Insert' } as const

type Decision = { status: 'applied'; rows: number } | { status: 'declined' }

/**
 * Changes waiting for the user's decision, with what each does and how many rows it writes, and
 * a review that applies or declines them. Shown where the assistant proposed them and on the
 * Changes page.
 */
export function PendingChangeCard({ change }: { change: PendingChange }) {
  const [reviewing, setReviewing] = useState(false)
  const [decision, setDecision] = useState<Decision | null>(null)
  const pending = usePendingChanges(change.connectionId)
  const connections = useConnections()
  const refresh = useRefreshAfterDecision()
  const profile = connections.data?.find((c) => c.id === change.connectionId)
  const items = proposalItems(change.changes)
  const approvalId = change.preview.approvalId
  const waiting =
    decision === null &&
    (pending.data === undefined || pending.data.some((c) => c.preview.approvalId === approvalId))

  const applied = (result: DataChangeResult) => {
    const rows = result.rowsAffected ?? 0
    setDecision({ status: 'applied', rows })
    setReviewing(false)
    toast.success(`Applied. ${rows === 1 ? '1 row' : `${rows} rows`} written.`)
    refresh(change)
  }

  return (
    <Item variant="outline" className="items-start">
      <ItemMedia variant="icon">
        <FilePenLineIcon />
      </ItemMedia>
      <ItemContent className="min-w-0">
        <ItemTitle className="flex flex-wrap items-center gap-2">
          {change.actor === 'agent' ? 'Change proposed by the assistant' : 'Your edits'}
          <Badge variant={change.preview.risk >= 'R3' ? 'destructive' : 'secondary'}>
            Risk {change.preview.risk}
          </Badge>
          <StatusBadge waiting={waiting} decision={decision} />
        </ItemTitle>
        <ul className="flex flex-col gap-0.5 text-sm text-muted-foreground">
          {items.map((item, index) => {
            const rows = change.preview.changes[index]?.rows ?? 0
            return (
              <li key={item.id} className="break-words">
                {KIND_LABELS[item.kind]} {item.label}: {rows === 1 ? '1 row' : `${rows} rows`}
              </li>
            )
          })}
        </ul>
        <ItemFooter className="text-sm text-muted-foreground">
          {decision?.status === 'applied'
            ? `Applied: ${decision.rows === 1 ? '1 row' : `${decision.rows} rows`} written.`
            : decision?.status === 'declined'
              ? 'Declined. Nothing was written.'
              : waiting
                ? 'Waiting for your review. Nothing is written until you apply it.'
                : 'No longer waiting: it was applied or declined, it expired, or its connection changed.'}
        </ItemFooter>
      </ItemContent>
      {waiting && (
        <ItemActions>
          <Button className="max-sm:min-h-11" onClick={() => setReviewing(true)}>
            Review
          </Button>
        </ItemActions>
      )}
      <ReviewChangesDialog
        open={reviewing}
        onOpenChange={setReviewing}
        connectionId={change.connectionId}
        environment={profile?.environment ?? 'development'}
        changes={change.changes}
        items={items}
        held={change}
        onApplied={applied}
        onReload={() => refresh(change)}
        onDeclined={() => {
          setDecision({ status: 'declined' })
          refresh(change)
        }}
      />
    </Item>
  )
}

function StatusBadge({ waiting, decision }: { waiting: boolean; decision: Decision | null }) {
  if (decision?.status === 'applied') return <Badge variant="secondary">Applied</Badge>
  if (decision?.status === 'declined') return <Badge variant="outline">Declined</Badge>
  return waiting ? (
    <Badge variant="outline">Waiting for you</Badge>
  ) : (
    <Badge variant="outline">Closed</Badge>
  )
}
