// Actions shared by keyboard shortcuts, menus, the command bar and the UI.
import { app, dialog } from 'electron'
import { NEW_TAB_URL } from '@shared/url'
import type { CommandId, InternalPage } from '@shared/types'
import { focusState, startFocus, stopFocus } from './focus'
import { settings, updateSettings } from './settings'
import { checkForUpdates } from './updater'
import type { BrowserWin } from './window'
import { createWindow } from './windows'

export function newTab(win: BrowserWin, url = NEW_TAB_URL): void {
  win.openTab({ url })
  if (url === NEW_TAB_URL) win.command({ type: 'focus-omnibox' })
}

export function openPage(win: BrowserWin, page: InternalPage, hash = ''): void {
  const active = win.activeTab()
  if (active?.page === page || active?.page === 'newtab') {
    active.showPage(page, hash)
    return
  }
  const existing = win.tabsInOrder().find((tab) => tab.page === page)
  if (existing) {
    existing.showPage(page, hash)
    win.activateTab(existing)
    return
  }
  const tab = win.openTab({ url: `peepwick://${page}` })
  tab.showPage(page, hash)
}

export function toggleSplit(win: BrowserWin): void {
  const workspace = win.activeWorkspace()
  if (workspace.split) {
    win.unsplit()
    return
  }
  const active = win.activeTab()
  const other = win.previousTab()
  if (active && other) win.split(other, active)
}

export function toggleFullscreen(win: BrowserWin): void {
  win.base.setFullScreen(!win.base.isFullScreen())
}

export function openDevTools(win: BrowserWin): void {
  const tab = win.activeTab()
  const wc = tab?.reader?.webContents ?? tab?.webContents
  if (!wc || tab?.page) return
  if (wc.isDevToolsOpened()) wc.closeDevTools()
  else wc.openDevTools({ mode: 'detach' })
}

export async function savePage(win: BrowserWin): Promise<void> {
  const wc = win.activeTab()?.webContents
  if (!wc) return
  const name = (wc.getTitle() || 'page').replace(/[/\\?%*:|"<>]/g, '-').slice(0, 120)
  const result = await dialog.showSaveDialog(win.base, {
    defaultPath: `${app.getPath('downloads')}/${name}.html`,
    filters: [{ name: 'HTML', extensions: ['html', 'htm'] }],
  })
  if (!result.canceled && result.filePath) await wc.savePage(result.filePath, 'HTMLComplete').catch(() => {})
}

export function isCommandAvailable(win: BrowserWin, id: CommandId): boolean {
  const active = win.activeTab()
  const workspace = win.activeWorkspace()
  switch (id) {
    case 'focus-start':
      return !focusState().active
    case 'focus-stop':
      return focusState().active
    case 'reader':
      return !!active && (active.readerAvailable || !!active.reader)
    case 'split':
      return !workspace.split && !!win.previousTab()
    case 'unsplit':
      return !!workspace.split
    case 'pin':
      return !!active && !active.page
    case 'extensions':
      return !win.incognito
    default:
      return true
  }
}

export function runCommand(win: BrowserWin, id: CommandId): void {
  switch (id) {
    case 'new-tab':
      newTab(win)
      break
    case 'new-window':
      createWindow({ incognito: false })
      break
    case 'new-incognito':
      createWindow({ incognito: true })
      break
    case 'focus-start':
      startFocus(settings().focusMinutes)
      break
    case 'focus-stop':
      stopFocus()
      break
    case 'reader':
      void win.activeTab()?.toggleReader()
      break
    case 'settings':
      openPage(win, 'settings')
      break
    case 'history':
      openPage(win, 'history')
      break
    case 'extensions':
      openPage(win, 'extensions')
      break
    case 'downloads':
      win.command({ type: 'downloads' })
      break
    case 'toggle-sidebar':
      updateSettings({ sidebarCollapsed: !settings().sidebarCollapsed })
      break
    case 'split':
    case 'unsplit':
      toggleSplit(win)
      break
    case 'pin': {
      const tab = win.activeTab()
      if (tab) win.togglePin(tab)
      break
    }
    case 'clear-data':
      openPage(win, 'settings', 'privacy')
      break
    case 'check-updates':
      void checkForUpdates()
      openPage(win, 'settings', 'about')
      break
  }
}
