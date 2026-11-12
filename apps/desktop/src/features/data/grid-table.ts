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

/** Column ids are the column names, which are unique within a table. */
export function gridColumns(columns: PageColumn[]): Array<ColumnDef<GridFeatures, GridRow>> {
  return columns.map((column, index) => ({
    id: column.name,
    header: column.name,
    accessorFn: (row: GridRow) => row[index],
    size: defaultColumnWidth(column.kind),
    minSize: 60,
    maxSize: 1200,
    enableSorting: canSort(column),
    sortDescFirst: false,
  }))
}
