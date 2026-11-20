import { useEffect, useId, useRef, useState } from 'react'
import type { SchemaDiff, SchemaFile } from '@aidb/contracts'
import { useMutation } from '@tanstack/react-query'
import { CircleAlertIcon, FileCode2Icon, FileDiffIcon, GitCompareArrowsIcon } from 'lucide-react'
import { PageContainer } from '@/components/shared/page'
import { RequireConnection } from '@/components/shared/status-states'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldGroup,
  FieldLabel,
  FieldLegend,
  FieldSet,
} from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Spinner } from '@/components/ui/spinner'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { useConnections } from '@/features/connections/queries'
import { useActiveConnection } from '@/features/connections/sessions'
import type { ActiveConnection } from '@/features/connections/types'
import { useCatalog } from '@/features/data/queries'
import { closeSchemaFile, compareSchemas, openSchemaFile } from '@/lib/commands'
import { errorMessage } from '@/lib/errors'
import { compareRequest, defaultSchema, needsPassword, type CompareForm } from './compare'
import { DiffView } from './diff-view'

export function MigrationsPage() {
  return (
    <PageContainer>
      <header className="flex flex-col gap-1">
        <h1 className="text-lg font-semibold">Migrations</h1>
        <p className="text-sm text-muted-foreground">
          Compare a schema on this connection with another schema or a .sql file, and get the SQL
          that makes it match, with what each step risks. Nothing runs here.
        </p>
      </header>
      <RequireConnection
        icon={FileDiffIcon}
        title="Compare schemas"
        description="Connect to a database to compare one of its schemas with another schema or a .sql file."
      >
        <CompareWithActive />
      </RequireConnection>
    </PageContainer>
  )
}

function CompareWithActive() {
  const active = useActiveConnection()
  // Mounted again per connection, so nothing chosen for one carries over to another.
  return active ? <Compare key={active.id} active={active} /> : null
}

