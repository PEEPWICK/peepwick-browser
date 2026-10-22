import { createRequire } from 'node:module'
import { execFileSync } from 'node:child_process'
import { describe, expect, it } from 'vitest'
import { displayVersion, isReleaseStage, type ReleaseStage } from '@shared/version'

const require = createRequire(import.meta.url)
const stage = require('../scripts/stage.cjs') as { displayVersion: (pkg: { version: string; releaseStage?: string }) => string }
const pkg = require('../package.json') as { version: string; releaseStage: string }

describe('release stage', () => {
  it('shows alpha and beta with a letter, stable without', () => {
    expect(displayVersion('0.1.3', 'alpha')).toBe('0.1.3a')
    expect(displayVersion('0.2.0', 'beta')).toBe('0.2.0b')
    expect(displayVersion('1.0.0', 'stable')).toBe('1.0.0')
  })

  it('agrees with the build scripts', () => {
    for (const s of ['alpha', 'beta', 'stable'] as ReleaseStage[]) {
      expect(stage.displayVersion({ version: '0.4.2', releaseStage: s })).toBe(displayVersion('0.4.2', s))
    }
  })

  it('keeps package.json valid', () => {
    expect(isReleaseStage(pkg.releaseStage)).toBe(true)
    expect(pkg.version).toMatch(/^\d+\.\d+\.\d+$/)
  })

  it('derives the tag the Release workflow expects', () => {
    const tag = execFileSync('node', ['scripts/release.mjs', '--print-tag'], { encoding: 'utf8' }).trim()
    expect(tag).toBe(`v${displayVersion(pkg.version, pkg.releaseStage as ReleaseStage)}`)
  })
})
