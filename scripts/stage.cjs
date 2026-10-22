// Release stage naming, shared by the build config and the release script.
// package.json keeps a SemVer version (0.1.3); the stage adds the suffix people see:
// alpha → 0.1.3a, beta → 0.1.3b, stable → 0.1.3. Mirrors src/shared/version.ts.
const SUFFIX = { alpha: 'a', beta: 'b', stable: '' }

function stageOf(pkg) {
  const stage = pkg.releaseStage ?? 'stable'
  if (!(stage in SUFFIX)) throw new Error(`Unknown releaseStage "${stage}" in package.json`)
  return stage
}

function displayVersion(pkg) {
  return `${pkg.version}${SUFFIX[stageOf(pkg)]}`
}

module.exports = { SUFFIX, stageOf, displayVersion }
