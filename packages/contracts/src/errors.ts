import { z } from 'zod'
import type { ErrorCode } from './generated/ErrorCode'
import type { ErrorPayload } from './generated/ErrorPayload'

export const ERROR_CODES = [
  'invalid_input',
  'not_found',
  'conflict',
  'permission_denied',
  'approval_required',
  'timeout',
  'cancelled',
  'unavailable',
  'storage',
  'credential_store',
  'internal',
] as const satisfies readonly ErrorCode[]

// Fails to compile when the native core adds a code that is missing above.
type MissingErrorCode = Exclude<ErrorCode, (typeof ERROR_CODES)[number]>
const allCodesListed: [MissingErrorCode] extends [never] ? true : MissingErrorCode = true
void allCodesListed

export const ErrorCodeSchema = z.enum(ERROR_CODES) satisfies z.ZodType<ErrorCode>

export const ErrorPayloadSchema = z.object({
  code: ErrorCodeSchema,
  message: z.string(),
  retryable: z.boolean(),
}) satisfies z.ZodType<ErrorPayload>
