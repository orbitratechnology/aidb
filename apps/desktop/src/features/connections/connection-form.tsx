import { useId, useState, type ReactNode } from 'react'
import { zodResolver } from '@hookform/resolvers/zod'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import {
  CircleAlertIcon,
  CircleCheckIcon,
  EyeIcon,
  EyeOffIcon,
  InfoIcon,
  LinkIcon,
  ShieldAlertIcon,
  TriangleAlertIcon,
} from 'lucide-react'
import { Controller, useForm, useWatch, type Path } from 'react-hook-form'
import { toast } from 'sonner'
import type {
  ConnectionDetails,
  ConnectionTestReport,
  CredentialBackend,
  SslMode,
} from '@aidb/contracts'
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
import { Switch } from '@/components/ui/switch'
import {
  createConnection,
  forgetConnectionPassword,
  parseConnectionUri,
  saveConnectionPassword,
  testConnection,
  updateConnection,
} from '@/lib/commands'
import { NativeError } from '@/lib/native'
import { ENVIRONMENT_DISPLAY } from './environments'
import {
  ConnectionFormSchema,
  applyParsedUri,
  emptyFormValues,
  formValuesFrom,
  needsUnencryptedConsent,
  toDraft,
  usesCaCertificate,
  type ConnectionFormValues,
} from './form'
import { CONNECTIONS_QUERY_KEY } from './queries'
import { SESSIONS_QUERY_KEY, useActiveSessions } from './sessions'
import { ENVIRONMENTS } from './types'

export type FormMode = { kind: 'create' } | { kind: 'edit'; connection: ConnectionDetails }

const SSL_MODES: readonly { value: SslMode; label: string; description: string }[] = [
  {
    value: 'verify-full',
    label: 'Verify certificate and host',
    description: 'Recommended. Encrypts and checks the server is who it says it is.',
  },
  {
    value: 'verify-ca',
    label: 'Verify certificate only',
    description: 'Encrypts and checks the certificate, but not that it names this host.',
  },
  {
    value: 'require',
    label: 'Encrypt without verifying',
    description: 'Encrypts, but cannot detect an impostor server.',
  },
  {
    value: 'disable',
    label: 'No TLS',
    description: 'The password and data cross the network unencrypted.',
  },
]

/** Create or edit a connection in a sheet; full width on phones. */
export function ConnectionSheet({ mode, onClose }: { mode: FormMode | null; onClose: () => void }) {
  return (
    <Sheet open={mode !== null} onOpenChange={(open) => !open && onClose()}>
      <SheetContent
        side="right"
        className="gap-0 data-[side=right]:w-full data-[side=right]:sm:max-w-xl"
      >
        {mode && (
          <ConnectionForm
            key={mode.kind === 'edit' ? mode.connection.id : 'new'}
            mode={mode}
            onDone={onClose}
          />
        )}
      </SheetContent>
    </Sheet>
  )
}

type SaveResult = { saved: ConnectionDetails; stored: CredentialBackend | null }

