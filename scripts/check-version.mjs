#!/usr/bin/env node
// Checks that every place holding the app's version agrees, and, on a release tag, that the
// tag is that version (`v1.2.3`). The installers take their version from tauri.conf.json, the
// app reports Cargo's, and the packages carry their own.
//
//   node scripts/check-version.mjs [tag]

import fs from 'node:fs'

const read = (file) => fs.readFileSync(new URL(`../${file}`, import.meta.url), 'utf8')
const json = (file) => JSON.parse(read(file)).version
const cargo = read('Cargo.toml').match(/\[workspace\.package\][^[]*?\nversion\s*=\s*"([^"]+)"/)?.[1]

const versions = {
  'Cargo.toml (workspace)': cargo,
  'src-tauri/tauri.conf.json': json('src-tauri/tauri.conf.json'),
  'package.json': json('package.json'),
  'apps/desktop/package.json': json('apps/desktop/package.json'),
  'packages/contracts/package.json': json('packages/contracts/package.json'),
}

const distinct = new Set(Object.values(versions))
if (distinct.size !== 1 || distinct.has(undefined)) {
  console.error('Versions disagree:')
  for (const [file, version] of Object.entries(versions)) console.error(`  ${file}: ${version}`)
  process.exit(1)
}
const [version] = distinct
if (!/^\d+\.\d+\.\d+$/.test(version)) {
  // MSI installers accept only numeric major.minor.patch versions.
  console.error(`Version ${version} is not major.minor.patch.`)
  process.exit(1)
}

const tag =
  process.argv[2] ?? (process.env.GITHUB_REF_TYPE === 'tag' ? process.env.GITHUB_REF_NAME : '')
if (tag && tag !== `v${version}`) {
  console.error(`The tag ${tag} does not match the version ${version} (expected v${version}).`)
  process.exit(1)
}
console.log(`Version ${version}${tag ? `, tag ${tag}` : ''}: consistent.`)
