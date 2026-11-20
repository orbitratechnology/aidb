#!/usr/bin/env node
// Install smoke test for an aidb package, on a clean machine (CI runner or container).
//
//   node scripts/smoke-test.mjs <format> <package> <newer-package>
//
// <format> is deb, rpm, appimage, msi, nsis or dmg. <newer-package> is the same build bundled
// with a higher version number, to test an upgrade. The test:
//
//   1. installs the package with the platform's installer (dependencies come from the system's
//      package manager, so a missing dependency fails the install);
//   2. starts the app and waits for the interface to reach the core ("interface connected" in
//      the log), then kills it, as a crash would;
//   3. starts it again: the local database must open after the crash; then quits it
//      (gracefully where the platform can ask an app to quit) and checks the database's
//      integrity;
//   4. upgrades to the newer package: one installation of the new version, user data kept,
//      and the app starts;
//   5. uninstalls: the program is gone, the user's data stays (as on every platform's
//      convention), and is then removed for the next run.
//
// Any ERROR line or panic in the app's log fails the test. Run it with a display: under
// xvfb-run on Linux; Windows and macOS runners have one.

import { execFileSync, spawn } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'

const IDENTIFIER = 'com.orbitratech.aidb'
const PRODUCT = 'aidb'
const READY = 'interface connected'
const START_TIMEOUT_MS = 120_000

const [format, firstArg, secondArg] = process.argv.slice(2)
if (!format || !firstArg || !secondArg) {
  console.error('usage: node scripts/smoke-test.mjs <format> <package> <newer-package>')
  process.exit(2)
}
const first = path.resolve(firstArg)
const second = path.resolve(secondArg)

function step(message) {
  console.log(`\n== ${message}`)
}

function fail(message) {
  throw new Error(message)
}

function run(command, args, options = {}) {
  console.log(`$ ${command} ${args.join(' ')}`)
  return execFileSync(command, args, { stdio: ['ignore', 'pipe', 'inherit'], ...options })
    .toString()
    .trim()
}

const isRoot = typeof process.getuid === 'function' && process.getuid() === 0
function asRoot(command, args) {
  return isRoot ? run(command, args) : run('sudo', [command, ...args])
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

/** Where Tauri keeps the app's data and logs on this platform (`app_data_dir`, `app_log_dir`). */
function appDirs() {
  const home = os.homedir()
  switch (process.platform) {
    case 'linux': {
      const base = process.env.XDG_DATA_HOME || path.join(home, '.local', 'share')
      return { data: path.join(base, IDENTIFIER), logs: path.join(base, IDENTIFIER, 'logs') }
    }
    case 'darwin':
      return {
        data: path.join(home, 'Library', 'Application Support', IDENTIFIER),
        logs: path.join(home, 'Library', 'Logs', IDENTIFIER),
      }
    case 'win32':
      return {
        data: path.join(process.env.APPDATA, IDENTIFIER),
        logs: path.join(process.env.LOCALAPPDATA, IDENTIFIER, 'logs'),
      }
    default:
      return fail(`unsupported platform ${process.platform}`)
  }
}

/** Versions of aidb that Windows lists as installed (Apps & features). */
function windowsInstalledVersions() {
  const script =
    "Get-ItemProperty 'HKLM:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\*'," +
    "'HKLM:\\Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\*'," +
    "'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\*' -ErrorAction SilentlyContinue" +
    ` | Where-Object { $_.DisplayName -eq '${PRODUCT}' } | ForEach-Object { $_.DisplayVersion }`
  const output = run('powershell', ['-NoProfile', '-Command', script])
  return output.split(/\r?\n/).filter(Boolean)
}

function msiexec(args) {
  try {
    run('msiexec', args)
  } catch (error) {
    // 3010: success, a restart is required to finish.
    if (error.status !== 3010) throw error
  }
}

async function waitUntil(check, timeoutMs, what) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (check()) return
    await sleep(1000)
  }
  fail(`timed out waiting for ${what}`)
}

const nsisDir = () => path.join(process.env.LOCALAPPDATA ?? '', PRODUCT)
const macApp = path.join(os.homedir(), 'Applications', `${PRODUCT}.app`)
const appImage = path.join(os.tmpdir(), 'aidb-smoke', `${PRODUCT}.AppImage`)

function installDmg(file) {
  const mount = fs.mkdtempSync(path.join(os.tmpdir(), 'aidb-dmg-'))
  run('hdiutil', ['attach', '-nobrowse', '-readonly', '-mountpoint', mount, file])
  try {
    fs.rmSync(macApp, { recursive: true, force: true })
    fs.mkdirSync(path.dirname(macApp), { recursive: true })
    run('ditto', [path.join(mount, `${PRODUCT}.app`), macApp])
  } finally {
    run('hdiutil', ['detach', mount])
  }
  // The bundle's signature (ad hoc until release signing) must be intact after copying.
  run('codesign', ['--verify', '--deep', '--strict', '--verbose=2', macApp])
}

