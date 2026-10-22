import { app } from 'electron'
import { join } from 'node:path'

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

/** The electron-vite dev server, when running `npm run dev`. */
export function devServerURL(): string | null {
  return isDev ? (process.env['ELECTRON_RENDERER_URL'] ?? null) : null
}
