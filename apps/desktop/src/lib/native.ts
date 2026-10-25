import { invoke, isTauri, type InvokeArgs } from '@tauri-apps/api/core'
import { ErrorPayloadSchema, type CommandName, type ErrorCode } from '@aidb/contracts'
import type { z } from 'zod'

/** A failure reported by, or on the way to, the native core. */
export class NativeError extends Error {
  readonly code: ErrorCode
  readonly retryable: boolean

  constructor(code: ErrorCode, message: string, retryable = false) {
    super(message)
    this.name = 'NativeError'
    this.code = code
    this.retryable = retryable
  }
}

/** Normalizes whatever `invoke` rejected with into a {@link NativeError}. */
export function toNativeError(raw: unknown): NativeError {
  const payload = ErrorPayloadSchema.safeParse(raw)
  if (payload.success) {
    return new NativeError(payload.data.code, payload.data.message, payload.data.retryable)
  }
  // Tauri rejects with a plain string when a command is unknown or not permitted.
  const message =
    typeof raw === 'string' && raw.length > 0
      ? raw
      : 'The native core returned an unexpected error.'
  return new NativeError('internal', message)
}

/**
 * Invokes a native command and validates its result. Input validation here is a convenience;
 * the native core validates everything again.
 */
export async function callCommand<T>(
  command: CommandName,
  resultSchema: z.ZodType<T>,
  args?: InvokeArgs,
): Promise<T> {
  // `pnpm dev:web` runs the UI in a plain browser, where no native core exists.
  if (!isTauri()) {
    throw new NativeError(
      'unavailable',
      'The native core is only available in the aidb desktop app.',
    )
  }

  let raw: unknown
  try {
    raw = await invoke(command, args)
  } catch (error) {
    throw toNativeError(error)
  }

  const result = resultSchema.safeParse(raw)
  if (!result.success) {
    throw new NativeError('internal', `The native core sent an unexpected response to ${command}.`)
  }
  return result.data
}
