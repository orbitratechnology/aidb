import { QueryClient } from '@tanstack/react-query'
import { NativeError } from './native'

const MAX_RETRIES = 2

/** Retries a failed read only when the native core says the failure is temporary. */
export function shouldRetry(failureCount: number, error: unknown): boolean {
  return error instanceof NativeError && error.retryable && failureCount < MAX_RETRIES
}

export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: shouldRetry,
        // A desktop window gains focus constantly; data changes through this app, not elsewhere.
        refetchOnWindowFocus: false,
      },
      // Writes are never repeated automatically.
      mutations: { retry: false },
    },
  })
}
