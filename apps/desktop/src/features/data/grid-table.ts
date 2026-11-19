import type { PageColumn } from '@aidb/contracts'
import {
  columnResizingFeature,
  columnSizingFeature,
  columnVisibilityFeature,
  rowSortingFeature,
  tableFeatures,
  type ColumnDef,
} from '@tanstack/react-table'
import { canSort, defaultColumnWidth } from './table-view'

/** Sorting is done by the database (`manualSorting`); no client row models are registered. */
export const gridFeatures = tableFeatures({
  columnVisibilityFeature,
  columnSizingFeature,
  columnResizingFeature,
  rowSortingFeature,
})

export type GridFeatures = typeof gridFeatures

/** One row of a page: values as text, `null` for SQL NULL, in the page's column order. */
export type GridRow = Array<string | null>

export const EMPTY_ROWS: GridRow[] = []

/**
 * Column ids: the column names, unique within a table. Query results can repeat a name, so later
 * duplicates get their position appended (`a (2)`).
 */
export function columnIds(columns: PageColumn[]): string[] {
  const seen = new Set<string>()
  return columns.map((column, index) => {
    const id = seen.has(column.name) ? `${column.name} (${index + 1})` : column.name
    seen.add(id)
    return id
  })
}

/** Column definitions for the grid. `sortable: false` turns header sorting off. */
export function gridColumns(
  columns: PageColumn[],
  { sortable = true }: { sortable?: boolean } = {},
): Array<ColumnDef<GridFeatures, GridRow>> {
  const ids = columnIds(columns)
  return columns.map((column, index) => ({
    id: ids[index] ?? column.name,
    header: column.name,
    accessorFn: (row: GridRow) => row[index],
    size: defaultColumnWidth(column.kind),
    minSize: 60,
    maxSize: 1200,
    enableSorting: sortable && canSort(column),
    sortDescFirst: false,
  }))
}