/** How each package format installs, reports its version, upgrades and uninstalls. */
const FORMATS = {
  deb: {
    binary: () => '/usr/bin/aidb',
    install: (file) => {
      const cache = process.env.APT_ARCHIVES
      asRoot('apt-get', [
        'install',
        '-y',
        '--no-install-recommends',
        ...(cache ? ['-o', `Dir::Cache::Archives=${cache}`] : []),
        file,
      ])
    },
    upgrade: (file) => FORMATS.deb.install(file),
    versions: () => [run('dpkg-query', ['-W', '-f=${Version}', PRODUCT])],
    uninstall: () => asRoot('apt-get', ['remove', '-y', PRODUCT]),
  },
  rpm: {
    binary: () => '/usr/bin/aidb',
    install: (file) => asRoot('dnf', ['install', '-y', file]),
    upgrade: (file) => asRoot('dnf', ['install', '-y', file]),
    versions: () => run('rpm', ['-q', '--qf', '%{VERSION}\\n', PRODUCT]).split('\n'),
    uninstall: () => asRoot('dnf', ['remove', '-y', PRODUCT]),
  },
  appimage: {
    binary: () => appImage,
    // No FUSE on CI machines: the AppImage runtime extracts itself instead.
    env: { APPIMAGE_EXTRACT_AND_RUN: '1' },
    install: (file) => {
      fs.mkdirSync(path.dirname(appImage), { recursive: true })
      fs.copyFileSync(file, appImage)
      fs.chmodSync(appImage, 0o755)
    },
    upgrade: (file) => FORMATS.appimage.install(file),
    versions: null,
    uninstall: () => fs.rmSync(appImage, { force: true }),
  },
  msi: {
    binary: () =>
      path.join(process.env.ProgramFiles ?? 'C:\\Program Files', PRODUCT, `${PRODUCT}.exe`),
    install: (file) => msiexec(['/i', file, '/qn', '/norestart']),
    upgrade: (file) => msiexec(['/i', file, '/qn', '/norestart']),
    versions: windowsInstalledVersions,
    uninstall: () => msiexec(['/x', second, '/qn', '/norestart']),
  },
  nsis: {
    binary: () => path.join(nsisDir(), `${PRODUCT}.exe`),
    install: (file) => run(file, ['/S']),
    upgrade: (file) => run(file, ['/S']),
    versions: windowsInstalledVersions,
    uninstall: async () => {
      run(path.join(nsisDir(), 'uninstall.exe'), ['/S'])
      // The uninstaller copies itself elsewhere and returns at once.
      await waitUntil(() => !fs.existsSync(FORMATS.nsis.binary()), 120_000, 'the uninstaller')
    },
  },
  dmg: {
    binary: () => path.join(macApp, 'Contents', 'MacOS', PRODUCT),
    install: installDmg,
    upgrade: installDmg,
    versions: () => [
      run('plutil', [
        '-extract',
        'CFBundleShortVersionString',
        'raw',
        path.join(macApp, 'Contents', 'Info.plist'),
      ]),
    ],
    uninstall: () => fs.rmSync(macApp, { recursive: true, force: true }),
  },
}

const spec = FORMATS[format] ?? fail(`unknown format ${format}`)
const dirs = appDirs()

function logText() {
  if (!fs.existsSync(dirs.logs)) return ''
  return fs
    .readdirSync(dirs.logs)
    .filter((name) => name.endsWith('.log'))
    .map((name) => fs.readFileSync(path.join(dirs.logs, name), 'utf8'))
    .join('\n')
}

function checkLog(context) {
  const problems = logText()
    .split('\n')
    .filter((line) => / ERROR |panicked|panic /.test(line))
  if (problems.length > 0) fail(`${context}: the log has errors:\n${problems.join('\n')}`)
}

/** Starts the installed app and waits until its interface has reached the core. */
async function start(context) {
  // Each start writes a fresh log, so its lines are the only ones checked.
  fs.rmSync(dirs.logs, { recursive: true, force: true })
  const binary = spec.binary()
  if (!fs.existsSync(binary)) fail(`${context}: ${binary} is not installed`)
  console.log(`$ ${binary}`)
  const child = spawn(binary, [], {
    env: {
      ...process.env,
      ...(spec.env ?? {}),
      // A display without a GPU: WebKitGTK's DMA-BUF renderer cannot start on Xvfb.
      ...(process.platform === 'linux' ? { WEBKIT_DISABLE_DMABUF_RENDERER: '1' } : {}),
    },
    stdio: ['ignore', 'pipe', 'pipe'],
    // Its own process group, so a signal reaches the helpers it starts too (the AppImage
    // runtime, WebKit's processes), as when the app itself crashes or quits.
    detached: process.platform !== 'win32',
  })
  let output = ''
  child.stdout.on('data', (data) => (output += data))
  child.stderr.on('data', (data) => (output += data))
  let exited = null
  child.on('exit', (code, signal) => (exited = { code, signal }))
  try {
    await waitUntil(
      () => {
        if (exited) fail(`${context}: the app exited (${JSON.stringify(exited)})`)
        return logText().includes(READY)
      },
      START_TIMEOUT_MS,
      `${context}: the interface to connect`,
    )
    checkLog(context)
  } catch (error) {
    signal(child, 'SIGKILL')
    console.error(`--- app output ---\n${output.slice(-4000)}\n--- app log ---\n${logText()}`)
    throw error
  }
  console.log(`${context}: started, the interface connected`)
  return { child, exitedRef: () => exited }
}

