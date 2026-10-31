// Cuts a release: bumps the SemVer version (package.json and the Rust crate), commits, and
// tags it with the stage suffix.
//
//   npm run release -- patch      0.1.3 → 0.1.4, tag v0.1.4a while in alpha
//   npm run release -- minor
//   npm run release -- --print-tag   prints the tag for the current version (used by CI)
//
// Push afterwards with `git push --follow-tags`; the Release workflow builds and publishes.
import { execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const { displayVersion } = require('./stage.cjs')
const readPackage = () => JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'))
const git = (...args) => execFileSync('git', args, { encoding: 'utf8' }).trim()

const arg = process.argv[2] ?? 'patch'

if (arg === '--print-tag') {
  console.log(`v${displayVersion(readPackage())}`)
  process.exit(0)
}

if (!['patch', 'minor', 'major'].includes(arg)) {
  console.error('Usage: npm run release -- patch|minor|major')
  process.exit(1)
}
if (git('status', '--porcelain')) {
  console.error('Commit or stash your changes first.')
  process.exit(1)
}

execFileSync('npm', ['version', arg, '--no-git-tag-version'], { stdio: 'ignore' })
// The Rust crate carries the same SemVer version, in its manifest and lock file.
const cargoToml = new URL('../src-tauri/Cargo.toml', import.meta.url)
const { version } = readPackage()
writeFileSync(cargoToml, readFileSync(cargoToml, 'utf8').replace(/^version = ".*"$/m, `version = "${version}"`))
execFileSync('cargo', ['update', '--workspace', '--offline'], {
  cwd: new URL('../src-tauri/', import.meta.url),
  stdio: 'ignore',
})
const shown = displayVersion(readPackage())
const tag = `v${shown}`
git('add', 'package.json', 'package-lock.json', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock')
git('commit', '-m', `PEEPWICK Browser ${shown}`)
git('tag', '-a', tag, '-m', `PEEPWICK Browser ${shown}`)
console.log(`Tagged ${tag}. Publish it with: git push --follow-tags`)
