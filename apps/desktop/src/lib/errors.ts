/** The message of a thrown value, for showing to the user. */
export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
