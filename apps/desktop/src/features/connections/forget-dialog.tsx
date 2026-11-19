import { useId, useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import type { ConnectionDetails } from '@aidb/contracts'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Checkbox } from '@/components/ui/checkbox'
import { Field, FieldContent, FieldDescription, FieldLabel } from '@/components/ui/field'
import { deleteConnection } from '@/lib/commands'
import { CONNECTIONS_QUERY_KEY } from './queries'
import { SESSIONS_QUERY_KEY } from './sessions'

/**
 * "Forget connection": removes the profile and its cached schema details, and asks whether to
 * remove its saved password too. Saved queries and history stay, detached from it.
 */
export function ForgetConnectionDialog({
  connection,
  onClose,
}: {
  connection: ConnectionDetails | null
  onClose: () => void
}) {
  return (
    <AlertDialog open={connection !== null} onOpenChange={(open) => !open && onClose()}>
      <AlertDialogContent>
        {connection && (
          <ForgetConfirmation key={connection.id} connection={connection} onClose={onClose} />
        )}
      </AlertDialogContent>
    </AlertDialog>
  )
}

function ForgetConfirmation({
  connection,
  onClose,
}: {
  connection: ConnectionDetails
  onClose: () => void
}) {
  const id = useId()
  const hasPassword = connection.password !== 'not_saved'
  const [forgetPassword, setForgetPassword] = useState(true)
  const queryClient = useQueryClient()
  const forget = useMutation({
    mutationFn: () =>
      deleteConnection(connection.id, { forgetPassword: hasPassword && forgetPassword }),
    onSuccess: () => {
      toast.success(`Forgot ${connection.name}`)
      onClose()
    },
    onError: (error) =>
      toast.error(`Couldn't forget ${connection.name}`, { description: error.message }),
    onSettled: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: CONNECTIONS_QUERY_KEY }),
        queryClient.invalidateQueries({ queryKey: SESSIONS_QUERY_KEY }),
      ]),
  })

  return (
    <>
      <AlertDialogHeader>
        <AlertDialogTitle>Forget {connection.name}?</AlertDialogTitle>
        <AlertDialogDescription>
          The connection and its cached schema details are removed from this device. Saved queries
          and history stay. If it is connected, it is disconnected.
        </AlertDialogDescription>
      </AlertDialogHeader>
      {hasPassword && (
        <Field orientation="horizontal">
          <Checkbox
            id={id}
            checked={forgetPassword}
            onCheckedChange={(checked) => setForgetPassword(checked === true)}
          />
          <FieldContent>
            <FieldLabel htmlFor={id}>Also remove its saved password</FieldLabel>
            <FieldDescription>
              {connection.password === 'os_store'
                ? 'From the system keychain. No other connection uses it.'
                : 'From memory for this session.'}
            </FieldDescription>
          </FieldContent>
        </Field>
      )}
      <AlertDialogFooter>
        <AlertDialogCancel disabled={forget.isPending}>Cancel</AlertDialogCancel>
        <AlertDialogAction
          variant="destructive"
          disabled={forget.isPending}
          onClick={(event) => {
            // Keep the dialog open until the core confirms.
            event.preventDefault()
            forget.mutate()
          }}
        >
          Forget connection
        </AlertDialogAction>
      </AlertDialogFooter>
    </>
  )
}
