import { zodResolver } from '@hookform/resolvers/zod'
import { ShieldCheckIcon } from 'lucide-react'
import { Controller, useForm } from 'react-hook-form'
import { toast } from 'sonner'
import {
  SAFETY_DEFAULTS,
  SAFETY_LIMITS,
  SafetySettingsSchema,
  type SafetySettings as SafetyValues,
} from '@aidb/contracts'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
} from '@/components/ui/field'
import {
  InputGroup,
  InputGroupAddon,
  InputGroupInput,
  InputGroupText,
} from '@/components/ui/input-group'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { useSettings, useUpdateSettings } from '@/hooks/use-settings'
import { LoadError } from '../load-error'
import { SettingsSection } from '../settings-section'

type FieldSpec = {
  name: keyof SafetyValues
  label: string
  unit: string
  description: string
}

// Labels match the field names in the native core's validation messages.
const FIELDS: readonly FieldSpec[] = [
  {
    name: 'maxRowsPerPage',
    label: 'Rows per page',
    unit: 'rows',
    description: 'Most rows loaded at once in a table or query result.',
  },
  {
    name: 'maxResultMegabytes',
    label: 'Maximum result size',
    unit: 'MB',
    description: 'A result stops loading at this size and is marked as cut short.',
  },
  {
    name: 'statementTimeoutSecs',
    label: 'Statement timeout',
    unit: 'sec',
    description: 'PostgreSQL cancels a statement still running after this long.',
  },
  {
    name: 'lockTimeoutSecs',
    label: 'Lock timeout',
    unit: 'sec',
    description: 'How long a statement may wait for a lock. No longer than the statement timeout.',
  },
]

export function SafetySettings() {
  const settings = useSettings()

  return (
    <SettingsSection
      title="Safety"
      description="Limits applied to every query, whether you or the agent runs it. Changes apply from the next connection."
    >
      <Alert>
        <ShieldCheckIcon />
        <AlertTitle>Connections are read-only by default</AlertTitle>
        <AlertDescription>
          New connections can only read. Changing data always needs a preview and your explicit
          approval. The limits below apply on top of that.
        </AlertDescription>
      </Alert>
      {settings.isPending && <Skeleton className="h-96 w-full" />}
      {settings.isError && (
        <LoadError
          title="Safety settings could not be loaded"
          error={settings.error}
          retrying={settings.isFetching}
          onRetry={() => void settings.refetch()}
        />
      )}
      {settings.isSuccess && <SafetyForm saved={settings.data.safety} />}
    </SettingsSection>
  )
}

function SafetyForm({ saved }: { saved: SafetyValues }) {
  const { mutate, isPending } = useUpdateSettings()
  const form = useForm({
    resolver: zodResolver(SafetySettingsSchema),
    defaultValues: saved,
    mode: 'onTouched',
  })
  const { isDirty } = form.formState

  const onSubmit = form.handleSubmit((safety) => {
    mutate(
      { safety },
      {
        onSuccess: (settings) => {
          form.reset(settings.safety)
          toast.success('Safety settings saved')
        },
        onError: (error) =>
          toast.error("Couldn't save safety settings", { description: error.message }),
      },
    )
  })

  return (
    <form noValidate onSubmit={(event) => void onSubmit(event)} className="flex flex-col gap-6">
      <FieldGroup>
        {FIELDS.map(({ name, label, unit, description }) => (
          <Controller
            key={name}
            name={name}
            control={form.control}
            render={({ field, fieldState }) => {
              const id = `safety-${name}`
              const describedBy = [`${id}-unit`, `${id}-description`]
              if (fieldState.invalid) describedBy.push(`${id}-error`)
              return (
                <Field orientation="responsive" data-invalid={fieldState.invalid}>
                  <FieldContent>
                    <FieldLabel htmlFor={id}>{label}</FieldLabel>
                    <FieldDescription id={`${id}-description`}>{description}</FieldDescription>
                    {fieldState.invalid && (
                      <FieldError id={`${id}-error`} errors={[fieldState.error]} />
                    )}
                  </FieldContent>
                  <InputGroup className="h-11 min-w-40 sm:h-9">
                    <InputGroupInput
                      id={id}
                      name={field.name}
                      ref={field.ref}
                      type="number"
                      inputMode="numeric"
                      min={SAFETY_LIMITS[name].min}
                      max={SAFETY_LIMITS[name].max}
                      step={1}
                      // An empty box reads as NaN, which the schema reports as missing.
                      value={Number.isNaN(field.value) ? '' : field.value}
                      onChange={(event) => field.onChange(event.target.valueAsNumber)}
                      onBlur={field.onBlur}
                      aria-invalid={fieldState.invalid}
                      aria-describedby={describedBy.join(' ')}
                      // Arrow keys still step the value; the tiny native spin buttons are hidden.
                      className="[appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none"
                    />
                    <InputGroupAddon align="inline-end">
                      <InputGroupText id={`${id}-unit`}>{unit}</InputGroupText>
                    </InputGroupAddon>
                  </InputGroup>
                </Field>
              )
            }}
          />
        ))}
      </FieldGroup>
      <div className="flex flex-wrap gap-2">
        <Button type="submit" disabled={!isDirty || isPending} className="min-h-11 sm:min-h-0">
          {isPending && <Spinner data-icon="inline-start" />}
          Save changes
        </Button>
        <Button
          type="button"
          variant="outline"
          disabled={!isDirty || isPending}
          className="min-h-11 sm:min-h-0"
          onClick={() => form.reset()}
        >
          Discard changes
        </Button>
        <Button
          type="button"
          variant="ghost"
          disabled={isPending}
          className="min-h-11 sm:min-h-0"
          // Fills in the defaults without saving, so they can be reviewed first.
          onClick={() => form.reset(SAFETY_DEFAULTS, { keepDefaultValues: true })}
        >
          Restore defaults
        </Button>
      </div>
    </form>
  )
}
