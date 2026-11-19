import { z } from 'zod'
import { EnvironmentSchema } from './connections'
import type { ApprovalLevel } from './generated/ApprovalLevel'
import type { AssistantProvider } from './generated/AssistantProvider'
import type { ContextItem } from './generated/ContextItem'
import type { ContextItemKind } from './generated/ContextItemKind'
import type { ContextPreview } from './generated/ContextPreview'
import type { PlanStep } from './generated/PlanStep'
import type { PlanTarget } from './generated/PlanTarget'
import type { RiskLevel } from './generated/RiskLevel'
import type { StepStatus } from './generated/StepStatus'
import type { TaskEvent } from './generated/TaskEvent'
import type { TaskOutcome } from './generated/TaskOutcome'
import type { TaskPlan } from './generated/TaskPlan'
import type { TaskStatus } from './generated/TaskStatus'
import type { ToolName } from './generated/ToolName'
import type { VerificationStatus } from './generated/VerificationStatus'
import type { VerificationStep } from './generated/VerificationStep'

export const RiskLevelSchema = z.enum(['R0', 'R1', 'R2', 'R3', 'R4']) satisfies z.ZodType<RiskLevel>

export const ApprovalLevelSchema = z.enum([
  'none',
  'apply',
  'elevated',
  'blocked',
]) satisfies z.ZodType<ApprovalLevel>

export const ToolNameSchema = z.enum([
  'list_schemas',
  'list_tables',
  'describe_table',
  'run_read_query',
  'explain_query',
]) satisfies z.ZodType<ToolName>

export const StepStatusSchema = z.enum([
  'planned',
  'running',
  'succeeded',
  'failed',
  'skipped',
  'cancelled',
]) satisfies z.ZodType<StepStatus>

export const VerificationStatusSchema = z.enum([
  'not_run',
  'passed',
  'failed',
]) satisfies z.ZodType<VerificationStatus>

export const TaskStatusSchema = z.enum([
  'executing',
  'verifying',
  'succeeded',
  'partial',
  'failed',
  'cancelled',
]) satisfies z.ZodType<TaskStatus>

export const PlanStepSchema = z.object({
  tool: ToolNameSchema,
  summary: z.string(),
  input: z.json(),
  risk: RiskLevelSchema,
  status: StepStatusSchema,
  outcome: z.string().nullable(),
}) satisfies z.ZodType<PlanStep>

export const VerificationStepSchema = z.object({
  description: z.string(),
  status: VerificationStatusSchema,
  summary: z.string().nullable(),
}) satisfies z.ZodType<VerificationStep>

export const PlanTargetSchema = z.object({
  connectionId: z.string(),
  database: z.string(),
  environment: EnvironmentSchema,
  readOnly: z.boolean(),
}) satisfies z.ZodType<PlanTarget>

export const TaskPlanSchema = z.object({
  risk: RiskLevelSchema,
  requiredApproval: ApprovalLevelSchema,
  version: z.number().int(),
  toolsVersion: z.number().int(),
  taskId: z.string(),
  target: PlanTargetSchema,
  intent: z.string(),
  assumptions: z.array(z.string()),
  steps: z.array(PlanStepSchema),
  verification: z.array(VerificationStepSchema),
}) satisfies z.ZodType<TaskPlan>

export const ContextItemKindSchema = z.enum([
  'question',
  'connection',
  'schema_overview',
  'selected_table',
  'attachment',
]) satisfies z.ZodType<ContextItemKind>

export const ContextItemSchema = z.object({
  kind: ContextItemKindSchema,
  label: z.string(),
  includedChars: z.number().int().nonnegative(),
  totalChars: z.number().int().nonnegative(),
  truncated: z.boolean(),
  containsDataValues: z.boolean(),
}) satisfies z.ZodType<ContextItem>

export const AssistantProviderSchema = z.object({
  name: z.string(),
  model: z.string(),
  hosted: z.boolean(),
}) satisfies z.ZodType<AssistantProvider>

export const ContextPreviewSchema = z.object({
  provider: AssistantProviderSchema,
  items: z.array(ContextItemSchema),
  estimatedTokens: z.number().int().nonnegative(),
  budgetTokens: z.number().int().positive(),
  dataAllowed: z.boolean(),
}) satisfies z.ZodType<ContextPreview>

const index = z.number().int().nonnegative()

export const TaskEventSchema = z.discriminatedUnion('event', [
  z.object({ event: z.literal('started'), taskId: z.string(), items: z.array(ContextItemSchema) }),
  z.object({ event: z.literal('round_started'), round: index }),
  z.object({ event: z.literal('text'), text: z.string() }),
  z.object({ event: z.literal('step_started'), index, step: PlanStepSchema }),
  z.object({ event: z.literal('step_finished'), index, step: PlanStepSchema }),
  z.object({ event: z.literal('call_rejected'), tool: z.string(), reason: z.string() }),
]) satisfies z.ZodType<TaskEvent>

export const TaskOutcomeSchema = z.object({
  taskId: z.string(),
  status: TaskStatusSchema,
  answer: z.string(),
  notice: z.string().nullable(),
  plan: TaskPlanSchema,
}) satisfies z.ZodType<TaskOutcome>