function Compare({ active }: { active: ActiveConnection }) {
  const formId = useId()
  const catalog = useCatalog(active.id)
  const connections = useConnections()
  const schemas = (catalog.data?.schemas ?? [])
    .filter((schema) => schema.accessible)
    .map((schema) => schema.name)

  const [form, setForm] = useState<CompareForm>({
    targetSchema: '',
    sourceKind: 'schema',
    sourceConnection: active.id,
    sourceSchema: '',
    file: null,
    fileSchema: '',
    concurrentIndexes: false,
  })
  const [password, setPassword] = useState('')
  const [diff, setDiff] = useState<SchemaDiff | null>(null)

  // Defaults once the schemas are known: change public, compare with another one.
  const targetSchema = form.targetSchema || defaultSchema(schemas)
  const sourceSchema =
    form.sourceSchema ||
    (form.sourceConnection === active.id ? defaultSchema(schemas, targetSchema) : 'public')
  const current = { ...form, targetSchema, sourceSchema }
  const update = (patch: Partial<CompareForm>) => {
    setForm((previous) => ({ ...previous, ...patch }))
    setDiff(null)
  }

  // The file stays in the core until another is chosen or the page closes.
  const fileId = useRef<string | null>(null)
  useEffect(
    () => () => {
      if (fileId.current) closeSchemaFile(fileId.current).catch(() => undefined)
    },
    [],
  )
  const chooseFile = useMutation({
    mutationFn: openSchemaFile,
    onSuccess: (file: SchemaFile | null) => {
      if (!file) return
      if (fileId.current) closeSchemaFile(fileId.current).catch(() => undefined)
      fileId.current = file.fileId
      update({
        file,
        fileSchema: file.schemas.includes(targetSchema) ? targetSchema : (file.schemas[0] ?? ''),
      })
    },
  })

  const sourceConnection = connections.data?.find((c) => c.id === form.sourceConnection)
  const askPassword = needsPassword(sourceConnection, active.id)
  const built = compareRequest(current, active.id)
  const compare = useMutation({
    mutationFn: () => {
      if (!('request' in built)) throw new Error(built.missing)
      return compareSchemas(active.id, built.request, askPassword ? password : null)
    },
    onSuccess: setDiff,
  })

  return (
    <div className="flex flex-col gap-6">
      <form
        className="flex flex-col gap-6"
        onSubmit={(event) => {
          event.preventDefault()
          compare.mutate()
        }}
      >
        <FieldGroup>
          <Field>
            <FieldLabel htmlFor={`${formId}-target`}>Schema to change</FieldLabel>
            <Select
              value={targetSchema}
              onValueChange={(value) => update({ targetSchema: value })}
              disabled={schemas.length === 0}
            >
              <SelectTrigger id={`${formId}-target`} className="min-h-11 w-full sm:min-h-0">
                <SelectValue placeholder="Loading schemas…" />
              </SelectTrigger>
              <SelectContent>
                {schemas.map((schema) => (
                  <SelectItem key={schema} value={schema}>
                    {schema}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <FieldDescription>
              On {active.name}. The steps change this schema so it matches the one below.
            </FieldDescription>
          </Field>

          <FieldSet>
            <FieldLegend variant="label">Make it match</FieldLegend>
            <ToggleGroup
              type="single"
              variant="outline"
              className="w-full sm:w-auto"
              value={form.sourceKind}
              onValueChange={(value) =>
                value && update({ sourceKind: value as CompareForm['sourceKind'] })
              }
              aria-label="What to compare with"
            >
              <ToggleGroupItem value="schema" className="min-h-11 flex-1 sm:min-h-0">
                <GitCompareArrowsIcon data-icon="inline-start" />
                Another schema
              </ToggleGroupItem>
              <ToggleGroupItem value="file" className="min-h-11 flex-1 sm:min-h-0">
                <FileCode2Icon data-icon="inline-start" />A .sql file
              </ToggleGroupItem>
            </ToggleGroup>

            {form.sourceKind === 'schema' ? (
              <div className="grid gap-4 sm:grid-cols-2">
                <Field>
                  <FieldLabel htmlFor={`${formId}-connection`}>Connection</FieldLabel>
                  <Select
                    value={form.sourceConnection}
                    onValueChange={(value) => update({ sourceConnection: value, sourceSchema: '' })}
                  >
                    <SelectTrigger
                      id={`${formId}-connection`}
                      className="min-h-11 w-full sm:min-h-0"
                    >
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {(connections.data ?? []).map((connection) => (
                        <SelectItem key={connection.id} value={connection.id}>
                          {connection.id === active.id
                            ? `${connection.name} (this connection)`
                            : connection.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  {form.sourceConnection !== active.id && (
                    <FieldDescription>
                      Read once, read-only, through a connection of its own that closes right after.
                    </FieldDescription>
                  )}
                </Field>
                <Field>
                  <FieldLabel htmlFor={`${formId}-source`}>Schema</FieldLabel>
                  {form.sourceConnection === active.id ? (
                    <Select
                      value={sourceSchema}
                      onValueChange={(value) => update({ sourceSchema: value })}
                    >
                      <SelectTrigger id={`${formId}-source`} className="min-h-11 w-full sm:min-h-0">
                        <SelectValue placeholder="No other schema" />
                      </SelectTrigger>
                      <SelectContent>
                        {schemas
                          .filter((schema) => schema !== targetSchema)
                          .map((schema) => (
                            <SelectItem key={schema} value={schema}>
                              {schema}
                            </SelectItem>
                          ))}
                      </SelectContent>
                    </Select>
                  ) : (
                    <Input
                      id={`${formId}-source`}
                      value={sourceSchema}
                      autoComplete="off"
                      spellCheck={false}
                      maxLength={63}
                      className="min-h-11 sm:min-h-0"
                      onChange={(event) => update({ sourceSchema: event.target.value })}
                    />
                  )}
                </Field>
                {askPassword && (
                  <Field className="sm:col-span-2">
                    <FieldLabel htmlFor={`${formId}-password`}>
                      Password for {sourceConnection?.name}
                    </FieldLabel>
                    <Input
                      id={`${formId}-password`}
                      type="password"
                      value={password}
                      autoComplete="off"
                      className="min-h-11 sm:min-h-0"
                      onChange={(event) => setPassword(event.target.value)}
                    />
                    <FieldDescription>
                      Its password is not saved. It is used for this comparison only.
                    </FieldDescription>
                  </Field>
                )}
              </div>
            ) : (
              <div className="grid gap-4 sm:grid-cols-2">
                <Field>
                  <FieldLabel htmlFor={`${formId}-file`}>File</FieldLabel>
                  <Button
                    id={`${formId}-file`}
                    type="button"
                    variant="outline"
                    className="min-h-11 justify-start sm:min-h-0"
                    disabled={chooseFile.isPending}
                    onClick={() => chooseFile.mutate()}
                  >
                    {chooseFile.isPending ? (
                      <Spinner data-icon="inline-start" />
                    ) : (
                      <FileCode2Icon data-icon="inline-start" />
                    )}
                    <span className="truncate">{form.file?.fileName ?? 'Choose a .sql file…'}</span>
                  </Button>
                  <FieldDescription>
                    {form.file
                      ? `${form.file.statements} ${form.file.statements === 1 ? 'statement' : 'statements'}. Read here; nothing in it runs.`
                      : 'A schema written as SQL, such as pg_dump --schema-only writes it. Nothing in it runs.'}
                  </FieldDescription>
                </Field>
                <Field>
                  <FieldLabel htmlFor={`${formId}-file-schema`}>Schema in the file</FieldLabel>
                  <Select
                    value={form.fileSchema}
                    onValueChange={(value) => update({ fileSchema: value })}
                    disabled={!form.file || form.file.schemas.length === 0}
                  >
                    <SelectTrigger
                      id={`${formId}-file-schema`}
                      className="min-h-11 w-full sm:min-h-0"
                    >
                      <SelectValue placeholder="Choose a file first" />
                    </SelectTrigger>
                    <SelectContent>
                      {(form.file?.schemas ?? []).map((schema) => (
                        <SelectItem key={schema} value={schema}>
                          {schema}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </Field>
                {chooseFile.isError && (
                  <Alert variant="destructive" className="sm:col-span-2">
                    <CircleAlertIcon />
                    <AlertTitle>The file cannot be compared</AlertTitle>
                    <AlertDescription>{errorMessage(chooseFile.error)}</AlertDescription>
                  </Alert>
                )}
              </div>
            )}
          </FieldSet>

          <Field orientation="horizontal">
            <Checkbox
              id={`${formId}-concurrent`}
              checked={form.concurrentIndexes}
              onCheckedChange={(checked) => update({ concurrentIndexes: checked === true })}
            />
            <FieldContent>
              <FieldLabel htmlFor={`${formId}-concurrent`}>
                Create and drop indexes concurrently
              </FieldLabel>
              <FieldDescription>
                Writes to the table continue while an index builds, but those steps run after the
                transaction and are not rolled back with it.
              </FieldDescription>
            </FieldContent>
          </Field>
        </FieldGroup>

        <div className="flex flex-wrap items-center gap-3">
          <Button
            type="submit"
            className="min-h-11 sm:min-h-0"
            disabled={compare.isPending || !('request' in built)}
          >
            {compare.isPending && <Spinner data-icon="inline-start" />}
            Compare
          </Button>
          {'missing' in built && schemas.length > 0 && (
            <span className="text-sm text-muted-foreground">{built.missing}</span>
          )}
        </div>
      </form>

      {compare.isError && (
        <Alert variant="destructive">
          <CircleAlertIcon />
          <AlertTitle>The schemas could not be compared</AlertTitle>
          <AlertDescription>{errorMessage(compare.error)}</AlertDescription>
        </Alert>
      )}
      {diff && <DiffView diff={diff} environment={active.environment} />}
    </div>
  )
}
