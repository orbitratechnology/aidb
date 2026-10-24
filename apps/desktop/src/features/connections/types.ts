export const ENVIRONMENTS = ['local', 'development', 'staging', 'production', 'other'] as const

export type EnvironmentKind = (typeof ENVIRONMENTS)[number]

/** Identity of the connection the workspace is bound to. Never carries secrets. */
export type ActiveConnection = {
  id: string
  name: string
  database: string
  environment: EnvironmentKind
  readOnly: boolean
}
