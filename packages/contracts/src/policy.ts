import { z } from 'zod'
import type { ApprovalLevel } from './generated/ApprovalLevel'
import type { RiskLevel } from './generated/RiskLevel'

export const RiskLevelSchema = z.enum(['R0', 'R1', 'R2', 'R3', 'R4']) satisfies z.ZodType<RiskLevel>

export const ApprovalLevelSchema = z.enum([
  'none',
  'apply',
  'elevated',
  'blocked',
]) satisfies z.ZodType<ApprovalLevel>
