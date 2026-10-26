import { EnvironmentSchema, type Environment } from '@aidb/contracts'

export const ENVIRONMENTS = EnvironmentSchema.options

export type EnvironmentKind = Environment

/** Identity of the connection the workspace is bound to. Never carries secrets. */
export type ActiveConnection = {
  id: string
  name: string
  database: string
  environment: EnvironmentKind
  readOnly: boolean
}
