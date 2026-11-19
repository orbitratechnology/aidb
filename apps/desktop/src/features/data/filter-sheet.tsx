import { useId } from 'react'
import { zodResolver } from '@hookform/resolvers/zod'
import {
  TABLE_DATA_LIMITS,
  type Filter,
  type FilterOperator,
  type PageColumn,
  type SortKey,
} from '@aidb/contracts'
import { PlusIcon, Trash2Icon } from 'lucide-react'
import { Controller, useFieldArray, useForm, useWatch, type UseFormReturn } from 'react-hook-form'
import { Button } from '@/components/ui/button'
import {
  Field,
  FieldDescription,
  FieldError,
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
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import {
  filterFormSchema,
  fromFormValues,
  newFilter,
  NO_SORT,
  toFormValues,
  type FilterFormValues,
} from './filter-form'
import { canSort, OPERATOR_LABELS, operatorsFor, takesValue } from './table-view'

type Props = {
  open: boolean
  onOpenChange: (open: boolean) => void
  columns: PageColumn[]
  filters: Filter[]
  sort: SortKey[]
  onApply: (next: { filters: Filter[]; sort: SortKey[] }) => void
}

/** Filters and sort order, edited together and applied at once. */
export function FilterSheet({ open, onOpenChange, ...props }: Props) {
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        side="right"
        className="gap-0 data-[side=right]:w-full data-[side=right]:sm:max-w-md"
      >
        <SheetHeader className="border-b">
          <SheetTitle>Filter and sort</SheetTitle>
          <SheetDescription>
            Rows must match every filter. Values go to the database as parameters, never as SQL.
          </SheetDescription>
        </SheetHeader>
        {/* Mounted only while open, so it starts from the applied filters each time. */}
        {open && <FilterForm {...props} onDone={() => onOpenChange(false)} />}
      </SheetContent>
    </Sheet>
  )
}

