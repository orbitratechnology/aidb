// The test build's entry: Tauri's IPC mocks answer from fixtures, then the real app loads.
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks'
import { handler, type Scenario } from './core'

const params = new URLSearchParams(location.search)
const scenario = (params.get('scenario') ?? 'connected') as Scenario
const theme = params.get('theme') === 'dark' ? 'dark' : 'light'
// The theme the app paints its first frame with, as a saved preference would.
localStorage.setItem('aidb.theme', theme)
;(window as unknown as { __unhandled: string[] }).__unhandled = []
;(globalThis as unknown as { isTauri: boolean }).isTauri = true
mockWindows('main')
mockIPC((command, args) => handler(scenario, theme)(command, args as Record<string, unknown>), {
  shouldMockEvents: true,
})

await import('../../src/main.tsx')
