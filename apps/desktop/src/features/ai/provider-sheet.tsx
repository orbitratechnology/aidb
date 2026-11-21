import { useId, useState, type ReactNode } from 'react'
import { zodResolver } from '@hookform/resolvers/zod'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { CircleAlertIcon, EyeIcon, EyeOffIcon, ShieldAlertIcon } from 'lucide-react'
import { Controller, useForm, useWatch, type UseFormReturn } from 'react-hook-form'
import { toast } from 'sonner'
import type { CredentialBackend, ProviderDetails } from '@aidb/contracts'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldError,
  FieldGroup,
  FieldLabel,
  FieldLegend,
  FieldSet,
} from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupInput,
  InputGroupText,
} from '@/components/ui/input-group'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { Spinner } from '@/components/ui/spinner'
import { createProvider, forgetProviderKey, saveProviderKey, updateProvider } from '@/lib/commands'
import { NativeError } from '@/lib/native'
import { SETTINGS_QUERY_KEY } from '@/hooks/use-settings'
import {
  emptyFormValues,
  formValuesFrom,
  needsInsecureConsent,
  PRESET_ORDER,
  PRESETS,
  ProviderFormSchema,
  switchKind,
  toDraft,
  type ProviderFormValues,
} from './provider-form'
import { PROVIDERS_QUERY_KEY } from './queries'
import { returnFocus } from '@/lib/focus-return'

export type ProviderFormMode = { kind: 'create' } | { kind: 'edit'; provider: ProviderDetails }

type Form = UseFormReturn<ProviderFormValues>

/** Create or edit a provider in a sheet; full width on phones. */
export function ProviderSheet({
  mode,
  onClose,
  onTest,
}: {
  mode: ProviderFormMode | null
  onClose: () => void
  /** Called after "Save and test" has saved the provider. */
  onTest: (provider: ProviderDetails) => void
}) {
  return (
    <Sheet open={mode !== null} onOpenChange={(open) => !open && onClose()}>
      <SheetContent
        {...returnFocus}
        side="right"
        className="gap-0 data-[side=right]:w-full data-[side=right]:sm:max-w-xl"
      >
        {mode && (
          <ProviderForm
            key={mode.kind === 'edit' ? mode.provider.id : 'new'}
            mode={mode}
            onDone={onClose}
            onTest={onTest}
          />
        )}
      </SheetContent>
    </Sheet>
  )
}

type SaveResult = { saved: ProviderDetails; stored: CredentialBackend | null; test: boolean }

/** The provider was saved, but storing its key failed. */
class SavedWithoutKey extends Error {
  constructor(cause: unknown) {
    super(cause instanceof Error ? cause.message : String(cause))
    this.name = 'SavedWithoutKey'
  }
}