function FilterForm({
  columns,
  filters,
  sort,
  onApply,
  onDone,
}: Omit<Props, 'open' | 'onOpenChange'> & { onDone: () => void }) {
  const formId = useId()
  const form = useForm<FilterFormValues>({
    resolver: zodResolver(filterFormSchema),
    defaultValues: toFormValues(filters, sort),
  })
  const rows = useFieldArray({ control: form.control, name: 'filters' })
  const sortColumn = useWatch({ control: form.control, name: 'sortColumn' })
  const sortable = columns.filter(canSort)

  const submit = form.handleSubmit((values) => {
    onApply(fromFormValues(values))
    onDone()
  })

  return (
    <form id={formId} onSubmit={submit} className="flex min-h-0 flex-1 flex-col">
      <div className="flex min-h-0 flex-1 flex-col gap-8 overflow-y-auto p-4">
        <FieldSet>
          <FieldLegend>Filters</FieldLegend>
          {rows.fields.length === 0 && (
            <FieldDescription>No filters: every row is shown.</FieldDescription>
          )}
          <FieldGroup className="gap-6">
            {rows.fields.map((row, index) => (
              <FilterRow
                key={row.id}
                form={form}
                index={index}
                columns={columns}
                onRemove={() => rows.remove(index)}
              />
            ))}
          </FieldGroup>
          <Button
            type="button"
            variant="outline"
            className="min-h-11 self-start sm:min-h-0"
            disabled={columns.length === 0 || rows.fields.length >= TABLE_DATA_LIMITS.maxFilters}
            onClick={() => rows.append(newFilter(columns))}
          >
            <PlusIcon data-icon="inline-start" />
            Add filter
          </Button>
        </FieldSet>

        <FieldSet>
          <FieldLegend>Sort</FieldLegend>
          {sort.length > 1 && (
            <FieldDescription>
              Rows are sorted by {sort.length} columns. Choosing here replaces that order.
            </FieldDescription>
          )}
          <div className="grid gap-4 sm:grid-cols-[1fr_9rem]">
            <Controller
              control={form.control}
              name="sortColumn"
              render={({ field }) => (
                <Field>
                  <FieldLabel htmlFor={`${formId}-sort`}>Column</FieldLabel>
                  <Select value={field.value} onValueChange={field.onChange}>
                    <SelectTrigger id={`${formId}-sort`} className="min-h-11 w-full sm:min-h-0">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value={NO_SORT}>Primary key order</SelectItem>
                      {sortable.map((column) => (
                        <SelectItem key={column.name} value={column.name}>
                          {column.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </Field>
              )}
            />
            <Controller
              control={form.control}
              name="sortDirection"
              render={({ field }) => (
                <Field>
                  <FieldLabel htmlFor={`${formId}-direction`}>Direction</FieldLabel>
                  <Select
                    value={field.value}
                    onValueChange={field.onChange}
                    disabled={sortColumn === NO_SORT}
                  >
                    <SelectTrigger
                      id={`${formId}-direction`}
                      className="min-h-11 w-full sm:min-h-0"
                    >
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="asc">Ascending</SelectItem>
                      <SelectItem value="desc">Descending</SelectItem>
                    </SelectContent>
                  </Select>
                </Field>
              )}
            />
          </div>
        </FieldSet>
      </div>
      <SheetFooter className="flex-row border-t">
        <Button type="submit" className="min-h-11 flex-1 sm:min-h-0 sm:flex-none">
          Apply
        </Button>
        <Button
          type="button"
          variant="outline"
          className="min-h-11 flex-1 sm:min-h-0 sm:flex-none"
          onClick={() => {
            onApply({ filters: [], sort: [] })
            onDone()
          }}
        >
          Clear all
        </Button>
      </SheetFooter>
    </form>
  )
}

function FilterRow({
  form,
  index,
  columns,
  onRemove,
}: {
  form: UseFormReturn<FilterFormValues>
  index: number
  columns: PageColumn[]
  onRemove: () => void
}) {
  const id = useId()
  const [columnName, operator] = useWatch({
    control: form.control,
    name: [`filters.${index}.column`, `filters.${index}.operator`],
  })
  const column = columns.find((c) => c.name === columnName)
  const operators = operatorsFor(column?.kind ?? 'other')

  return (
    <div
      role="group"
      aria-label={`Filter ${index + 1}`}
      className="grid gap-3 rounded-md border p-3 sm:grid-cols-2"
    >
      <Controller
        control={form.control}
        name={`filters.${index}.column`}
        render={({ field }) => (
          <Field>
            <FieldLabel htmlFor={`${id}-column`}>Column</FieldLabel>
            <Select
              value={field.value}
              onValueChange={(name) => {
                field.onChange(name)
                const kind = columns.find((c) => c.name === name)?.kind ?? 'other'
                if (!operatorsFor(kind).includes(operator)) {
                  form.setValue(`filters.${index}.operator`, operatorsFor(kind)[0] ?? 'is_null')
                }
              }}
            >
              <SelectTrigger id={`${id}-column`} className="min-h-11 w-full sm:min-h-0">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {columns.map((c) => (
                  <SelectItem key={c.name} value={c.name}>
                    {c.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>
        )}
      />
      <Controller
        control={form.control}
        name={`filters.${index}.operator`}
        render={({ field }) => (
          <Field>
            <FieldLabel htmlFor={`${id}-operator`}>Condition</FieldLabel>
            <Select
              value={field.value}
              onValueChange={(value) => field.onChange(value as FilterOperator)}
            >
              <SelectTrigger id={`${id}-operator`} className="min-h-11 w-full sm:min-h-0">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {operators.map((op) => (
                  <SelectItem key={op} value={op}>
                    {OPERATOR_LABELS[op]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </Field>
        )}
      />
      <Controller
        control={form.control}
        name={`filters.${index}.value`}
        render={({ field, fieldState }) => (
          <Field data-invalid={fieldState.invalid} className="sm:col-span-2">
            <FieldLabel htmlFor={`${id}-value`}>Value</FieldLabel>
            <Input
              {...field}
              id={`${id}-value`}
              disabled={!takesValue(operator)}
              aria-invalid={fieldState.invalid}
              placeholder={takesValue(operator) ? column?.dataType : 'Not needed'}
              autoComplete="off"
              spellCheck={false}
              className="min-h-11 sm:min-h-0"
            />
            {fieldState.invalid && <FieldError errors={[fieldState.error]} />}
          </Field>
        )}
      />
      <Button
        type="button"
        variant="ghost"
        className="min-h-11 justify-self-start sm:min-h-0"
        onClick={onRemove}
      >
        <Trash2Icon data-icon="inline-start" />
        Remove filter
      </Button>
    </div>
  )
}
