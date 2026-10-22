import { app } from 'electron'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { displayVersion, isReleaseStage, type ReleaseStage } from '@shared/version'

export const isDev = !app.isPackaged

/** Files shipped next to the app (fonts for reader view, the window icon). */
export function resourcePath(...parts: string[]): string {
  const base = app.isPackaged ? join(process.resourcesPath, 'resources') : join(app.getAppPath(), 'resources')
  return join(base, ...parts)
}

export function rendererPath(...parts: string[]): string {
  return join(app.getAppPath(), 'out', 'renderer', ...parts)
}

export function preloadPath(): string {
  return join(app.getAppPath(), 'out', 'preload', 'ui.cjs')
}

export function userPath(...parts: string[]): string {
  return join(app.getPath('userData'), ...parts)
}

let stage: ReleaseStage | null = null

/** The release stage from package.json ("alpha" while the browser is in alpha). */
export function releaseStage(): ReleaseStage {
  if (!stage) {
    try {
      const pkg = JSON.parse(readFileSync(join(app.getAppPath(), 'package.json'), 'utf8')) as { releaseStage?: unknown }
      stage = isReleaseStage(pkg.releaseStage) ? pkg.releaseStage : 'stable'
    } catch {
      stage = 'stable'
    }
  }
  return stage
}

/** The version people see, such as 0.1.3a. */
export function shownVersion(version = app.getVersion()): string {
  return displayVersion(version, releaseStage())
}

/** The electron-vite dev server, when running `npm run dev`. */
export function devServerURL(): string | null {
  return isDev ? (process.env['ELECTRON_RENDERER_URL'] ?? null) : null
}
