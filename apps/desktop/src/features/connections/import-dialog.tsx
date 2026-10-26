import { useId, useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import {
  CircleCheckIcon,
  CircleSlashIcon,
  LockIcon,
  ShieldOffIcon,
  type LucideIcon,
} from 'lucide-react'
import { toast } from 'sonner'
import type { ImportCandidate, ImportPreview, ImportStatus } from '@aidb/contracts'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Field, FieldContent, FieldDescription, FieldLabel } from '@/components/ui/field'
import { Item, ItemContent, ItemDescription, ItemGroup, ItemTitle } from '@/components/ui/item'
import { Spinner } from '@/components/ui/spinner'
import { importConnections } from '@/lib/commands'
import { connectionAddress, pluralize } from './format'
import { planImport } from './import-plan'
import { CONNECTIONS_QUERY_KEY } from './queries'

const STATUS_DISPLAY: Record<
  ImportStatus,
  { label: string; icon: LucideIcon; variant: 'outline' | 'secondary' }
> = {
  ready: { label: 'Will import', icon: CircleCheckIcon, variant: 'outline' },
  duplicate: { label: 'Skipped', icon: CircleSlashIcon, variant: 'secondary' },
  invalid: { label: 'Skipped', icon: CircleSlashIcon, variant: 'secondary' },
}

/** Shows what an import file holds and imports the usable entries, read-only. */
export function ImportConnectionsDialog({
  preview,
  onClose,
}: {
  preview: ImportPreview | null
  onClose: () => void
}) {
  return (
    <Dialog open={preview !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="flex max-h-[90dvh] flex-col sm:max-w-xl">
        {preview && <ImportReview preview={preview} onClose={onClose} />}
      </DialogContent>
    </Dialog>
  )
}

function ImportReview({ preview, onClose }: { preview: ImportPreview; onClose: () => void }) {
  const consentId = useId()
  const [allowUnencrypted, setAllowUnencrypted] = useState(false)
  const plan = planImport(preview.candidates, allowUnencrypted)
  const queryClient = useQueryClient()
  const { mutate, isPending } = useMutation({
    mutationFn: () => importConnections(plan.connections, { allowUnencrypted }),
    onSuccess: ({ imported, skipped }) => {
      const message = `Imported ${pluralize(imported, 'connection', 'connections')}`
      if (skipped.length === 0) {
        toast.success(message)
      } else {
        toast.warning(message, {
          description: skipped.map((entry) => `${entry.name}: ${entry.reason}`).join('\n'),
        })
      }
      onClose()
    },
    onError: (error) => toast.error("Couldn't import connections", { description: error.message }),
    onSettled: () => queryClient.invalidateQueries({ queryKey: CONNECTIONS_QUERY_KEY }),
  })

  const count = plan.connections.length
  return (
    <>
      <DialogHeader>
        <DialogTitle>Import connections</DialogTitle>
        <DialogDescription>
          From {preview.fileName}. Imported connections start read-only, and files never contain
          passwords.
        </DialogDescription>
      </DialogHeader>
      {preview.candidates.length === 0 ? (
        <p className="text-sm text-muted-foreground">The file holds no connections.</p>
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain rounded-lg border">
          <ItemGroup className="gap-0">
            {preview.candidates.map((candidate, index) => (
              <CandidateRow key={index} candidate={candidate} />
            ))}
          </ItemGroup>
        </div>
      )}
      {plan.needsUnencryptedConsent && (
        <Field orientation="horizontal">
          <Checkbox
            id={consentId}
            checked={allowUnencrypted}
            onCheckedChange={(checked) => setAllowUnencrypted(checked === true)}
          />
          <FieldContent>
            <FieldLabel htmlFor={consentId}>Allow unencrypted connections</FieldLabel>
            <FieldDescription>
              TLS is off for some remote hosts, so their passwords and data would cross the network
              in the clear. Without this, they are not imported.
            </FieldDescription>
          </FieldContent>
        </Field>
      )}
      <DialogFooter>
        <Button variant="outline" className="min-h-11 sm:min-h-0" onClick={onClose}>
          Cancel
        </Button>
        <Button
          className="min-h-11 sm:min-h-0"
          disabled={count === 0 || isPending}
          onClick={() => mutate()}
        >
          {isPending && <Spinner data-icon="inline-start" />}
          {count === 0
            ? 'Nothing to import'
            : `Import ${pluralize(count, 'connection', 'connections')}`}
        </Button>
      </DialogFooter>
    </>
  )
}

function CandidateRow({ candidate }: { candidate: ImportCandidate }) {
  const status = STATUS_DISPLAY[candidate.status]
  const notes = [
    candidate.problem,
    candidate.writesDisabled && 'Writes were enabled in the file; imported read-only.',
  ].filter((note): note is string => Boolean(note))
  return (
    <Item size="sm" role="listitem" className="border-b last:border-b-0">
      <ItemContent className="min-w-0">
        <ItemTitle className="flex-wrap">
          <span className="truncate">{candidate.label}</span>
          <Badge variant={status.variant}>
            <status.icon data-icon="inline-start" />
            {status.label}
          </Badge>
          {candidate.unencryptedRemote && (
            <Badge variant="destructive">
              <ShieldOffIcon data-icon="inline-start" />
              TLS off
            </Badge>
          )}
          {candidate.writesDisabled && (
            <Badge variant="outline">
              <LockIcon data-icon="inline-start" />
              Read-only
            </Badge>
          )}
        </ItemTitle>
        {candidate.connection && (
          <ItemDescription className="font-mono text-xs break-all">
            {connectionAddress(candidate.connection)}
          </ItemDescription>
        )}
        {notes.map((note) => (
          <ItemDescription key={note}>{note}</ItemDescription>
        ))}
      </ItemContent>
    </Item>
  )
}
