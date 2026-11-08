import { z } from 'zod'
import type { CatalogSnapshot } from './generated/CatalogSnapshot'
import type { Column } from './generated/Column'
import type { ForeignKey } from './generated/ForeignKey'
import type { Identity } from './generated/Identity'
import type { Index } from './generated/Index'
import type { KeyConstraint } from './generated/KeyConstraint'
import type { ObjectDetails } from './generated/ObjectDetails'
import type { ObjectKind } from './generated/ObjectKind'
import type { ObjectName } from './generated/ObjectName'
import type { ObjectSummary } from './generated/ObjectSummary'
import type { ReferentialAction } from './generated/ReferentialAction'
import type { SchemaSummary } from './generated/SchemaSummary'

export const ObjectKindSchema = z.enum([
  'table',
  'partitioned_table',
  'view',
  'materialized_view',
  'foreign_table',
]) satisfies z.ZodType<ObjectKind>

export const SchemaSummarySchema = z.object({
  name: z.string(),
  comment: z.string().nullable(),
  accessible: z.boolean(),
}) satisfies z.ZodType<SchemaSummary>

export const ObjectSummarySchema = z.object({
  schema: z.string(),
  name: z.string(),
  kind: ObjectKindSchema,
  rowEstimate: z.number().nonnegative().nullable(),
  comment: z.string().nullable(),
  readable: z.boolean(),
}) satisfies z.ZodType<ObjectSummary>

export const CatalogSnapshotSchema = z.object({
  connectionId: z.uuid(),
  loadedAt: z.string(),
  database: z.string(),
  fingerprint: z.string(),
  schemas: z.array(SchemaSummarySchema),
  objects: z.array(ObjectSummarySchema),
  truncated: z.boolean(),
}) satisfies z.ZodType<CatalogSnapshot>

export const ObjectNameSchema = z.object({
  schema: z.string(),
  name: z.string(),
}) satisfies z.ZodType<ObjectName>

export const IdentitySchema = z.enum(['always', 'by_default']) satisfies z.ZodType<Identity>

export const ColumnSchema = z.object({
  name: z.string(),
  position: z.number().int().positive(),
  dataType: z.string(),
  nullable: z.boolean(),
  default: z.string().nullable(),
  identity: IdentitySchema.nullable(),
  generated: z.string().nullable(),
  comment: z.string().nullable(),
}) satisfies z.ZodType<Column>

export const KeyConstraintSchema = z.object({
  name: z.string(),
  columns: z.array(z.string()),
}) satisfies z.ZodType<KeyConstraint>

export const ReferentialActionSchema = z.enum([
  'no_action',
  'restrict',
  'cascade',
  'set_null',
  'set_default',
]) satisfies z.ZodType<ReferentialAction>

export const ForeignKeySchema = z.object({
  name: z.string(),
  from: ObjectNameSchema,
  columns: z.array(z.string()),
  to: ObjectNameSchema,
  referencedColumns: z.array(z.string()),
  onUpdate: ReferentialActionSchema,
  onDelete: ReferentialActionSchema,
}) satisfies z.ZodType<ForeignKey>

export const IndexSchema = z.object({
  name: z.string(),
  definition: z.string(),
  unique: z.boolean(),
  primary: z.boolean(),
  method: z.string(),
}) satisfies z.ZodType<Index>

export const ObjectDetailsSchema = z.object({
  summary: ObjectSummarySchema,
  columns: z.array(ColumnSchema),
  primaryKey: KeyConstraintSchema.nullable(),
  uniqueConstraints: z.array(KeyConstraintSchema),
  foreignKeys: z.array(ForeignKeySchema),
  referencedBy: z.array(ForeignKeySchema),
  indexes: z.array(IndexSchema),
  definition: z.string().nullable(),
}) satisfies z.ZodType<ObjectDetails>
