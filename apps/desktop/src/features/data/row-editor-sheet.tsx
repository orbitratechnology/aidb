import { useId, useState, type FormEvent } from 'react'
import type { PageColumn } from '@aidb/contracts'
import { LockIcon } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Field, FieldDescription, FieldGroup, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
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
import { Textarea } from '@/components/ui/textarea'
import { CellValue } from './cell-value'
import type { Values } from './edits'

/** A column as the editor shows it. */
export type EditorColumn = {
  column: PageColumn
  nullable: boolean
  /** Why it cannot be given a value here, or `null`. */
  readOnly: string | null
}

/** What is being edited: a loaded row, or a row to add. */
export type EditorTarget =
  | {
      mode: 'edit'
      /** "id = 7". */
      label: string
      /** The row's values, with any pending edits applied. */
      values: Values
    }
  | {
      mode: 'new'
      id: string
      label: string
      /** Values already given; other columns get their defaults. */
      values: Values
    }

type FieldState = { mode: 'value' | 'null' | 'default'; text: string }

const MULTILINE = new Set(['text', 'json', 'binary', 'other'])

function initialFields(target: EditorTarget, columns: EditorColumn[]): Record<string, FieldState> {
  const fields: Record<string, FieldState> = {}
  for (const { column } of columns) {
    if (!(column.name in target.values)) {
      fields[column.name] = { mode: target.mode === 'new' ? 'default' : 'value', text: '' }
      continue
    }
    const value = target.values[column.name] ?? null
    fields[column.name] =
      value === null ? { mode: 'null', text: '' } : { mode: 'value', text: value }
  }
  return fields
}

/**
 * Edits one row, or a new one, field by field. Nothing is written: "Keep changes" adds them to
 * the pending changes, which are reviewed and saved together.
 */
export function RowEditorSheet({
  target,
  source,
  columns,
  onKeep,
  onClose,
}: {
  target: EditorTarget | null
  /** What the row belongs to, such as `public.orders`. */
  source: string
  columns: EditorColumn[]
  onKeep: (values: Values) => void
  onClose: () => void
}) {
  return (
    <Sheet open={target !== null} onOpenChange={(open) => !open && onClose()}>
      <SheetContent
        side="right"
        className="gap-0 data-[side=right]:w-full data-[side=right]:sm:max-w-lg"
      >
        <SheetHeader className="border-b">
          <SheetTitle>
            {target?.mode === 'new' ? 'New row' : `Edit ${target?.label ?? 'row'}`}
          </SheetTitle>
          <SheetDescription>
            {source}. Changes stay pending until you review and save them.
          </SheetDescription>
        </SheetHeader>
        {/* Mounted per row, so each edit starts from that row's values. */}
        {target && (
          <EditorForm
            key={target.mode === 'new' ? target.id : target.label}
            target={target}
            columns={columns}
            onKeep={onKeep}
            onClose={onClose}
          />
        )}
      </SheetContent>
    </Sheet>
  )
}

function EditorForm({
  target,
  columns,
  onKeep,
  onClose,
}: {
  target: EditorTarget
  columns: EditorColumn[]
  onKeep: (values: Values) => void
  onClose: () => void
}) {
  const formId = useId()
  const [fields, setFields] = useState(() => initialFields(target, columns))
  const editable = columns.filter((c) => c.readOnly === null)
  const setField = (name: string, next: Partial<FieldState>) =>
    setFields((current) => ({
      ...current,
      [name]: { ...(current[name] ?? { mode: 'value', text: '' }), ...next },
    }))

  const submit = (event: FormEvent) => {
    event.preventDefault()
    const values: Values = {}
    for (const { column } of editable) {
      const field = fields[column.name]
      if (!field || field.mode === 'default') continue
      values[column.name] = field.mode === 'null' ? null : field.text
    }
    onKeep(values)
  }

  return (
    <form id={formId} onSubmit={submit} className="flex min-h-0 flex-1 flex-col">
      <FieldGroup className="min-h-0 flex-1 gap-5 overflow-y-auto p-4">
        {columns.map((entry) => (
          <ColumnField
            key={entry.column.name}
            entry={entry}
            isNew={target.mode === 'new'}
            field={fields[entry.column.name] ?? { mode: 'value', text: '' }}
            current={target.values[entry.column.name]}
            onChange={(next) => setField(entry.column.name, next)}
          />
        ))}
      </FieldGroup>
      <SheetFooter className="flex-row border-t">
        <Button type="submit" className="min-h-11 flex-1 sm:min-h-0 sm:flex-none">
          Keep changes
        </Button>
        <Button
          type="button"
          variant="outline"
          className="min-h-11 flex-1 sm:min-h-0 sm:flex-none"
          onClick={onClose}
        >
          Cancel
        </Button>
      </SheetFooter>
    </form>
  )
}

