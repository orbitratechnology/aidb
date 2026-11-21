import { z } from 'zod'
import type { PlanNode } from './generated/PlanNode'
import type { QueryPlan } from './generated/QueryPlan'
import type { Suggestion } from './generated/Suggestion'

export const PlanNodeSchema = z.object({
  id: z.number().int().nonnegative(),
  parent: z.number().int().nonnegative().nullable(),
  depth: z.number().int().nonnegative(),
  nodeType: z.string(),
  label: z.string(),
  details: z.array(z.string()),
  totalCost: z.number(),
  planRows: z.number(),
  actualRows: z.number().nullable(),
  loops: z.number().nullable(),
  actualMs: z.number().nullable(),
  selfMs: z.number().nullable(),
}) satisfies z.ZodType<PlanNode>

export const SuggestionSchema = z.object({
  kind: z.enum([
    'missing_index',
    'stale_statistics',
    'sort_spill',
    'hash_spill',
    'lossy_bitmap',
    'heap_fetches',
    'fewer_workers',
  ]),
  severity: z.enum(['info', 'warning']),
  title: z.string(),
  detail: z.string(),
  evidence: z.array(z.string()),
  node: z.number().int().nonnegative().nullable(),
  sql: z.string().nullable(),
}) satisfies z.ZodType<Suggestion>

export const QueryPlanSchema = z.object({
  analyzed: z.boolean(),
  planningMs: z.number().nullable(),
  executionMs: z.number().nullable(),
  nodes: z.array(PlanNodeSchema),
  truncated: z.boolean(),
  suggestions: z.array(SuggestionSchema),
}) satisfies z.ZodType<QueryPlan>