function ConnectionForm({ mode, onDone }: { mode: FormMode; onDone: () => void }) {
  const editing = mode.kind === 'edit' ? mode.connection : null
  const formId = useId()
  const queryClient = useQueryClient()
  const sessions = useActiveSessions()
  const wasConnected =
    editing !== null && (sessions.data ?? []).some((s) => s.connectionId === editing.id)
  const form = useForm({
    resolver: zodResolver(ConnectionFormSchema),
    defaultValues: editing ? formValuesFrom(editing) : emptyFormValues(),
    mode: 'onTouched',
  })
  const [sslMode, host] = useWatch({ control: form.control, name: ['sslMode', 'host'] })
  const readOnly = useWatch({ control: form.control, name: 'readOnly' })
  const environment = useWatch({ control: form.control, name: 'environment' })
  const [report, setReport] = useState<
    { ok: true; report: ConnectionTestReport } | { ok: false; message: string } | null
  >(null)
  const [hasSavedPassword, setHasSavedPassword] = useState(
    editing !== null && editing.password !== 'not_saved',
  )

  const refresh = () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: CONNECTIONS_QUERY_KEY }),
      queryClient.invalidateQueries({ queryKey: SESSIONS_QUERY_KEY }),
    ])

  const test = useMutation({
    mutationFn: (values: ConnectionFormValues) =>
      testConnection(toDraft(values), {
        password: values.password || undefined,
        savedId: editing?.id,
        allowUnencrypted: values.allowUnencrypted,
      }),
    onMutate: () => setReport(null),
    onSuccess: (result) => setReport({ ok: true, report: result }),
    onError: (error) => setReport({ ok: false, message: error.message }),
  })

  const save = useMutation({
    mutationFn: async (values: ConnectionFormValues): Promise<SaveResult> => {
      const draft = toDraft(values)
      const options = { allowUnencrypted: values.allowUnencrypted }
      const saved = editing
        ? await updateConnection(editing.id, draft, options)
        : await createConnection(draft, options)
      const stored =
        values.password && values.rememberPassword
          ? await saveConnectionPassword(saved.id, values.password).catch((error: unknown) => {
              throw new SavedWithoutPassword(error)
            })
          : null
      return { saved, stored }
    },
    onSuccess: ({ saved, stored }, values) => {
      const passwordNote =
        stored === 'os_store'
          ? 'The password is in the system keychain.'
          : stored === 'session_only'
            ? 'The password is kept until aidb quits; no keychain is available.'
            : values.password
              ? "The password was not saved; you'll be asked for it when connecting."
              : undefined
      const disconnectNote = wasConnected
        ? 'Disconnected so the new settings apply; connect again to continue.'
        : undefined
      toast.success(editing ? `Saved ${saved.name}` : `Added ${saved.name}`, {
        description: [disconnectNote, passwordNote].filter(Boolean).join(' ') || undefined,
      })
      onDone()
    },
    onError: (error) => {
      if (error instanceof SavedWithoutPassword) {
        toast.error('Saved, but the password could not be stored', {
          description: error.message,
        })
        onDone()
      } else if (error instanceof NativeError && error.code === 'conflict') {
        form.setError('name', { message: error.message })
      } else {
        form.setError('root.server', { message: error.message })
      }
    },
    onSettled: () => void refresh(),
  })

  const forgetPassword = useMutation({
    mutationFn: () => forgetConnectionPassword(editing?.id ?? ''),
    onSuccess: () => {
      setHasSavedPassword(false)
      toast.success('Saved password removed')
      void refresh()
    },
    onError: (error) => toast.error("Couldn't remove the password", { description: error.message }),
  })

  const onSave = form.handleSubmit((values) => save.mutate(values))
  const onTest = form.handleSubmit((values) => test.mutate(values))
  const busy = save.isPending || test.isPending
  const serverError = form.formState.errors.root?.server?.message

  return (
    <>
      <SheetHeader className="border-b">
        <SheetTitle>{editing ? `Edit ${editing.name}` : 'Add a connection'}</SheetTitle>
        <SheetDescription>
          New connections are read-only. Passwords go to the system keychain, never into app files.
        </SheetDescription>
      </SheetHeader>
      <form
        id={formId}
        noValidate
        onSubmit={(event) => void onSave(event)}
        className="flex min-h-0 flex-1 flex-col gap-8 overflow-y-auto px-4 py-6"
      >
        <UriField
          onParsed={(parsed) => {
            const { values, filled } = applyParsedUri(form.getValues(), parsed)
            for (const field of filled) {
              form.setValue(field, values[field], { shouldDirty: true, shouldValidate: true })
            }
            return parsed.notes
          }}
        />

        <FieldSet>
          <FieldLegend>Connection</FieldLegend>
          <FieldGroup>
            <TextField form={form} name="name" label="Name" placeholder="Orders (staging)" />
            <Controller
              control={form.control}
              name="environment"
              render={({ field }) => (
                <Field>
                  <FieldLabel htmlFor={`${formId}-environment`}>Environment</FieldLabel>
                  <Select value={field.value} onValueChange={field.onChange}>
                    <SelectTrigger
                      id={`${formId}-environment`}
                      className="min-h-11 w-full sm:min-h-0"
                    >
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {ENVIRONMENTS.map((value) => {
                        const { label, icon: Icon } = ENVIRONMENT_DISPLAY[value]
                        return (
                          <SelectItem key={value} value={value}>
                            <Icon />
                            {label}
                          </SelectItem>
                        )
                      })}
                    </SelectContent>
                  </Select>
                  <FieldDescription>
                    Shown, with its icon, wherever this connection is in use.
                  </FieldDescription>
                </Field>
              )}
            />
            <div className="grid gap-7 sm:grid-cols-[1fr_8rem] sm:gap-4">
              <TextField form={form} name="host" label="Host" placeholder="db.example.com" />
              <NumberField form={form} name="port" label="Port" min={1} max={65_535} />
            </div>
            <TextField form={form} name="database" label="Database" placeholder="postgres" mono />
            <TextField form={form} name="username" label="User" mono autoComplete="username" />
            <PasswordField
              form={form}
              hasSavedPassword={hasSavedPassword}
              removing={forgetPassword.isPending}
              onRemove={() => forgetPassword.mutate()}
            />
          </FieldGroup>
        </FieldSet>

        <FieldSet>
          <FieldLegend>Security</FieldLegend>
          <FieldGroup>
            <Controller
              control={form.control}
              name="sslMode"
              render={({ field }) => (
                <Field>
                  <FieldLabel htmlFor={`${formId}-ssl`}>TLS</FieldLabel>
                  <Select value={field.value} onValueChange={field.onChange}>
                    <SelectTrigger id={`${formId}-ssl`} className="min-h-11 w-full sm:min-h-0">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {SSL_MODES.map((mode) => (
                        <SelectItem key={mode.value} value={mode.value}>
                          {mode.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <FieldDescription>
                    {SSL_MODES.find((mode) => mode.value === field.value)?.description}
                  </FieldDescription>
                </Field>
              )}
            />
            {usesCaCertificate(sslMode) && (
              <TextField
                form={form}
                name="caCertPath"
                label="CA certificate (optional)"
                placeholder="/path/to/root.crt"
                description="Only needed when the server's certificate is not signed by an authority this computer trusts."
                mono
              />
            )}
            {needsUnencryptedConsent({ sslMode, host }) && (
              <Controller
                control={form.control}
                name="allowUnencrypted"
                render={({ field, fieldState }) => (
                  <Field orientation="horizontal" data-invalid={fieldState.invalid}>
                    <Checkbox
                      id={`${formId}-unencrypted`}
                      checked={field.value}
                      onCheckedChange={(checked) => field.onChange(checked === true)}
                      aria-invalid={fieldState.invalid}
                    />
                    <FieldContent>
                      <FieldLabel htmlFor={`${formId}-unencrypted`}>
                        <ShieldAlertIcon className="text-destructive" />
                        Allow an unencrypted connection to {host.trim()}
                      </FieldLabel>
                      <FieldDescription>
                        Anyone on the network path could read the password and the data.
                      </FieldDescription>
                      {fieldState.invalid && <FieldError errors={[fieldState.error]} />}
                    </FieldContent>
                  </Field>
                )}
              />
            )}
            <NumberField
              form={form}
              name="connectTimeoutSecs"
              label="Connection timeout"
              unit="sec"
              min={1}
              max={300}
            />
            <Controller
              control={form.control}
              name="readOnly"
              render={({ field }) => (
                <Field orientation="horizontal">
                  <FieldContent>
                    <FieldLabel htmlFor={`${formId}-read-only`}>Read-only</FieldLabel>
                    <FieldDescription>
                      The server rejects every change. Turning this off still never lets aidb change
                      data without a preview and your approval.
                    </FieldDescription>
                  </FieldContent>
                  <Switch
                    id={`${formId}-read-only`}
                    checked={field.value}
                    onCheckedChange={field.onChange}
                  />
                </Field>
              )}
            />
            {!readOnly && (
              <Alert variant={environment === 'production' ? 'destructive' : 'default'}>
                <TriangleAlertIcon />
                <AlertTitle>
                  Writes enabled
                  {environment === 'production' ? ' on a production database' : ''}
                </AlertTitle>
                <AlertDescription>
                  Use a database role that can only change what it must; the role's grants are the
                  final safeguard.
                </AlertDescription>
              </Alert>
            )}
          </FieldGroup>
        </FieldSet>

        {report && <TestResult result={report} />}
        {serverError && (
          <Alert variant="destructive">
            <CircleAlertIcon />
            <AlertTitle>The connection was not saved</AlertTitle>
            <AlertDescription>{serverError}</AlertDescription>
          </Alert>
        )}
      </form>
      <SheetFooter className="flex-row flex-wrap border-t">
        <Button
          type="button"
          variant="outline"
          className="min-h-11 sm:min-h-0"
          disabled={busy}
          onClick={() => void onTest()}
        >
          {test.isPending && <Spinner data-icon="inline-start" />}
          Test connection
        </Button>
        <div className="ms-auto flex gap-2">
          <Button type="button" variant="ghost" className="min-h-11 sm:min-h-0" onClick={onDone}>
            Cancel
          </Button>
          <Button type="submit" form={formId} className="min-h-11 sm:min-h-0" disabled={busy}>
            {save.isPending && <Spinner data-icon="inline-start" />}
            {editing ? 'Save changes' : 'Add connection'}
          </Button>
        </div>
      </SheetFooter>
    </>
  )
}

/** The profile was saved, but storing its password failed. */
class SavedWithoutPassword extends Error {
  constructor(cause: unknown) {
    super(cause instanceof Error ? cause.message : String(cause))
    this.name = 'SavedWithoutPassword'
  }
}

function UriField({
  onParsed,
}: {
  onParsed: (parsed: Awaited<ReturnType<typeof parseConnectionUri>>) => string[]
}) {
  const id = useId()
  const [uri, setUri] = useState('')
  const [notes, setNotes] = useState<string[]>([])
  const parse = useMutation({
    mutationFn: parseConnectionUri,
    onSuccess: (parsed) => {
      setNotes(onParsed(parsed))
      // The URI can hold a password; keep it only in the fields it was split into.
      setUri('')
    },
  })
  return (
    <Field data-invalid={parse.isError}>
      <FieldLabel htmlFor={id}>Paste a connection URI (optional)</FieldLabel>
      <InputGroup className="h-11 sm:h-9">
        <InputGroupAddon>
          <LinkIcon />
        </InputGroupAddon>
        <InputGroupInput
          id={id}
          type="password"
          autoComplete="off"
          spellCheck={false}
          placeholder="postgres://user@host:5432/database"
          value={uri}
          aria-invalid={parse.isError}
          onChange={(event) => setUri(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') {
              event.preventDefault()
              if (uri.trim()) parse.mutate(uri)
            }
          }}
        />
        <InputGroupAddon align="inline-end">
          <InputGroupButton
            disabled={!uri.trim() || parse.isPending}
            onClick={() => parse.mutate(uri)}
          >
            {parse.isPending && <Spinner />}
            Fill in
          </InputGroupButton>
        </InputGroupAddon>
      </InputGroup>
      <FieldDescription>
        The fields below are filled from it. The URI itself is not stored.
      </FieldDescription>
      {parse.isError && <FieldError>{parse.error.message}</FieldError>}
      {notes.length > 0 && (
        <Alert>
          <InfoIcon />
          <AlertDescription>
            {notes.map((note) => (
              <p key={note}>{note}</p>
            ))}
          </AlertDescription>
        </Alert>
      )}
    </Field>
  )
}

type Form = ReturnType<typeof useForm<ConnectionFormValues>>
type TextName = Extract<
  Path<ConnectionFormValues>,
  'name' | 'host' | 'database' | 'username' | 'caCertPath'
>
type NumberName = Extract<Path<ConnectionFormValues>, 'port' | 'connectTimeoutSecs'>

function TextField({
  form,
  name,
  label,
  placeholder,
  description,
  mono,
  autoComplete = 'off',
}: {
  form: Form
  name: TextName
  label: string
  placeholder?: string
  description?: ReactNode
  mono?: boolean
  autoComplete?: string
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
            autoComplete={autoComplete}
            spellCheck={false}
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

function NumberField({
  form,
  name,
  label,
  unit,
  min,
  max,
}: {
  form: Form
  name: NumberName
  label: string
  unit?: string
  min: number
  max: number
}) {
  const id = useId()
  return (
    <Controller
      control={form.control}
      name={name}
      render={({ field, fieldState }) => (
        <Field data-invalid={fieldState.invalid}>
          <FieldLabel htmlFor={id}>{label}</FieldLabel>
          <InputGroup className="h-11 sm:h-9">
            <InputGroupInput
              id={id}
              name={field.name}
              ref={field.ref}
              type="number"
              inputMode="numeric"
              min={min}
              max={max}
              step={1}
              value={Number.isNaN(field.value) ? '' : field.value}
              onChange={(event) => field.onChange(event.target.valueAsNumber)}
              onBlur={field.onBlur}
              aria-invalid={fieldState.invalid}
              className="[appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none"
            />
            {unit && (
              <InputGroupAddon align="inline-end">
                <InputGroupText>{unit}</InputGroupText>
              </InputGroupAddon>
            )}
          </InputGroup>
          {fieldState.invalid && <FieldError errors={[fieldState.error]} />}
        </Field>
      )}
    />
  )
}

function PasswordField({
  form,
  hasSavedPassword,
  removing,
  onRemove,
}: {
  form: Form
  hasSavedPassword: boolean
  removing: boolean
  onRemove: () => void
}) {
  const id = useId()
  const rememberId = useId()
  const [visible, setVisible] = useState(false)
  return (
    <FieldGroup className="gap-3">
      <Controller
        control={form.control}
        name="password"
        render={({ field }) => (
          <Field>
            <FieldLabel htmlFor={id}>Password</FieldLabel>
            <InputGroup className="h-11 sm:h-9">
              <InputGroupInput
                {...field}
                id={id}
                type={visible ? 'text' : 'password'}
                autoComplete="new-password"
                spellCheck={false}
                placeholder={hasSavedPassword ? 'Saved; leave blank to keep it' : undefined}
              />
              <InputGroupAddon align="inline-end">
                <InputGroupButton
                  size="icon-xs"
                  aria-label={visible ? 'Hide password' : 'Show password'}
                  aria-pressed={visible}
                  onClick={() => setVisible((shown) => !shown)}
                >
                  {visible ? <EyeOffIcon /> : <EyeIcon />}
                </InputGroupButton>
              </InputGroupAddon>
            </InputGroup>
          </Field>
        )}
      />
      <Controller
        control={form.control}
        name="rememberPassword"
        render={({ field }) => (
          <Field orientation="horizontal">
            <Checkbox
              id={rememberId}
              checked={field.value}
              onCheckedChange={(checked) => field.onChange(checked === true)}
            />
            <FieldContent>
              <FieldLabel htmlFor={rememberId} className="font-normal">
                Remember the password
              </FieldLabel>
              <FieldDescription>
                In the system keychain, or until aidb quits if no keychain is available.
              </FieldDescription>
            </FieldContent>
          </Field>
        )}
      />
      {hasSavedPassword && (
        <Button
          type="button"
          variant="link"
          size="sm"
          className="min-h-11 self-start px-0 sm:min-h-0"
          disabled={removing}
          onClick={onRemove}
        >
          Remove the saved password
        </Button>
      )}
    </FieldGroup>
  )
}

function TestResult({
  result,
}: {
  result: { ok: true; report: ConnectionTestReport } | { ok: false; message: string }
}) {
  if (!result.ok) {
    return (
      <Alert variant="destructive" role="status">
        <CircleAlertIcon />
        <AlertTitle>Could not connect</AlertTitle>
        <AlertDescription>{result.message}</AlertDescription>
      </Alert>
    )
  }
  const { server, roundTripMs } = result.report
  const details = [
    `PostgreSQL ${server.version}`,
    `user ${server.user}`,
    server.encrypted ? `encrypted (${server.tlsVersion ?? 'TLS'})` : 'not encrypted',
    server.readOnly ? 'read-only' : 'writes allowed',
    `${roundTripMs} ms`,
  ]
  return (
    <Alert role="status">
      <CircleCheckIcon />
      <AlertTitle>Connection works</AlertTitle>
      <AlertDescription>
        <p>{details.join(' · ')}</p>
        {!server.supportedVersion && (
          <p>aidb is tested with PostgreSQL 15 to 18; some features may not work as expected.</p>
        )}
        {server.inRecovery && <p>This server is a standby, so it accepts no changes.</p>}
      </AlertDescription>
    </Alert>
  )
}
