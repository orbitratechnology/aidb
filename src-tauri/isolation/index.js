// aidb isolation hook (Tauri isolation pattern).
//
// Runs in Tauri's sandboxed isolation iframe. Every IPC message from the app passes through this
// hook before it is encrypted for the native core; messages that skip it are rejected by the core.
//
// The capability ACL in src-tauri/capabilities is the real allowlist. This hook is an independent
// second check: only app commands, the event plugin and Tauri's channel transport are reachable,
// so a compromised frontend dependency cannot reach other plugin APIs even if a capability is
// misconfigured. Keep this file dependency-free and free of ES module syntax (Tauri inlines it).
'use strict'
;(function () {
  var MAX_PAYLOAD_CHARS = 16 * 1024 * 1024
  var APP_COMMAND = /^[a-z][a-z0-9_]{0,63}$/
  var CORE_PLUGIN_COMMAND =
    /^plugin:(event\|(listen|unlisten|emit|emit_to)|__TAURI_CHANNEL__\|fetch)$/
  // Not a real command: the core rejects it, so a blocked call fails instead of hanging.
  var BLOCKED_COMMAND = '__aidb_isolation_blocked__'

  function isAllowedCommand(cmd) {
    return typeof cmd === 'string' && (APP_COMMAND.test(cmd) || CORE_PLUGIN_COMMAND.test(cmd))
  }

  function payloadChars(payload) {
    if (payload instanceof ArrayBuffer || ArrayBuffer.isView(payload)) {
      return payload.byteLength
    }
    try {
      return JSON.stringify(payload === undefined ? null : payload).length
    } catch (error) {
      return Infinity
    }
  }

  window.__TAURI_ISOLATION_HOOK__ = function (message) {
    if (!isAllowedCommand(message.cmd) || payloadChars(message.payload) > MAX_PAYLOAD_CHARS) {
      message.cmd = BLOCKED_COMMAND
      message.payload = {}
    }
    return message
  }
})()
