// Routes calls from the browser UI. Only the UI views this process created may call
// in; web pages have no bridge at all.
import { app, ipcMain, type IpcMainEvent, type IpcMainInvokeEvent } from 'electron'
import { execFile } from 'node:child_process'
import { IPC_INVOKE, IPC_SEND, type BookmarkOpen, type ClearDataOptions, type InvokeMethods, type SendMethods } from '@shared/ipc'
import { WORKSPACE_ICONS, type AppInfo, type CommandId, type InternalPage, type PaneRect, type PermissionKind, type WorkspaceIcon } from '@shared/types'
import { bookmarkCurrentPage, bookmarkSpaceTabs, isCommandAvailable, newTab, openPage, runCommand } from './actions'
import {
  bookmarkTree,
  createBookmark,
  exportBookmarks,
  moveBookmarks,
  openBookmarks,
  removeAndTell,
  showBookmarkMenu,
  showFolderMenu,
  sortFolder,
  undoRemove,
  updateBookmark,
} from './bookmarks'
import { detectSources, importBookmarks } from './importers'
import { links } from '@shared/bookmarks'
import { clearDownloads, downloadAction } from './downloads'
import { listExtensions, removeExtension } from './extensions'
import { startFocus, stopFocus } from './focus'
import { clearHistory, getFavicon, matchPages, queryHistory, removeVisit, topSites } from './history'
import { showAppMenu, showTabMenu } from './menus'
import { resolveInput } from '@shared/input'
import { commandSuggestions, suggest } from './omnibox'
import { clearPermissions, listPermissions, removePermission, respond } from './permissions'
import { releaseStage, shownVersion } from './paths'
import { browsingSession } from './sessions'
import { settings, updateSettings } from './settings'
import { siteOf } from './shield'
import { stats } from './stats'
import { checkForUpdates, installUpdate } from './updater'
import { BrowserWin } from './window'
import { globalState } from './windows'
import { hostOf } from '@shared/url'

const PAGES: InternalPage[] = ['newtab', 'settings', 'history', 'extensions', 'bookmarks']
const OPEN_MODES: BookmarkOpen[] = ['current', 'tab', 'background', 'window', 'incognito']

const ids = (value: unknown): string[] =>
  Array.isArray(value) ? value.filter((id): id is string => typeof id === 'string').slice(0, 5000) : []

/** Bookmarks whose title or address contains the text, closest matches first. */
function matchBookmarks(text: string, limit: number): { title: string; url: string; favicon: string | null }[] {
  const needle = text.trim().toLocaleLowerCase('tr').replace(/^https?:\/\/(www\.)?/, '')
  if (!needle) return []
  const scored: { title: string; url: string; score: number }[] = []
  for (const link of links(bookmarkTree())) {
    const title = link.title.toLocaleLowerCase('tr')
    const url = link.url.toLowerCase().replace(/^https?:\/\/(www\.)?/, '')
    const score = url.startsWith(needle) ? 3 : title.startsWith(needle) ? 2 : title.includes(needle) || url.includes(needle) ? 1 : 0
    if (score > 0) scored.push({ title: link.title, url: link.url, score })
  }
  return scored
    .sort((a, b) => b.score - a.score)
    .slice(0, limit)
    .map(({ title, url }) => ({ title, url, favicon: getFavicon(hostOf(url)) }))
}

function windowFor(event: IpcMainEvent | IpcMainInvokeEvent): BrowserWin {
  for (const win of BrowserWin.all.values()) {
    if (win.ui.webContents.id === event.sender.id && event.senderFrame?.parent === null) return win
  }
  throw new Error('Unknown sender')
}

const num = (value: unknown): number => (typeof value === 'number' && Number.isFinite(value) ? value : NaN)
const str = (value: unknown, max = 4096): string => (typeof value === 'string' ? value.slice(0, max) : '')

function isDefaultBrowser(): boolean {
  return app.isDefaultProtocolClient('https') && app.isDefaultProtocolClient('http')
}

async function makeDefaultBrowser(): Promise<boolean> {
  const desktop = 'peepwick-browser.desktop'
  await new Promise<void>((resolve) => {
    execFile('xdg-settings', ['set', 'default-web-browser', desktop], () => resolve())
  })
  app.setAsDefaultProtocolClient('http')
  app.setAsDefaultProtocolClient('https')
  return isDefaultBrowser()
}

export function appInfo(): AppInfo {
  return {
    version: shownVersion(),
    stage: releaseStage(),
    electron: process.versions.electron,
    chrome: process.versions.chrome,
    node: process.versions.node,
    platform: `${process.platform} ${process.arch}`,
    packaged: app.isPackaged,
    isDefaultBrowser: isDefaultBrowser(),
  }
}

