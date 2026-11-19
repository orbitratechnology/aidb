import { describe, expect, it } from 'vitest'
import hookSource from '../../../../src-tauri/isolation/index.js?raw'

type IpcMessage = { cmd: unknown; callback: number; error: number; payload: unknown }
type Hook = (message: IpcMessage) => IpcMessage

function loadHook(): Hook {
  const fakeWindow: { __TAURI_ISOLATION_HOOK__?: Hook } = {}
  // The hook is a classic script that assigns to `window`; run it against a stand-in.
  new Function('window', hookSource)(fakeWindow)
  if (!fakeWindow.__TAURI_ISOLATION_HOOK__) throw new Error('hook was not installed')
  return fakeWindow.__TAURI_ISOLATION_HOOK__
}

const hook = loadHook()

function send(cmd: unknown, payload: unknown = {}): IpcMessage {
  return hook({ cmd, callback: 1, error: 2, payload })
}

describe('isolation hook', () => {
  it('passes app commands, the event plugin and channel transport through unchanged', () => {
    for (const cmd of [
      'app_info',
      'open_external_url',
      'plugin:event|listen',
      'plugin:event|unlisten',
      'plugin:__TAURI_CHANNEL__|fetch',
    ]) {
      expect(send(cmd, { url: 'https://example.com' })).toEqual({
        cmd,
        callback: 1,
        error: 2,
        payload: { url: 'https://example.com' },
      })
    }
  })

  it('reroutes other plugins and malformed names so the core rejects them', () => {
    for (const cmd of [
      'plugin:opener|open_url',
      'plugin:dialog|open',
      'plugin:dialog|save',
      'plugin:fs|read_file',
      'plugin:fs|write_file',
      'plugin:event|other',
      'App_Info',
      '../app_info',
      '',
      undefined,
    ]) {
      const message = send(cmd, { secret: 'x' })
      expect(message.cmd).toBe('__aidb_isolation_blocked__')
      expect(message.payload).toEqual({})
      expect(message).toMatchObject({ callback: 1, error: 2 })
    }
  })

  it('reroutes oversized payloads', () => {
    expect(send('app_info', { blob: 'x'.repeat(17 * 1024 * 1024) }).cmd).toBe(
      '__aidb_isolation_blocked__',
    )
  })
})