function ColumnField({
  entry,
  isNew,
  field,
  current,
  onChange,
}: {
  entry: EditorColumn
  isNew: boolean
  field: FieldState
  current: string | null | undefined
  onChange: (next: Partial<FieldState>) => void
}) {
  const id = useId()
  const { column, nullable, readOnly } = entry
  const label = (
    <FieldLabel htmlFor={readOnly ? undefined : id} className="flex-wrap gap-x-2">
      <span className="break-all">{column.name}</span>
      <span className="font-mono text-xs font-normal text-muted-foreground">{column.dataType}</span>
    </FieldLabel>
  )

  if (readOnly) {
    return (
      <Field>
        {label}
        {!isNew && current !== undefined && (
          <CellValue value={current} kind={column.kind} className="text-sm" />
        )}
        <FieldDescription className="flex items-start gap-1.5">
          <LockIcon aria-hidden className="mt-0.5 size-3.5 shrink-0" />
          {readOnly}
        </FieldDescription>
      </Field>
    )
  }

  // Typing (or choosing) a value leaves NULL or the default; the checkboxes go back to them.
  const shown = field.mode === 'value' ? field.text : ''
  const placeholder = field.mode === 'null' ? 'NULL' : field.mode === 'default' ? 'Default' : ''
  const type = (text: string) => onChange({ text, mode: 'value' })
  const control =
    column.kind === 'boolean' ? (
      <Select value={shown} onValueChange={type}>
        <SelectTrigger id={id} className="min-h-11 w-full sm:min-h-0">
          <SelectValue placeholder={placeholder || 'Choose'} />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="true">true</SelectItem>
          <SelectItem value="false">false</SelectItem>
        </SelectContent>
      </Select>
    ) : MULTILINE.has(column.kind) ? (
      <Textarea
        id={id}
        value={shown}
        placeholder={placeholder}
        spellCheck={column.kind === 'text'}
        className={column.kind === 'text' ? undefined : 'font-mono text-xs md:text-xs'}
        onChange={(event) => type(event.target.value)}
      />
    ) : (
      <Input
        id={id}
        value={shown}
        placeholder={placeholder}
        inputMode={column.kind === 'number' ? 'decimal' : undefined}
        className="min-h-11 sm:min-h-0"
        onChange={(event) => type(event.target.value)}
      />
    )

  return (
    <Field>
      {label}
      {control}
      <div className="flex flex-wrap gap-x-4 gap-y-2">
        {isNew && (
          <ModeCheckbox
            label="Use the default"
            checked={field.mode === 'default'}
            onChange={(checked) => onChange({ mode: checked ? 'default' : 'value' })}
          />
        )}
        {nullable && (
          <ModeCheckbox
            label="NULL"
            checked={field.mode === 'null'}
            onChange={(checked) => onChange({ mode: checked ? 'null' : 'value' })}
          />
        )}
      </div>
    </Field>
  )
}

function ModeCheckbox({
  label,
  checked,
  onChange,
}: {
  label: string
  checked: boolean
  onChange: (checked: boolean) => void
}) {
  const id = useId()
  return (
    <Field orientation="horizontal" className="w-auto">
      <Checkbox
        id={id}
        checked={checked}
        onCheckedChange={(value) => onChange(value === true)}
        className="max-sm:size-5"
      />
      <FieldLabel htmlFor={id} className="font-normal">
        {label}
      </FieldLabel>
    </Field>
  )
}
