import type {
  CompareRequest,
  ConnectionDetails,
  DiffSummary,
  MigrationStep,
  NoteKind,
  SchemaFile,
} from '@aidb/contracts'

/** What the compare form holds. */
export type CompareForm = {
  targetSchema: string
  sourceKind: 'schema' | 'file'
  /** The connection the source schema is on: the open one by default. */
  sourceConnection: string
  sourceSchema: string
  file: SchemaFile | null
  fileSchema: string
  concurrentIndexes: boolean
}

/** The request for the core, or what is missing for one. */
export function compareRequest(
  form: CompareForm,
  activeId: string,
): { request: CompareRequest } | { missing: string } {
  const options = { concurrentIndexes: form.concurrentIndexes }
  if (!form.targetSchema) return { missing: 'Choose the schema to change.' }
  if (form.sourceKind === 'file') {
    if (!form.file) return { missing: 'Choose a .sql file.' }
    if (!form.fileSchema) return { missing: 'Choose the schema in the file.' }
    return {
      request: {
        targetSchema: form.targetSchema,
        source: { kind: 'file', fileId: form.file.fileId, schema: form.fileSchema },
        options,
      },
    }
  }
  const schema = form.sourceSchema.trim()
  if (!schema) return { missing: 'Name the schema to compare with.' }
  if (form.sourceConnection === activeId) {
    if (schema === form.targetSchema) {
      return { missing: 'Choose another schema: this is the one being changed.' }
    }
    return {
      request: { targetSchema: form.targetSchema, source: { kind: 'schema', schema }, options },
    }
  }
  return {
    request: {
      targetSchema: form.targetSchema,
      source: { kind: 'connection', connectionId: form.sourceConnection, schema },
      options,
    },
  }
}

/** Whether reading the source connection needs a password typed here. */
export function needsPassword(
  connection: ConnectionDetails | undefined,
  activeId: string,
): boolean {
  return (
    connection !== undefined && connection.id !== activeId && connection.password === 'not_saved'
  )
}

/** "public", or the first schema, as the default choice. */
export function defaultSchema(schemas: readonly string[], avoid?: string): string {
  const choices = schemas.filter((schema) => schema !== avoid)
  return choices.includes('public') ? 'public' : (choices[0] ?? '')
}

const plural = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`

/** "1 table added", "2 views changed": the parts of a comparison that are not zero. */
export function summaryParts(summary: DiffSummary): string[] {
  return [
    [summary.tablesAdded, 'table added', 'tables added'],
    [summary.tablesChanged, 'table changed', 'tables changed'],
    [summary.tablesRemoved, 'table removed', 'tables removed'],
    [summary.viewsAdded, 'view added', 'views added'],
    [summary.viewsChanged, 'view changed', 'views changed'],
    [summary.viewsRemoved, 'view removed', 'views removed'],
  ]
    .filter(([count]) => (count as number) > 0)
    .map(([count, one, many]) => plural(count as number, one as string, many as string))
}

export const NOTE_LABELS: Record<NoteKind, string> = {
  data_loss: 'Data loss',
  lock: 'Locks',
  may_fail: 'May fail',
  non_transactional: 'Outside the transaction',
}

/** Steps with a note of each kind, for the warnings above the steps. */
export function stepsWith(steps: readonly MigrationStep[], kind: NoteKind): MigrationStep[] {
  return steps.filter((step) => step.notes.some((note) => note.kind === kind))
}