const invoke: { [K in keyof InvokeMethods]: (win: BrowserWin, ...args: unknown[]) => unknown } = {
  ready(win) {
    win.markUIReady()
    win.updateTitle()
    return { state: win.toState(), global: globalState(), settings: settings(), app: appInfo(), bookmarks: bookmarkTree() }
  },
  async suggest(win, input) {
    const text = str(input, 500)
    const prefs = settings()
    return suggest(text, {
      lang: prefs.language,
      engine: prefs.searchEngine,
      remoteSuggestions: prefs.searchSuggestions && !win.incognito,
      tabs: win.incognito
        ? []
        : [...win.tabs.values()]
            .filter((tab) => tab !== win.activeTab() && !tab.page)
            .map((tab) => ({ id: tab.id, title: tab.displayTitle(), url: tab.url, favicon: tab.favicon })),
      bookmarks: win.incognito ? [] : matchBookmarks(text, 4),
      pages: win.incognito
        ? []
        : matchPages(text).map((page) => ({ ...page, favicon: getFavicon(hostOf(page.url)) })),
    })
  },
  palette(win, input) {
    const text = str(input, 200)
    const prefs = settings()
    const commands = commandSuggestions(text, prefs.language, (id) => isCommandAvailable(win, id))
    const needle = text.trim().toLocaleLowerCase('tr')
    const tabs = [...win.tabs.values()]
      .filter((tab) => !needle || tab.displayTitle().toLocaleLowerCase('tr').includes(needle) || tab.url.toLowerCase().includes(needle))
      .slice(0, 8)
      .map((tab) => ({ kind: 'tab' as const, title: tab.displayTitle(), value: tab.displayURL(), favicon: tab.favicon, tabId: tab.id }))
    const pages =
      needle && !win.incognito
        ? matchPages(text, 6).map((page) => ({
            kind: 'history' as const,
            title: page.title || page.url,
            value: page.url,
            favicon: getFavicon(hostOf(page.url)),
          }))
        : []
    const marks =
      needle && !win.incognito
        ? matchBookmarks(text, 6).map((b) => ({ kind: 'bookmark' as const, title: b.title, value: b.url, favicon: b.favicon }))
        : []
    return [...commands.slice(0, needle ? 6 : 8), ...tabs, ...marks, ...pages.filter((p) => !marks.some((m) => m.value === p.value))]
  },
  historyQuery(win, query) {
    if (win.incognito) return []
    const q = (query ?? {}) as { text?: unknown; before?: unknown; limit?: unknown }
    const before = num(q.before)
    const limit = num(q.limit)
    return queryHistory(str(q.text, 200), Number.isNaN(before) ? undefined : before, Number.isNaN(limit) ? 100 : limit)
  },
  topSites(win) {
    return win.incognito ? [] : topSites(8)
  },
  stats() {
    return stats()
  },
  extensionsList(win) {
    return win.incognito ? [] : listExtensions()
  },
  permissionsList() {
    return listPermissions()
  },
  async clearData(_win, options) {
    const o = (options ?? {}) as Partial<ClearDataOptions>
    const ses = browsingSession()
    if (o.history) clearHistory()
    if (o.cookies) {
      await ses.clearStorageData()
      clearPermissions()
    }
    if (o.cache) await ses.clearCache()
    return true
  },
  appInfo() {
    return appInfo()
  },
  async setDefaultBrowser() {
    return makeDefaultBrowser()
  },
  bookmarkCreate(_win, input) {
    const i = (input ?? {}) as { parentId?: unknown; index?: unknown; title?: unknown; url?: unknown }
    const index = num(i.index)
    return createBookmark({
      parentId: str(i.parentId, 64),
      index: Number.isNaN(index) ? undefined : index,
      title: str(i.title, 500),
      url: typeof i.url === 'string' ? i.url.slice(0, 8192) : undefined,
    })
  },
  bookmarkPage(win) {
    return bookmarkCurrentPage(win)
  },
  importSources(win) {
    return win.incognito ? [] : detectSources()
  },
  async importBookmarks(win, sourceId, skipDuplicates) {
    return importBookmarks(win, str(sourceId, 64), skipDuplicates !== false)
  },
  async exportBookmarks(win) {
    return exportBookmarks(win)
  },
  shieldReport(win) {
    const tab = win.activeTab()
    if (!tab) return []
    return [...tab.blockedHosts.entries()]
      .map(([host, count]) => ({ host, count }))
      .sort((a, b) => b.count - a.count)
      .slice(0, 12)
  },
}