function ProviderForm({
  mode,
  onDone,
  onTest,
}: {
  mode: ProviderFormMode
  onDone: () => void
  onTest: (provider: ProviderDetails) => void
}) {
  const editing = mode.kind === 'edit' ? mode.provider : null
  const formId = useId()
  const queryClient = useQueryClient()
  const form = useForm({
    resolver: zodResolver(ProviderFormSchema),
    defaultValues: editing ? formValuesFrom(editing) : emptyFormValues(),
    mode: 'onTouched',
  })
  const [kind, endpoint, hasSavedKey] = useWatch({
    control: form.control,
    name: ['kind', 'endpoint', 'hasSavedKey'],
  })
  const preset = PRESETS[kind]

  const refresh = () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: PROVIDERS_QUERY_KEY }),
      queryClient.invalidateQueries({ queryKey: SETTINGS_QUERY_KEY }),
    ])

  const save = useMutation({
    mutationFn: async ({
      values,
      test,
    }: {
      values: ProviderFormValues
      test: boolean
    }): Promise<SaveResult> => {
      const draft = toDraft(values)
      let saved = editing ? await updateProvider(editing.id, draft) : await createProvider(draft)
      const key = values.apiKey.trim()
      const stored = key
        ? await saveProviderKey(saved.id, key).catch((error: unknown) => {
            throw new SavedWithoutKey(error)
          })
        : null
      if (stored) saved = { ...saved, key: stored === 'os_store' ? 'os_store' : 'session_only' }
      return { saved, stored, test }
    },
    onSuccess: ({ saved, stored, test }) => {
      const keyNote =
        stored === 'os_store'
          ? 'The API key is in the system keychain.'
          : stored === 'session_only'
            ? 'The API key is kept until aidb quits; no keychain is available.'
            : undefined
      toast.success(editing ? `Saved ${saved.name}` : `Added ${saved.name}`, {
        description: keyNote,
      })
      onDone()
      if (test) onTest(saved)
    },
    onError: (error) => {
      if (error instanceof SavedWithoutKey) {
        toast.error('Saved, but the API key could not be stored', { description: error.message })
        onDone()
      } else if (error instanceof NativeError && error.code === 'conflict') {
        form.setError('name', { message: error.message })
      } else {
        form.setError('root.server', { message: error.message })
      }
    },
    onSettled: () => void refresh(),
  })

  const forgetKey = useMutation({
    mutationFn: () => forgetProviderKey(editing?.id ?? ''),
    onSuccess: () => {
      form.setValue('hasSavedKey', false, { shouldValidate: form.formState.isSubmitted })
      toast.success('API key removed')
      void refresh()
    },
    onError: (error) => toast.error("Couldn't remove the API key", { description: error.message }),
  })

  const submit = (test: boolean) => form.handleSubmit((values) => save.mutate({ values, test }))
  const serverError = form.formState.errors.root?.server?.message

  return (
    <>
      <SheetHeader className="border-b">
        <SheetTitle>{editing ? `Edit ${editing.name}` : 'Add an AI provider'}</SheetTitle>
        <SheetDescription>
          API keys go to the system keychain, never into app files. Database credentials are never
          sent to a provider.
        </SheetDescription>
      </SheetHeader>
      <form
        id={formId}
        noValidate
        onSubmit={(event) => void submit(false)(event)}
        className="flex min-h-0 flex-1 flex-col gap-8 overflow-y-auto px-4 py-6"
      >
        <FieldSet>
          <FieldLegend>Provider</FieldLegend>
          <FieldGroup>
            <Controller
              control={form.control}
              name="kind"
              render={({ field }) => (
                <Field>
                  <FieldLabel htmlFor={`${formId}-kind`}>Kind</FieldLabel>
                  <Select
                    value={field.value}
                    onValueChange={(next) => {
                      const values = switchKind(form.getValues(), next as typeof field.value)
                      form.setValue('kind', values.kind)
                      form.setValue('name', values.name)
                      form.setValue('model', values.model)
                    }}
                  >
                    <SelectTrigger id={`${formId}-kind`} className="min-h-11 w-full sm:min-h-0">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {PRESET_ORDER.map((value) => (
                        <SelectItem key={value} value={value}>
                          {PRESETS[value].label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <FieldDescription>{preset.description}</FieldDescription>
                </Field>
              )}
            />
            <TextField form={form} name="name" label="Name" />
            <TextField
              form={form}
              name="model"
              label="Model"
              placeholder={preset.modelPlaceholder}
              description="The model ID as the provider names it. Test the provider to see the models it lists."
              mono
            />
            <TextField
              form={form}
              name="endpoint"
              label={preset.defaultEndpoint ? 'Endpoint (optional)' : 'Endpoint'}
              placeholder={preset.defaultEndpoint ?? 'http://localhost:1234/v1'}
              description={
                preset.defaultEndpoint
                  ? `Leave empty to use ${preset.defaultEndpoint}.`
                  : 'The API root of the server, usually ending in /v1.'
              }
              mono
            />
            {needsInsecureConsent(kind, endpoint) && (
              <Controller
                control={form.control}
                name="allowInsecureHttp"
                render={({ field, fieldState }) => (
                  <Field orientation="horizontal" data-invalid={fieldState.invalid}>
                    <Checkbox
                      id={`${formId}-insecure`}
                      checked={field.value}
                      onCheckedChange={(checked) => field.onChange(checked === true)}
                      aria-invalid={fieldState.invalid}
                    />
                    <FieldContent>
                      <FieldLabel htmlFor={`${formId}-insecure`}>
                        <ShieldAlertIcon className="text-destructive" />
                        Allow plain HTTP to another computer
                      </FieldLabel>
                      <FieldDescription>
                        Anyone on the network path could read your prompts, the schema details sent
                        with them, and the API key.
                      </FieldDescription>
                      {fieldState.invalid && <FieldError errors={[fieldState.error]} />}
                    </FieldContent>
                  </Field>
                )}
              />
            )}
            <KeyField
              form={form}
              required={preset.keyRequired}
              hasSavedKey={hasSavedKey}
              removing={forgetKey.isPending}
              onRemove={editing ? () => forgetKey.mutate() : undefined}
            />
          </FieldGroup>
        </FieldSet>

        <FieldSet>
          <FieldLegend>Limits</FieldLegend>
          <FieldGroup>
            <Controller
              control={form.control}
              name="requestTimeoutSecs"
              render={({ field, fieldState }) => (
                <Field data-invalid={fieldState.invalid}>
                  <FieldLabel htmlFor={`${formId}-timeout`}>Timeout</FieldLabel>
                  <InputGroup className="h-11 sm:h-9">
                    <InputGroupInput
                      id={`${formId}-timeout`}
                      name={field.name}
                      ref={field.ref}
                      type="number"
                      inputMode="numeric"
                      min={5}
                      max={600}
                      step={1}
                      value={Number.isNaN(field.value) ? '' : field.value}
                      onChange={(event) => field.onChange(event.target.valueAsNumber)}
                      onBlur={field.onBlur}
                      aria-invalid={fieldState.invalid}
                      className="[appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none"
                    />
                    <InputGroupAddon align="inline-end">
                      <InputGroupText>sec</InputGroupText>
                    </InputGroupAddon>
                  </InputGroup>
                  <FieldDescription>
                    The longest wait for the provider to start or continue an answer. Local models
                    can be slow to load.
                  </FieldDescription>
                  {fieldState.invalid && <FieldError errors={[fieldState.error]} />}
                </Field>
              )}
            />
            <TextField
              form={form}
              name="contextBudgetTokens"
              label="Context budget (optional)"
              placeholder="Automatic"
              description="The most input tokens the assistant sends in one request. Leave empty to let it choose."
              inputMode="numeric"
            />
          </FieldGroup>
        </FieldSet>

        {serverError && (
          <Alert variant="destructive">
            <CircleAlertIcon />
            <AlertTitle>The provider was not saved</AlertTitle>
            <AlertDescription>{serverError}</AlertDescription>
          </Alert>
        )}
      </form>
      <SheetFooter className="flex-row flex-wrap border-t">
        <Button
          type="button"
          variant="outline"
          className="min-h-11 sm:min-h-0"
          disabled={save.isPending}
          onClick={() => void submit(true)()}
        >
          {save.isPending && save.variables.test && <Spinner data-icon="inline-start" />}
          Save and test
        </Button>
        <div className="ms-auto flex gap-2">
          <Button type="button" variant="ghost" className="min-h-11 sm:min-h-0" onClick={onDone}>
            Cancel
          </Button>
          <Button
            type="submit"
            form={formId}
            className="min-h-11 sm:min-h-0"
            disabled={save.isPending}
          >
            {save.isPending && !save.variables.test && <Spinner data-icon="inline-start" />}
            {editing ? 'Save changes' : 'Add provider'}
          </Button>
        </div>
      </SheetFooter>
    </>
  )
}

type TextName = 'name' | 'model' | 'endpoint' | 'contextBudgetTokens'

function TextField({
  form,
  name,
  label,
  placeholder,
  description,
  mono,
  inputMode,
}: {
  form: Form
  name: TextName
  label: string
  placeholder?: string
  description?: ReactNode
  mono?: boolean
  inputMode?: 'numeric'
}) {
  const id = useId()
  return (
    <Controller
      control={form.control}
      name={name}
      render={({ field, fieldState }) => (
        <Field data-invalid={fieldState.invalid}>
          <FieldLabel htmlFor={id}>{label}</FieldLabel>
          <Input
            {...field}
            id={id}
            placeholder={placeholder}
            autoComplete="off"
            spellCheck={false}
            inputMode={inputMode}
            className={mono ? 'min-h-11 font-mono sm:min-h-0' : 'min-h-11 sm:min-h-0'}
            aria-invalid={fieldState.invalid}
          />
          {description && <FieldDescription>{description}</FieldDescription>}
          {fieldState.invalid && <FieldError errors={[fieldState.error]} />}
        </Field>
      )}
    />
  )
}

function KeyField({
  form,
  required,
  hasSavedKey,
  removing,
  onRemove,
}: {
  form: Form
  required: boolean
  hasSavedKey: boolean
  removing: boolean
  onRemove?: () => void
}) {
  const id = useId()
  const [visible, setVisible] = useState(false)
  return (
    <Controller
      control={form.control}
      name="apiKey"
      render={({ field, fieldState }) => (
        <Field data-invalid={fieldState.invalid}>
          <FieldLabel htmlFor={id}>{required ? 'API key' : 'API key (optional)'}</FieldLabel>
          <InputGroup className="h-11 sm:h-9">
            <InputGroupInput
              {...field}
              id={id}
              type={visible ? 'text' : 'password'}
              autoComplete="off"
              spellCheck={false}
              placeholder={hasSavedKey ? 'Saved; leave blank to keep it' : undefined}
              aria-invalid={fieldState.invalid}
              className="font-mono"
            />
            <InputGroupAddon align="inline-end">
              <InputGroupButton
                size="icon-xs"
                aria-label={visible ? 'Hide API key' : 'Show API key'}
                aria-pressed={visible}
                onClick={() => setVisible((shown) => !shown)}
              >
                {visible ? <EyeOffIcon /> : <EyeIcon />}
              </InputGroupButton>
            </InputGroupAddon>
          </InputGroup>
          <FieldDescription>
            Kept in the system keychain, or until aidb quits if no keychain is available. It is sent
            only to this provider.
          </FieldDescription>
          {fieldState.invalid && <FieldError errors={[fieldState.error]} />}
          {hasSavedKey && onRemove && (
            <Button
              type="button"
              variant="link"
              size="sm"
              className="min-h-11 self-start px-0 sm:min-h-0"
              disabled={removing}
              onClick={onRemove}
            >
              Remove the saved key
            </Button>
          )}
        </Field>
      )}
    />
  )
}
