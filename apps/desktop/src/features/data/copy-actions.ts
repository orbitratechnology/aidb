import type { TablePage } from '@aidb/contracts'
import { toast } from 'sonner'
import { copyText } from '@/lib/clipboard'
import { rowAsJson } from './copy'

const CUT_HINT = 'Export the table to get complete values.'

function failed(error: unknown) {
  toast.error('Could not copy', {
    description: error instanceof Error ? error.message : String(error),
  })
}

/** Copies one row of the page as a JSON object. */
export async function copyRow(page: TablePage, row: number, truncated: Set<string>) {
  const values = page.rows[row]
  if (!values) return
  const cut = (column: number) => truncated.has(`${row}:${column}`)
  try {
    await copyText(rowAsJson(page.columns, values, cut))
    const anyCut = page.columns.some((_, column) => cut(column))
    toast.success(`Copied row ${page.offset + row + 1} as JSON`, {
      description: anyCut
        ? `Some values were cut at ${page.cellCharLimit.toLocaleString()} characters and are copied as loaded. ${CUT_HINT}`
        : undefined,
    })
  } catch (error) {
    failed(error)
  }
}

/** Copies one value as text. */
export async function copyCell(page: TablePage, row: number, column: number, cut: boolean) {
  const value = page.rows[row]?.[column]
  const name = page.columns[column]?.name ?? ''
  if (value === null || value === undefined) return
  try {
    await copyText(value)
    toast.success(
      cut
        ? `Copied the first ${page.cellCharLimit.toLocaleString()} characters of ${name}`
        : `Copied ${name}`,
      {
        description: cut ? CUT_HINT : undefined,
      },
    )
  } catch (error) {
    failed(error)
  }
}
