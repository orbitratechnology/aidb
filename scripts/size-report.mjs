#!/usr/bin/env node
// Size of the built interface: what the first screen loads (the scripts and styles that
// index.html references, and the modules they preload) and everything, including the screens
// loaded on demand. Sizes are raw and gzip-compressed. Written to the CI job summary when run
// there.
//
//   pnpm build:web && node scripts/size-report.mjs [dist directory]

import fs from 'node:fs'
import path from 'node:path'
import zlib from 'node:zlib'

const dist = path.resolve(process.argv[2] ?? 'apps/desktop/dist')
const html = fs.readFileSync(path.join(dist, 'index.html'), 'utf8')

const initial = [...html.matchAll(/(?:src|href)="\/?(assets\/[^"]+\.(?:js|css))"/g)].map(
  (match) => match[1],
)

function sizes(file) {
  const bytes = fs.readFileSync(path.join(dist, file))
  return { raw: bytes.length, gzip: zlib.gzipSync(bytes, { level: 9 }).length }
}

function total(files) {
  return files.map(sizes).reduce((a, b) => ({ raw: a.raw + b.raw, gzip: a.gzip + b.gzip }), {
    raw: 0,
    gzip: 0,
  })
}

const kb = (bytes) => `${(bytes / 1024).toFixed(0)} KB`
const all = fs
  .readdirSync(path.join(dist, 'assets'))
  .filter((name) => /\.(js|css)$/.test(name))
  .map((name) => `assets/${name}`)

const first = total(initial)
const everything = total(all)
const largest = all
  .map((file) => ({ file, ...sizes(file) }))
  .sort((a, b) => b.raw - a.raw)
  .slice(0, 5)

const table = [
  '| Interface bundle | Files | Size | Gzip |',
  '| --- | ---: | ---: | ---: |',
  `| Loaded for the first screen | ${initial.length} | ${kb(first.raw)} | ${kb(first.gzip)} |`,
  `| All screens | ${all.length} | ${kb(everything.raw)} | ${kb(everything.gzip)} |`,
  ...largest.map(
    ({ file, raw, gzip }) =>
      `| Largest: \`${path.basename(file)}\` | 1 | ${kb(raw)} | ${kb(gzip)} |`,
  ),
].join('\n')

console.log(table)
if (process.env.GITHUB_STEP_SUMMARY) {
  fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, `\n${table}\n`)
}
