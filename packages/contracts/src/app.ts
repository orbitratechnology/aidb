import { z } from 'zod'
import type { AppInfo } from './generated/AppInfo'

export const AppInfoSchema = z.object({
  name: z.string(),
  version: z.string(),
  platform: z.string(),
}) satisfies z.ZodType<AppInfo>