/** Sends a signal to the app and every process it started. */
function signal(child, name) {
  if (process.platform === 'win32') {
    child.kill(name)
  } else {
    try {
      process.kill(-child.pid, name)
    } catch {
      // The group is already gone.
    }
  }
}

async function crash({ child, exitedRef }) {
  signal(child, 'SIGKILL')
  await waitUntil(() => exitedRef() !== null, 30_000, 'the app to be killed')
}

/** Asks the app to quit the way the platform's own UI would, and checks it closed cleanly. */
async function quit(app, context) {
  const { child, exitedRef } = app
  if (process.platform === 'win32') {
    // Without /F, taskkill sends WM_CLOSE: the same as closing the window.
    run('taskkill', ['/PID', String(child.pid)])
  } else if (process.platform === 'darwin') {
    // The same Quit as the app menu. Sending it can need an automation permission that a CI
    // machine may not grant; then the app is stopped instead.
    try {
      run('osascript', ['-e', `tell application id "${IDENTIFIER}" to quit`], { timeout: 30_000 })
    } catch (error) {
      console.log(`${context}: could not ask the app to quit (${error.message}); stopping it`)
      signal(child, 'SIGTERM')
      await waitUntil(() => exitedRef() !== null, 30_000, 'the app to stop')
      return
    }
  } else {
    // Linux has no window manager on the CI display to close the window: stop the process.
    signal(child, 'SIGTERM')
    await waitUntil(() => exitedRef() !== null, 30_000, 'the app to stop')
    return
  }
  await waitUntil(() => exitedRef() !== null, 60_000, `${context}: the app to quit`)
  if (!logText().includes('local database closed')) {
    fail(`${context}: the app did not close its database on quit`)
  }
  checkLog(context)
}

async function checkDatabase(context) {
  const file = path.join(dirs.data, 'aidb.sqlite3')
  if (!fs.existsSync(file)) fail(`${context}: no local database at ${file}`)
  const { DatabaseSync } = await import('node:sqlite')
  const db = new DatabaseSync(file)
  try {
    const integrity = db.prepare('PRAGMA integrity_check').get()
    if (Object.values(integrity)[0] !== 'ok') fail(`${context}: ${JSON.stringify(integrity)}`)
    const { applied } = db
      .prepare('SELECT count(*) AS applied FROM _sqlx_migrations WHERE success')
      .get()
    if (applied < 1) fail(`${context}: no schema migrations recorded`)
    console.log(`${context}: database intact, ${applied} migrations applied`)
  } finally {
    db.close()
  }
}

function checkVersion(expected, context) {
  if (!spec.versions) return
  const versions = spec.versions()
  if (versions.length !== 1 || !versions[0].startsWith(expected)) {
    fail(`${context}: expected one installation of ${expected}, found ${JSON.stringify(versions)}`)
  }
  console.log(`${context}: ${versions[0]} installed`)
}

function packageVersion(file) {
  const match = path.basename(file).match(/(\d+\.\d+\.\d+)/)
  return match ? match[1] : fail(`no version in ${file}`)
}

async function main() {
  console.log(`Smoke test: ${format} on ${process.platform}/${process.arch}`)
  console.log(`data: ${dirs.data}\nlogs: ${dirs.logs}`)
  // A clean machine: no data from an earlier run.
  fs.rmSync(dirs.data, { recursive: true, force: true })
  fs.rmSync(dirs.logs, { recursive: true, force: true })

  step(`Install ${path.basename(first)}`)
  await spec.install(first)
  checkVersion(packageVersion(first), 'install')

  step('Start, then crash')
  await crash(await start('first start'))

  step('Start after the crash, then quit')
  await quit(await start('start after a crash'), 'quit')
  await checkDatabase('after the crash and a quit')
  const marker = path.join(dirs.data, 'smoke-marker')
  fs.writeFileSync(marker, 'kept across upgrades')

  step(`Upgrade to ${path.basename(second)}`)
  await spec.upgrade(second)
  checkVersion(packageVersion(second), 'upgrade')
  if (!fs.existsSync(marker)) fail('upgrade: the user data was removed')
  await quit(await start('start after the upgrade'), 'quit after the upgrade')
  await checkDatabase('after the upgrade')

  step('Uninstall')
  await spec.uninstall()
  if (fs.existsSync(spec.binary())) fail(`uninstall: ${spec.binary()} is still there`)
  if (!fs.existsSync(marker)) fail('uninstall: the user data was removed')
  console.log('uninstall: the program is gone; the user data stays, by convention')
  fs.rmSync(dirs.data, { recursive: true, force: true })
  fs.rmSync(dirs.logs, { recursive: true, force: true })

  console.log(`\nSmoke test passed: ${format}`)
}

main().then(
  // Exit even if a process the app started still holds its output open.
  () => process.exit(0),
  (error) => {
    console.error(`\nSmoke test failed: ${error.message}`)
    process.exit(1)
  },
)
