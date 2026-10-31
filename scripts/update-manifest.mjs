// Writes the update list (latest.json) that installed browsers read through Tauri's updater:
// the newest version and where its packages are, each with its signature. The Release
// workflow runs it and puts the result in the `updates` branch.
//
//   node scripts/update-manifest.mjs v0.2.1a <deb.sig> <AppImage.sig> > latest.json
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const { displayVersion } = require('./stage.cjs')
const pkg = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'))

const [tag, debSignature, appImageSignature] = process.argv.slice(2)
const shown = displayVersion(pkg)
if (tag !== `v${shown}` || !debSignature || !appImageSignature) {
  console.error('Usage: node scripts/update-manifest.mjs <tag matching package.json> <deb.sig> <AppImage.sig>')
  process.exit(1)
}

const base = `https://github.com/PEEPWICK/peepwick-browser/releases/download/${tag}`
const signature = (path) => readFileSync(path, 'utf8').trim()
const appImage = { signature: signature(appImageSignature), url: `${base}/peepwick-browser-${shown}-x86_64.AppImage` }
const deb = { signature: signature(debSignature), url: `${base}/peepwick-browser-${shown}-amd64.deb` }

const manifest = {
  version: pkg.version,
  // The version people see, with the stage suffix; the browser shows it while updating.
  displayVersion: shown,
  notes: `PEEPWICK Browser ${shown}`,
  pub_date: new Date().toISOString(),
  platforms: {
    'linux-x86_64-appimage': appImage,
    'linux-x86_64-deb': deb,
    'linux-x86_64': appImage,
  },
}
console.log(JSON.stringify(manifest, null, 2))
