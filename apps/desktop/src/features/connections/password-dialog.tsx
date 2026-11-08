import { useId, useState } from 'react'
import type { ConnectionDetails } from '@aidb/contracts'
import { Alert, AlertDescription } from '@/components/ui/alert'
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
import { Field, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Spinner } from '@/components/ui/spinner'

export type PasswordPrompt = { connection: ConnectionDetails; problem: string | null }

/** Asks for a connection's password, with the option to save it once it works. */
export function PasswordDialog({
  prompt,
  pending,
  onSubmit,
  onCancel,
}: {
  prompt: PasswordPrompt | null
  pending: boolean
  onSubmit: (password: string, remember: boolean) => void
  onCancel: () => void
}) {
  return (
    <Dialog open={prompt !== null} onOpenChange={(open) => !open && onCancel()}>
      <DialogContent>
        {prompt && (
          <PasswordForm
            key={prompt.connection.id}
            prompt={prompt}
            pending={pending}
            onSubmit={onSubmit}
            onCancel={onCancel}
          />
        )}
      </DialogContent>
    </Dialog>
  )
}

function PasswordForm({
  prompt,
  pending,
  onSubmit,
  onCancel,
}: {
  prompt: PasswordPrompt
  pending: boolean
  onSubmit: (password: string, remember: boolean) => void
  onCancel: () => void
}) {
  const passwordId = useId()
  const rememberId = useId()
  const [password, setPassword] = useState('')
  const [remember, setRemember] = useState(true)
  const { connection, problem } = prompt
  return (
    <form
      className="flex flex-col gap-6"
      onSubmit={(event) => {
        event.preventDefault()
        onSubmit(password, remember)
      }}
    >
      <DialogHeader>
        <DialogTitle>Password for {connection.name}</DialogTitle>
        <DialogDescription>
          User {connection.username} on {connection.host}.
        </DialogDescription>
      </DialogHeader>
      {problem && (
        <Alert variant="destructive">
          <AlertDescription>{problem}</AlertDescription>
        </Alert>
      )}
      <FieldGroup className="gap-4">
        <Field>
          <FieldLabel htmlFor={passwordId}>Password</FieldLabel>
          <Input
            id={passwordId}
            type="password"
            autoComplete="current-password"
            autoFocus
            className="min-h-11 sm:min-h-0"
            value={password}
            onChange={(event) => setPassword(event.target.value)}
          />
        </Field>
        <Field orientation="horizontal">
          <Checkbox
            id={rememberId}
            checked={remember}
            onCheckedChange={(checked) => setRemember(checked === true)}
          />
          <FieldLabel htmlFor={rememberId} className="font-normal">
            Remember it once it works
          </FieldLabel>
        </Field>
      </FieldGroup>
      <DialogFooter>
        <Button type="button" variant="outline" className="min-h-11 sm:min-h-0" onClick={onCancel}>
          Cancel
        </Button>
        <Button type="submit" className="min-h-11 sm:min-h-0" disabled={pending}>
          {pending && <Spinner data-icon="inline-start" />}
          Connect
        </Button>
      </DialogFooter>
    </form>
  )
}
