import {
  FilterOperatorSchema,
  SortDirectionSchema,
  TABLE_DATA_LIMITS,
  type Filter,
  type PageColumn,
  type SortKey,
} from '@aidb/contracts'
import { z } from 'zod'
import { operatorsFor, takesValue } from './table-view'

/** Select value for "no sort". Column names cannot contain NUL, so it never clashes. */
export const NO_SORT = '\u0000none'

export const filterFormSchema = z
  .object({
    filters: z
      .array(
        z.object({
          column: z.string().min(1, 'Choose a column.'),
          operator: FilterOperatorSchema,
          value: z
            .string()
            .max(
              TABLE_DATA_LIMITS.maxFilterValueChars,
              `Use at most ${TABLE_DATA_LIMITS.maxFilterValueChars.toLocaleString('en-US')} characters.`,
            ),
        }),
      )
      .max(TABLE_DATA_LIMITS.maxFilters),
    sortColumn: z.string(),
    sortDirection: SortDirectionSchema,
  })
  .superRefine((form, context) => {
    form.filters.forEach((filter, index) => {
      if (takesValue(filter.operator) && filter.value === '') {
        context.addIssue({
          code: 'custom',
          path: ['filters', index, 'value'],
          message: 'Enter a value.',
        })
      }
    })
  })

export type FilterFormValues = z.infer<typeof filterFormSchema>

export function toFormValues(filters: Filter[], sort: SortKey[]): FilterFormValues {
  const [first] = sort
  return {
    filters: filters.map(({ column, operator, value }) => ({
      column,
      operator,
      value: value ?? '',
    })),
    sortColumn: first?.column ?? NO_SORT,
    sortDirection: first?.direction ?? 'asc',
  }
}

/** The request parts the form describes. Values of the null checks are dropped. */
export function fromFormValues(values: FilterFormValues): { filters: Filter[]; sort: SortKey[] } {
  return {
    filters: values.filters.map(({ column, operator, value }) =>
      takesValue(operator) ? { column, operator, value } : { column, operator },
    ),
    sort:
      values.sortColumn === NO_SORT
        ? []
        : [{ column: values.sortColumn, direction: values.sortDirection }],
  }
}

/** A new filter row on the first column, with an operator that suits it. */
export function newFilter(columns: PageColumn[]): FilterFormValues['filters'][number] {
  const [first] = columns
  const operator = first ? (operatorsFor(first.kind)[0] ?? 'is_not_null') : 'is_not_null'
  return { column: first?.name ?? '', operator, value: '' }
}
