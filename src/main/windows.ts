// Creating windows, restoring the last session and keeping every UI in sync with
// app-wide state (focus, updates, downloads, settings).
import { app } from 'electron'
import type { GlobalState } from '@shared/types'
import { downloads } from './downloads'
import { bus } from './events'
import { focusState } from './focus'
import { userPath } from './paths'
import { browsingSession, incognitoSession, wipeIncognito } from './sessions'
import { settings } from './settings'
import { JsonFile } from './store'
import { updateState } from './updater'
import { BrowserWin, type SavedWindow } from './window'

interface SessionFile {
  version: 1
  windows: SavedWindow[]
}

const sessionFile = new JsonFile<SessionFile>(userPath('session.json'), 0)
let quitting = false
let saveTimer: NodeJS.Timeout | null = null

export function createWindow(options: { incognito: boolean; urls?: string[]; saved?: SavedWindow }): BrowserWin {
  const session = options.incognito ? incognitoSession() : browsingSession()
  const win = new BrowserWin(session, options.incognito, options.saved, options.urls ?? [])
  if (!options.incognito) {
    win.base.on('close', () => {
      // Closing the last window ends the session: remember it for next time.
      const others = [...BrowserWin.all.values()].filter((w) => w !== win && !w.incognito)
      if (!quitting && others.length === 0) saveSessionNow()
    })
  }
  win.base.on('closed', () => {
    if (options.incognito && ![...BrowserWin.all.values()].some((w) => w.incognito)) void wipeIncognito()
    if (!options.incognito && !quitting && [...BrowserWin.all.values()].some((w) => !w.incognito)) scheduleSave()
  })
  return win
}

export function restoreSession(urls: string[]): void {
  const saved = settings().restoreSession ? sessionFile.read() : null
  const windows = saved?.version === 1 && Array.isArray(saved.windows) ? saved.windows.filter((w) => w.workspaces?.length) : []
  if (windows.length === 0) {
    createWindow({ incognito: false, urls })
    return
  }
  windows.forEach((w, index) => createWindow({ incognito: false, saved: w, urls: index === windows.length - 1 ? urls : [] }))
}

function collect(): SessionFile {
  return {
    version: 1,
    windows: [...BrowserWin.all.values()].filter((w) => !w.incognito && !w.base.isDestroyed()).map((w) => w.serialize()),
  }
}

export function scheduleSave(): void {
  if (saveTimer) return
  saveTimer = setTimeout(() => {
    saveTimer = null
    if (!quitting) saveSessionNow()
  }, 3000)
}

export function saveSessionNow(): void {
  if (saveTimer) clearTimeout(saveTimer)
  saveTimer = null
  const data = collect()
  if (data.windows.length > 0 || !settings().restoreSession) sessionFile.writeNow(data)
}

export function markQuitting(): void {
  if (!quitting) saveSessionNow()
  quitting = true
}

export function globalState(): GlobalState {
  return { focus: focusState(), update: updateState(), downloads: downloads(), version: app.getVersion() }
}

let globalTimer: NodeJS.Timeout | null = null

export function wireBroadcasts(): void {
  bus.on('global', () => {
    if (globalTimer) return
    globalTimer = setTimeout(() => {
      globalTimer = null
      const state = globalState()
      for (const win of BrowserWin.all.values()) win.send('global', state)
    }, 30)
  })
  bus.on('settings', () => {
    const current = settings()
    for (const win of BrowserWin.all.values()) {
      win.send('settings', current)
      win.markDirty()
    }
  })
  bus.on('window-dirty', (id) => {
    if (!BrowserWin.all.get(id)?.incognito) scheduleSave()
  })
}

export function anyWindow(): BrowserWin | null {
  return BrowserWin.lastFocused ?? [...BrowserWin.all.values()][0] ?? null
}
