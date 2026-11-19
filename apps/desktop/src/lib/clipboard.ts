/** Copies text to the system clipboard. Only ever called from an explicit user action. */
export async function copyText(text: string): Promise<void> {
  if (!navigator.clipboard) {
    throw new Error('The clipboard is not available.')
  }
  await navigator.clipboard.writeText(text)
}
