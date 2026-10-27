// Over-the-air updates from GitHub Releases (PEEPWICK/peepwick-browser) through
// electron-updater. AppImage and .deb installs update themselves; updates download in
// the background and install on restart.
import { app } from 'electron'
import { appendFileSync, mkdirSync } from 'node:fs'
import { dirname } from 'node:path'
import updaterPackage from 'electron-updater'
import type { UpdateState } from '@shared/types'
import { bus } from './events'
import { recordError } from './diagnostics'
import { shownVersion, userPath } from './paths'

const { autoUpdater } = updaterPackage
const CHECK_EVERY = 4 * 60 * 60 * 1000

let state: UpdateState = { status: 'idle', version: null, progress: 0, error: null }
let started = false

function set(patch: Partial<UpdateState>): void {
  state = { ...state, ...patch }
  bus.emit('global')
}

export function updateState(): UpdateState {
  return state
}

function supported(): boolean {
  if (!app.isPackaged) return false
  // electron-updater handles AppImage (APPIMAGE is set) and .deb installs on Linux.
  return process.platform !== 'linux' || !!process.env['APPIMAGE'] || isDebInstall()
}

function isDebInstall(): boolean {
  return process.execPath.startsWith('/opt/')
}

function log(level: string, args: unknown[]): void {
  const path = userPath('logs', 'updater.log')
  try {
    mkdirSync(dirname(path), { recursive: true })
    appendFileSync(path, `${new Date().toISOString()} ${level} ${args.map(String).join(' ')}\n`)
  } catch {
    // Logging must never break updates.
  }
}

export function startUpdater(): void {
  if (started) return
  started = true
  if (!supported()) {
    state = { ...state, status: 'unsupported' }
    return
  }
  autoUpdater.logger = {
    info: (...args: unknown[]) => log('info', args),
    warn: (...args: unknown[]) => log('warn', args),
    error: (...args: unknown[]) => log('error', args),
    debug: () => {},
  }
  autoUpdater.autoDownload = true
  autoUpdater.autoInstallOnAppQuit = true
  autoUpdater.allowPrerelease = false

  autoUpdater.on('checking-for-update', () => set({ status: 'checking', error: null }))
  autoUpdater.on('update-available', (info) => set({ status: 'available', version: shownVersion(info.version), progress: 0 }))
  autoUpdater.on('update-not-available', () => set({ status: 'latest', progress: 0 }))
  autoUpdater.on('download-progress', (progress) => set({ status: 'downloading', progress: Math.round(progress.percent) }))
  autoUpdater.on('update-downloaded', (info) => set({ status: 'ready', version: shownVersion(info.version), progress: 100 }))
  autoUpdater.on('error', (error) => {
    set({ status: 'error', error: error?.message ?? String(error) })
    recordError('update', error?.message ?? String(error))
  })

  setTimeout(() => void checkForUpdates(), 15_000)
  setInterval(() => void checkForUpdates(), CHECK_EVERY)
}

export async function checkForUpdates(): Promise<void> {
  if (!supported()) {
    set({ status: 'unsupported' })
    return
  }
  if (state.status === 'downloading' || state.status === 'ready') return
  try {
    await autoUpdater.checkForUpdates()
  } catch (error) {
    set({ status: 'error', error: error instanceof Error ? error.message : String(error) })
  }
}

export function installUpdate(): void {
  if (state.status === 'ready') autoUpdater.quitAndInstall(false, true)
}
