import { AppInfoSchema, COMMANDS, type AppInfo } from '@aidb/contracts'
import { z } from 'zod'
import { callCommand, NativeError } from './native'

export function getAppInfo(): Promise<AppInfo> {
  return callCommand(COMMANDS.appInfo, AppInfoSchema)
}

const EXTERNAL_URL = z.url({ protocol: /^(https?|mailto)$/ })

/** Opens a link in the system browser. The native core enforces the same rules. */
export async function openExternalUrl(url: string): Promise<void> {
  if (!EXTERNAL_URL.safeParse(url).success) {
    throw new NativeError('invalid_input', 'Only web and email links can be opened.')
  }
  await callCommand(COMMANDS.openExternalUrl, z.null(), { url })
}
