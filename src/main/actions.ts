// Actions shared by keyboard shortcuts, menus, the command bar and the UI.
import { app, dialog } from 'electron'
import { NEW_TAB_URL } from '@shared/url'
import type { CommandId, InternalPage } from '@shared/types'
import { bookmarkPage, bookmarkTabs } from './bookmarks'
import { addPage } from './readinglist'
import { focusState, startFocus, stopFocus } from './focus'
import { settings, t, updateSettings } from './settings'
import { checkForUpdates } from './updater'
import type { BrowserWin } from './window'
import { createWindow } from './windows'

export function newTab(win: BrowserWin, url = NEW_TAB_URL): void {
  win.openTab({ url })
  // The new tab page has its own search box; typing goes straight there.
  if (url === NEW_TAB_URL) win.command({ type: 'focus-page-search' })
}

export function openPage(win: BrowserWin, page: InternalPage, subpage = ''): void {
  const active = win.activeTab()
  if (active?.page === page || active?.page === 'newtab') {
    active.showPage(page, subpage)
    return
  }
  const existing = win.tabsInOrder().find((tab) => tab.page === page)
  if (existing) {
    existing.showPage(page, subpage)
    win.activateTab(existing)
    return
  }
  const tab = win.openTab({ url: `peepwick://${page}` })
  tab.showPage(page, subpage)
}

/** Ctrl+D: bookmarks the active page and opens the star panel to edit it. */
export function bookmarkCurrentPage(win: BrowserWin): { id: string; created: boolean } | null {
  const tab = win.activeTab()
  if (!tab || tab.page || win.incognito) return null
  const result = bookmarkPage(tab.url, tab.displayTitle())
  if (result) win.command({ type: 'bookmark-star', created: result.created })
  return result
}

export function bookmarkSpaceTabs(win: BrowserWin): void {
  const workspace = win.activeWorkspace()
  const tabs = win.tabsInOrder(workspace).filter((tab) => !tab.page).map((tab) => ({ url: tab.url, title: tab.displayTitle() }))
  const id = bookmarkTabs(tabs, workspace.name)
  if (id) win.command({ type: 'bookmark-edit', id })
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
    case 'translate':
      return !!active && !active.page && !active.interstitial && /^https?:/.test(active.url)
    case 'reading-add':
      return !win.incognito && !!active && !active.page && !active.interstitial && /^https?:/.test(active.url)
    case 'reading-list':
    case 'extensions':
    case 'import-bookmarks':
    case 'backup':
    case 'passwords':
      return !win.incognito
    case 'bookmark-page':
      return !!active && !active.page && /^(https?|file):/.test(active.url) && !win.incognito
    case 'bookmark-tabs':
      return !win.incognito && win.tabsInOrder().some((tab) => !tab.page)
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
      openPage(win, 'downloads')
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
      win.command({ type: 'clear-data' })
      break
    case 'report':
      openPage(win, 'settings', 'report')
      break
    case 'backup':
      openPage(win, 'settings', 'backup')
      break
    case 'passwords':
      openPage(win, 'passwords')
      break
    case 'task-manager':
      win.command({ type: 'task-manager' })
      break
    case 'reading-list':
      openPage(win, 'reading')
      break
    case 'translate':
      if (isCommandAvailable(win, 'translate')) void win.activeTab()?.offerTranslation()
      break
    case 'reading-add': {
      const contents = win.activeTab()?.webContents
      if (contents && isCommandAvailable(win, 'reading-add')) {
        void addPage(contents).then((item) => item && win.command({ type: 'toast', text: t(item.saved ? 'reading.addedOffline' : 'reading.added') }))
      }
      break
    }
    case 'check-updates':
      void checkForUpdates()
      openPage(win, 'settings', 'about')
      break
    case 'bookmark-page':
      bookmarkCurrentPage(win)
      break
    case 'bookmark-tabs':
      bookmarkSpaceTabs(win)
      break
    case 'bookmarks':
      openPage(win, 'bookmarks')
      break
    case 'toggle-bookmarks-bar':
      updateSettings({ bookmarksBar: settings().bookmarksBar === 'never' ? 'always' : 'never' })
      break
    case 'import-bookmarks':
      win.command({ type: 'import-bookmarks' })
      break
  }
}
