import { useId } from 'react'
import { zodResolver } from '@hookform/resolvers/zod'
import { Controller, useForm } from 'react-hook-form'
import { z } from 'zod'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Field, FieldError, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Spinner } from '@/components/ui/spinner'
import { returnFocus } from '@/lib/focus-return'

/** The core enforces the same limit. */
const MAX_NAME_CHARS = 200

const schema = z.object({
  name: z
    .string()
    .trim()
    .min(1, 'Give the query a name.')
    .max(MAX_NAME_CHARS, `Names can be at most ${MAX_NAME_CHARS} characters.`),
})

type Values = z.infer<typeof schema>

type Props = {
  open: boolean
  onOpenChange: (open: boolean) => void
  defaultName: string
  /** Resolves once saved; a rejection keeps the dialog open. */
  onSave: (name: string) => Promise<void>
}

/** Names the current tab's SQL and keeps it as a saved query. */
export function SaveQueryDialog({ open, onOpenChange, ...props }: Props) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent {...returnFocus} className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Save query</DialogTitle>
          <DialogDescription>
            Saved queries are kept on this device until you delete them. They are never run on their
            own.
          </DialogDescription>
        </DialogHeader>
        {/* Mounted only while open, so it starts from the tab's name each time. */}
        {open && <SaveForm {...props} onCancel={() => onOpenChange(false)} />}
      </DialogContent>
    </Dialog>
  )
}

function SaveForm({
  defaultName,
  onSave,
  onCancel,
}: Omit<Props, 'open' | 'onOpenChange'> & { onCancel: () => void }) {
  const id = useId()
  const form = useForm<Values>({
    resolver: zodResolver(schema),
    defaultValues: { name: defaultName },
  })
  const submit = form.handleSubmit(async ({ name }) => {
    try {
      await onSave(name)
    } catch (error) {
      form.setError('name', { message: error instanceof Error ? error.message : String(error) })
    }
  })

  return (
    <form onSubmit={submit} className="flex flex-col gap-6">
      <Controller
        control={form.control}
        name="name"
        render={({ field, fieldState }) => (
          <Field data-invalid={fieldState.invalid}>
            <FieldLabel htmlFor={id}>Name</FieldLabel>
            <Input
              {...field}
              id={id}
              autoComplete="off"
              className="min-h-11 sm:min-h-0"
              aria-invalid={fieldState.invalid}
            />
            {fieldState.invalid && <FieldError errors={[fieldState.error]} />}
          </Field>
        )}
      />
      <DialogFooter>
        <Button type="button" variant="outline" className="max-sm:min-h-11" onClick={onCancel}>
          Cancel
        </Button>
        <Button type="submit" className="max-sm:min-h-11" disabled={form.formState.isSubmitting}>
          {form.formState.isSubmitting && <Spinner data-icon="inline-start" />}
          Save
        </Button>
      </DialogFooter>
    </form>
  )
}
