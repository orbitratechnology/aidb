import { z } from 'zod'
import { ErrorCodeSchema } from './errors'
import type { AnalyzedStatement } from './generated/AnalyzedStatement'
import type { QueryFailure } from './generated/QueryFailure'
import type { QueryRefusal } from './generated/QueryRefusal'
import type { QueryResult } from './generated/QueryResult'
import type { QueryRun } from './generated/QueryRun'
import type { SourcePosition } from './generated/SourcePosition'
import type { SourceSpan } from './generated/SourceSpan'
import type { SqlAnalysis } from './generated/SqlAnalysis'
import type { SqlProblem } from './generated/SqlProblem'
import type { StatementKind } from './generated/StatementKind'
import { CellRefSchema, PageColumnSchema } from './table-data'

export const SourcePositionSchema = z.object({
  line: z.number().int().positive(),
  column: z.number().int().positive(),
  offset: z.number().int().nonnegative(),
}) satisfies z.ZodType<SourcePosition>

export const SourceSpanSchema = z.object({
  start: SourcePositionSchema,
  end: SourcePositionSchema,
}) satisfies z.ZodType<SourceSpan>

export const StatementKindSchema = z.enum([
  'read',
  'write',
  'schema',
  'session',
  'privileged',
  'unknown',
]) satisfies z.ZodType<StatementKind>

export const AnalyzedStatementSchema = z.object({
  span: SourceSpanSchema,
  kind: StatementKindSchema,
  reason: z.string().nullable(),
  problemAt: SourcePositionSchema.nullable(),
}) satisfies z.ZodType<AnalyzedStatement>

export const SqlProblemSchema = z.object({
  message: z.string(),
  position: SourcePositionSchema.nullable(),
}) satisfies z.ZodType<SqlProblem>

export const SqlAnalysisSchema = z.object({
  statements: z.array(AnalyzedStatementSchema),
  problem: SqlProblemSchema.nullable(),
}) satisfies z.ZodType<SqlAnalysis>

export const QueryResultSchema = z.object({
  statement: SourceSpanSchema,
  columns: z.array(PageColumnSchema),
  rows: z.array(z.array(z.string().nullable())),
  truncatedCells: z.array(CellRefSchema),
  offset: z.number().int().nonnegative(),
  hasMore: z.boolean(),
  byteLimitReached: z.boolean(),
  cellCharLimit: z.number().int().positive(),
  elapsedMs: z.number().int().nonnegative(),
}) satisfies z.ZodType<QueryResult>

export const QueryRefusalSchema = z.object({
  message: z.string(),
  kind: StatementKindSchema.nullable(),
  position: SourcePositionSchema.nullable(),
}) satisfies z.ZodType<QueryRefusal>

export const QueryFailureSchema = z.object({
  code: ErrorCodeSchema,
  message: z.string(),
  sqlstate: z.string().nullable(),
  position: SourcePositionSchema.nullable(),
  statement: SourceSpanSchema,
}) satisfies z.ZodType<QueryFailure>

export const QueryRunSchema = z.discriminatedUnion('status', [
  QueryResultSchema.extend({ status: z.literal('completed') }),
  QueryRefusalSchema.extend({ status: z.literal('refused') }),
  QueryFailureSchema.extend({ status: z.literal('failed') }),
]) satisfies z.ZodType<QueryRun>
