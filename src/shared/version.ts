// The version people see: package.json keeps SemVer (0.1.3) for the build tools and the
// updater, and the release stage adds a suffix (alpha → 0.1.3a). Mirrors scripts/stage.cjs.
export type ReleaseStage = 'alpha' | 'beta' | 'stable'

const SUFFIX: Record<ReleaseStage, string> = { alpha: 'a', beta: 'b', stable: '' }

export function isReleaseStage(value: unknown): value is ReleaseStage {
  return value === 'alpha' || value === 'beta' || value === 'stable'
}

export function displayVersion(version: string, stage: ReleaseStage): string {
  return `${version}${SUFFIX[stage]}`
}