function tabOf(win: BrowserWin, id: unknown) {
  return win.tabs.get(num(id))
}

const send: { [K in keyof SendMethods]: (win: BrowserWin, ...args: unknown[]) => void } = {
  windowControl(win, action) {
    if (action === 'minimize') win.base.minimize()
    else if (action === 'maximize') {
      if (win.base.isMaximized()) win.base.unmaximize()
      else win.base.maximize()
    } else if (action === 'close') win.base.close()
  },
  setPanes(win, rects) {
    if (!Array.isArray(rects)) return
    win.setPanes(
      rects.slice(0, 4).map((r) => {
        const rect = (r ?? {}) as Partial<PaneRect>
        return { tabId: num(rect.tabId), x: num(rect.x) || 0, y: num(rect.y) || 0, width: num(rect.width) || 0, height: num(rect.height) || 0 }
      }),
    )
  },
  setOverlay(win, open) {
    win.setOverlay(open === true)
  },
  appMenu(win, x, y) {
    showAppMenu(win, num(x) || 0, num(y) || 0)
  },
  newTab(win, options) {
    const o = (options ?? {}) as { url?: unknown; background?: unknown }
    if (typeof o.url === 'string' && o.url) {
      win.openTab({ url: resolveInput(o.url, settings().searchEngine), background: o.background === true })
    } else newTab(win)
  },
  closeTab(win, id) {
    const tab = tabOf(win, id)
    if (tab) win.closeTab(tab)
  },
  activateTab(win, id) {
    const tab = tabOf(win, id)
    if (tab) win.activateTab(tab)
  },
  navigate(win, id, input) {
    const tab = tabOf(win, id)
    const url = resolveInput(str(input), settings().searchEngine)
    if (!tab || !url) return
    tab.load(url)
    win.focusActive()
  },
  back(win, id) {
    tabOf(win, id)?.back()
  },
  forward(win, id) {
    tabOf(win, id)?.forward()
  },
  reload(win, id, hard) {
    tabOf(win, id)?.reload(hard === true)
  },
  stop(win, id) {
    tabOf(win, id)?.stop()
  },
  togglePin(win, id) {
    const tab = tabOf(win, id)
    if (tab) win.togglePin(tab)
  },
  toggleMute(win, id) {
    const tab = tabOf(win, id)
    if (tab) tab.setMuted(!tab.muted)
  },
  moveTab(win, id, workspaceId, index, pinned) {
    const tab = tabOf(win, id)
    if (tab) win.moveTab(tab, str(workspaceId, 64), num(index) || 0, typeof pinned === 'boolean' ? pinned : undefined)
  },
  tabMenu(win, id, x, y) {
    const tab = tabOf(win, id)
    if (tab) showTabMenu(win, tab, num(x) || 0, num(y) || 0)
  },
  openPage(win, page, hash) {
    if (PAGES.includes(page as InternalPage)) openPage(win, page as InternalPage, str(hash, 40))
  },
  openURL(win, url, newTabFlag) {
    const target = str(url)
    if (!/^(https?|peepwick|chrome-extension):/.test(target)) return
    if (newTabFlag === true) win.openTab({ url: target })
    else win.activeTab()?.load(target)
  },
  split(win, left, right) {
    const a = tabOf(win, left)
    const b = tabOf(win, right)
    if (a && b) win.split(a, b)
  },
  unsplit(win) {
    win.unsplit()
  },
  splitRatio(win, ratio) {
    win.setSplitRatio(num(ratio))
  },
  focusTab(win, id) {
    const tab = tabOf(win, id)
    if (tab) {
      win.paneFocused(tab)
      tab.webContents?.focus()
    }
  },
  createWorkspace(win, name, icon) {
    win.createWorkspace(str(name, 40), (WORKSPACE_ICONS.includes(icon as WorkspaceIcon) ? icon : 'circle') as WorkspaceIcon)
  },
  updateWorkspace(win, id, patch) {
    const p = (patch ?? {}) as { name?: unknown; icon?: unknown }
    win.updateWorkspace(str(id, 64), {
      name: typeof p.name === 'string' ? p.name : undefined,
      icon: WORKSPACE_ICONS.includes(p.icon as WorkspaceIcon) ? (p.icon as WorkspaceIcon) : undefined,
    })
  },
  deleteWorkspace(win, id) {
    win.deleteWorkspace(str(id, 64))
  },
  activateWorkspace(win, id) {
    win.activateWorkspace(str(id, 64))
  },
  find(win, text, forward, findNext) {
    win.activeTab()?.findInPage(str(text, 500), forward !== false, findNext === true)
  },
  stopFind(win) {
    win.activeTab()?.stopFind()
  },
  zoom(win, direction) {
    if (direction === 'in' || direction === 'out' || direction === 'reset') win.activeTab()?.zoom(direction)
  },
  toggleReader(win) {
    void win.activeTab()?.toggleReader()
  },
  shieldToggleSite(win) {
    const tab = win.activeTab()
    const site = tab && !tab.page ? siteOf(tab.url) : null
    if (!site) return
    const list = settings().shieldAllowlist
    updateSettings({ shieldAllowlist: list.includes(site) ? list.filter((s) => s !== site) : [...list, site] })
    tab?.reload()
  },
  focusStart(_win, minutes) {
    startFocus(num(minutes) || settings().focusMinutes)
  },
  focusStop() {
    stopFocus()
  },
  interstitial(win, tabId, action) {
    const tab = tabOf(win, tabId)
    if (!tab) return
    if (action === 'retry') tab.retry()
    else if (action === 'proceed') tab.proceedInsecure()
    else if (action === 'back') tab.back()
    else if (action === 'open-anyway') tab.openAnyway()
  },
  command(win, id) {
    if (typeof id === 'string' && isCommandAvailable(win, id as CommandId)) runCommand(win, id as CommandId)
  },
  permission(_win, id, allow, remember) {
    respond(num(id), allow === true, remember === true)
  },
  download(_win, id, action) {
    if (['open', 'show', 'cancel', 'pause', 'resume', 'remove'].includes(action as string)) {
      downloadAction(str(id, 64), action as 'open')
    }
  },
  downloadsClear() {
    clearDownloads()
  },
  updateCheck() {
    void checkForUpdates()
  },
  updateInstall() {
    installUpdate()
  },
  setSettings(_win, patch) {
    updateSettings(patch)
  },
  historyRemove(_win, id) {
    removeVisit(num(id))
  },
  historyClear() {
    clearHistory()
  },
  extensionRemove(_win, id) {
    void removeExtension(str(id, 64))
  },
  extensionOptions(win, id) {
    const extension = listExtensions().find((e) => e.id === id)
    if (extension?.optionsUrl) win.openTab({ url: extension.optionsUrl })
  },
  permissionRemove(_win, origin, kind) {
    removePermission(str(origin, 500), kind as PermissionKind)
  },
  bookmarkUpdate(_win, id, patch) {
    const p = (patch ?? {}) as { title?: unknown; url?: unknown }
    updateBookmark(str(id, 64), {
      title: typeof p.title === 'string' ? p.title : undefined,
      url: typeof p.url === 'string' ? p.url.slice(0, 8192) : undefined,
    })
  },
  bookmarkMove(_win, list, parentId, index) {
    moveBookmarks(ids(list), str(parentId, 64), num(index) || 0)
  },
  bookmarkRemove(win, list) {
    removeAndTell(win, ids(list))
  },
  bookmarkUndo() {
    undoRemove()
  },
  bookmarkSort(_win, folderId) {
    sortFolder(str(folderId, 64))
  },
  bookmarkOpen(win, list, how) {
    if (OPEN_MODES.includes(how as BookmarkOpen)) void openBookmarks(win, ids(list), how as BookmarkOpen)
  },
  bookmarkMenu(win, id, x, y) {
    showBookmarkMenu(win, str(id, 64), num(x) || 0, num(y) || 0)
  },
  bookmarkFolderMenu(win, folderId, x, y, fromIndex) {
    showFolderMenu(win, str(folderId, 64), num(x) || 0, num(y) || 0, num(fromIndex) || 0)
  },
  bookmarkTabs(win) {
    bookmarkSpaceTabs(win)
  },
  onboardingDone() {
    updateSettings({ onboarded: true })
  },
}

export function installIPC(): void {
  ipcMain.handle(IPC_INVOKE, async (event, method: unknown, ...args: unknown[]) => {
    const win = windowFor(event)
    const handler = typeof method === 'string' ? invoke[method as keyof InvokeMethods] : undefined
    if (!handler) throw new Error(`Unknown method ${String(method)}`)
    return handler(win, ...args)
  })
  ipcMain.on(IPC_SEND, (event, method: unknown, ...args: unknown[]) => {
    let win: BrowserWin
    try {
      win = windowFor(event)
    } catch {
      return
    }
    const handler = typeof method === 'string' ? send[method as keyof SendMethods] : undefined
    if (!handler) return
    try {
      handler(win, ...args)
    } catch (error) {
      console.error('[ipc]', method, error)
    }
  })
}
