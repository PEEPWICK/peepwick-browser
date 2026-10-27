// Menus: the page context menu, the tab menu in the sidebar and the app menu. The UI
// draws them (popup.ts).
import { app, clipboard, type ContextMenuParams, type WebContents } from 'electron'
import { searchURL } from '@shared/search'
import { hostOf } from '@shared/url'
import { focusState, startFocus, stopFocus } from './focus'
import { getExtensions } from './extensions'
import { isCommandAvailable, newTab, openDevTools, openPage, runCommand, savePage } from './actions'
import { createBookmark, exportBookmarks } from './bookmarks'
import { allDownloads, downloadAction } from './downloads'
import { deleteHost, removeVisits } from './history'
import { confirm, fromElectron, popupMenu, separator as sep, type MenuSpec } from './popup'
import { settings, t } from './settings'
import type { Tab } from './tab'
import { BrowserWin } from './window'
import { createWindow } from './windows'

export function showPageMenu(tab: Tab, wc: WebContents, params: ContextMenuParams): void {
  const win = tab.win
  const items: MenuSpec[] = []

  if (params.misspelledWord) {
    for (const suggestion of params.dictionarySuggestions.slice(0, 5)) {
      items.push({ label: suggestion, click: () => wc.replaceMisspelling(suggestion) })
    }
    items.push({
      label: t('ctx.addToDictionary'),
      click: () => wc.session.addWordToSpellCheckerDictionary(params.misspelledWord),
    })
    items.push(sep)
  }

  const link = params.linkURL
  if (link && /^(https?|ftp|file|mailto):/.test(link)) {
    items.push(
      { label: t('ctx.openLinkNewTab'), click: () => win.openTab({ url: link, background: true, opener: tab }) },
      {
        label: t('ctx.openLinkSplit'),
        click: () => {
          const other = win.openTab({ url: link, background: true, opener: tab })
          win.split(tab, other)
        },
      },
      { label: t('ctx.openLinkIncognito'), click: () => createWindow({ incognito: true, urls: [link] }) },
      { label: t('ctx.copyLink'), click: () => clipboard.writeText(link) },
      ...(win.incognito || !/^(https?|ftp|file):/.test(link)
        ? []
        : [{ label: t('ctx.bookmarkLink'), click: () => void createBookmark({ parentId: 'bar', title: params.linkText.trim() || link, url: link }) }]),
      sep,
    )
  }

  if (params.mediaType === 'image' && params.srcURL) {
    const src = params.srcURL
    items.push(
      { label: t('ctx.openImage'), click: () => win.openTab({ url: src, background: true, opener: tab }) },
      { label: t('ctx.saveImage'), click: () => wc.downloadURL(src) },
      { label: t('ctx.copyImage'), click: () => wc.copyImageAt(params.x, params.y) },
      { label: t('ctx.copyImageAddress'), click: () => clipboard.writeText(src) },
      sep,
    )
  } else if ((params.mediaType === 'video' || params.mediaType === 'audio') && params.srcURL) {
    const src = params.srcURL
    items.push({ label: t('ctx.saveMedia'), click: () => wc.downloadURL(src) }, sep)
  }

  if (params.isEditable) {
    const flags = params.editFlags
    items.push(
      { label: t('ctx.cut'), accelerator: 'Ctrl+X', enabled: flags.canCut, click: () => wc.cut() },
      { label: t('ctx.copy'), accelerator: 'Ctrl+C', enabled: flags.canCopy, click: () => wc.copy() },
      { label: t('ctx.paste'), accelerator: 'Ctrl+V', enabled: flags.canPaste, click: () => wc.paste() },
      { label: t('ctx.selectAll'), accelerator: 'Ctrl+A', enabled: flags.canSelectAll, click: () => wc.selectAll() },
      sep,
    )
  } else if (params.selectionText.trim()) {
    const text = params.selectionText.trim()
    const short = text.length > 24 ? `${text.slice(0, 24)}…` : text
    items.push(
      { label: t('ctx.copy'), accelerator: 'Ctrl+C', click: () => wc.copy() },
      {
        label: t('ctx.searchFor', { text: short }),
        click: () => win.openTab({ url: searchURL(settings().searchEngine, text), opener: tab }),
      },
      sep,
    )
  }

  if (!link && !params.isEditable && !params.selectionText.trim() && params.mediaType === 'none') {
    items.push(
      { label: t('ctx.back'), accelerator: 'Alt+Left', enabled: tab.canGoBack(), click: () => tab.back() },
      { label: t('ctx.forward'), accelerator: 'Alt+Right', enabled: tab.canGoForward(), click: () => tab.forward() },
      { label: t('ctx.reload'), accelerator: 'Ctrl+R', click: () => tab.reload() },
      sep,
      { label: t('ctx.savePage'), accelerator: 'Ctrl+S', click: () => void savePage(win) },
      { label: t('ctx.print'), accelerator: 'Ctrl+P', click: () => wc.print() },
      sep,
    )
  }

  const extensionItems = win.incognito ? [] : (getExtensions()?.getContextMenuItems(wc, params) ?? [])
  items.push(sep, ...fromElectron(win, extensionItems, wc), sep, {
    label: t('ctx.inspect'),
    click: () => {
      if (!wc.isDevToolsOpened()) wc.openDevTools({ mode: 'detach' })
      wc.inspectElement(params.x, params.y)
    },
  })
  // Page coordinates are relative to the page's view.
  const view = tab.reader?.webContents === wc ? tab.reader.view : tab.view
  const origin = view?.getBounds() ?? { x: 0, y: 0 }
  popupMenu(win, items, origin.x + params.x, origin.y + params.y)
}

export function showTabMenu(win: BrowserWin, tab: Tab, x: number, y: number): void {
  const workspace = win.workspaceOf(tab)
  const active = win.activeTab()
  const tabs = workspace ? win.tabsInOrder(workspace) : []
  const index = tabs.indexOf(tab)
  const inSplit = !!workspace?.split && (workspace.split.left === tab.id || workspace.split.right === tab.id)
  const others = win.workspaces.filter((w) => w !== workspace)

  const items: MenuSpec[] = [
    { label: t('tabmenu.reload'), enabled: !tab.page, click: () => tab.reload() },
    { label: t('tabmenu.duplicate'), enabled: !tab.page, click: () => win.duplicateTab(tab) },
    { label: tab.pinned ? t('tabmenu.unpin') : t('tabmenu.pin'), click: () => win.togglePin(tab) },
    { label: tab.muted ? t('tabmenu.unmute') : t('tabmenu.mute'), click: () => tab.setMuted(!tab.muted) },
    ...(!win.incognito && !tab.page && /^(https?|file):/.test(tab.url)
      ? [{ label: t('tabmenu.bookmark'), click: () => void createBookmark({ parentId: 'bar', title: tab.displayTitle(), url: tab.url }) }]
      : []),
    sep,
    ...(active && active !== tab && !inSplit
      ? [{ label: t('tabmenu.split'), click: () => win.split(active, tab) }]
      : []),
    ...(inSplit ? [{ label: t('tabmenu.unsplit'), click: () => win.unsplit() }] : []),
    ...(others.length > 0
      ? [
          {
            label: t('tabmenu.moveTo'),
            submenu: others.map((w) => ({ label: w.name, click: () => win.moveTab(tab, w.id, w.tabIds.length) })),
          },
        ]
      : []),
    { label: t('tabmenu.sleep'), enabled: !!tab.view && !win.isShown(tab), click: () => win.sleepTab(tab) },
    { label: t('tabmenu.copyLink'), enabled: !tab.page, click: () => clipboard.writeText(tab.displayURL()) },
    sep,
    {
      label: t('tabmenu.closeOthers'),
      enabled: tabs.length > 1,
      click: () => tabs.filter((other) => other !== tab && !other.pinned).forEach((other) => win.closeTab(other)),
    },
    {
      label: t('tabmenu.closeBelow'),
      enabled: index >= 0 && index < tabs.length - 1,
      click: () => tabs.slice(index + 1).filter((other) => !other.pinned).forEach((other) => win.closeTab(other)),
    },
    { label: t('tabmenu.close'), click: () => win.closeTab(tab) },
  ]
  popupMenu(win, items, x, y)
}

export function showAppMenu(win: BrowserWin, x: number, y: number): void {
  const tab = win.activeTab()
  const focus = focusState()
  const items: MenuSpec[] = [
    { label: t('menu.newTab'), accelerator: 'Ctrl+T', click: () => newTab(win) },
    { label: t('menu.newWindow'), accelerator: 'Ctrl+N', click: () => createWindow({ incognito: false }) },
    { label: t('menu.newIncognito'), accelerator: 'Ctrl+Shift+N', click: () => createWindow({ incognito: true }) },
    sep,
    { label: t('menu.history'), accelerator: 'Ctrl+H', click: () => openPage(win, 'history') },
    ...(!win.incognito ? [{ label: t('menu.clearData'), accelerator: 'Ctrl+Shift+Delete', click: () => runCommand(win, 'clear-data') }] : []),
    ...(!win.incognito
      ? [
          {
            label: t('menu.bookmarks'),
            submenu: [
              { label: t('cmd.bookmark-page'), accelerator: 'Ctrl+D', enabled: isCommandAvailable(win, 'bookmark-page'), click: () => runCommand(win, 'bookmark-page') },
              { label: t('cmd.bookmark-tabs'), accelerator: 'Ctrl+Shift+D', enabled: isCommandAvailable(win, 'bookmark-tabs'), click: () => runCommand(win, 'bookmark-tabs') },
              sep,
              {
                label: t('bookmarks.showBar'),
                type: 'checkbox' as const,
                checked: settings().bookmarksBar !== 'never',
                accelerator: 'Ctrl+Shift+B',
                click: () => runCommand(win, 'toggle-bookmarks-bar'),
              },
              { label: t('bookmarks.manager'), accelerator: 'Ctrl+Shift+O', click: () => openPage(win, 'bookmarks') },
              sep,
              { label: t('import.menu'), click: () => runCommand(win, 'import-bookmarks') },
              { label: t('bookmarks.exportMenu'), click: () => void exportBookmarks(win) },
            ],
          },
        ]
      : []),
    ...(!win.incognito ? [{ label: t('menu.passwords'), click: () => openPage(win, 'passwords') }] : []),
    { label: t('menu.downloads'), accelerator: 'Ctrl+J', click: () => openPage(win, 'downloads') },
    ...(!win.incognito ? [{ label: t('menu.extensions'), click: () => openPage(win, 'extensions') }] : []),
    sep,
    { label: t('menu.zoomIn'), accelerator: 'Ctrl+=', enabled: !!tab?.webContents, click: () => tab?.zoom('in') },
    { label: t('menu.zoomOut'), accelerator: 'Ctrl+-', enabled: !!tab?.webContents, click: () => tab?.zoom('out') },
    { label: t('menu.zoomReset'), accelerator: 'Ctrl+0', enabled: !!tab?.webContents, click: () => tab?.zoom('reset') },
    sep,
    { label: t('menu.find'), accelerator: 'Ctrl+F', enabled: !!tab?.webContents && !tab.page, click: () => win.command({ type: 'find' }) },
    { label: t('menu.print'), accelerator: 'Ctrl+P', enabled: !!tab?.webContents && !tab.page, click: () => tab?.webContents?.print() },
    {
      label: t('menu.reader'),
      accelerator: 'Ctrl+Alt+R',
      enabled: !!tab && (tab.readerAvailable || !!tab.reader),
      click: () => void tab?.toggleReader(),
    },
    {
      label: t('menu.focus'),
      submenu: focus.active
        ? [{ label: t('focus.stop'), click: () => stopFocus() }]
        : [25, 50, 90].map((minutes) => ({ label: t('focus.minutes', { n: minutes }), click: () => startFocus(minutes) })),
    },
    { label: t('menu.fullscreen'), accelerator: 'F11', click: () => win.base.setFullScreen(!win.base.isFullScreen()) },
    { label: t('menu.devtools'), accelerator: 'F12', enabled: !!tab?.webContents && !tab.page, click: () => openDevTools(win) },
    sep,
    { label: t('menu.settings'), click: () => openPage(win, 'settings') },
    { label: t('menu.report'), click: () => runCommand(win, 'report') },
    { label: t('menu.about'), click: () => runCommand(win, 'check-updates') },
    sep,
    { label: t('menu.quit'), accelerator: 'Ctrl+Q', click: () => app.quit() },
  ]
  // x is the menu button's right edge: the menu lines up with it.
  popupMenu(win, items, x, y, { align: 'end' })
}

export function showDownloadMenu(win: BrowserWin, id: string, x: number, y: number): void {
  const entry = allDownloads().find((d) => d.id === id)
  if (!entry) return
  const active = entry.status === 'progressing' || entry.status === 'paused'
  const done = entry.status === 'completed' && !entry.missing
  const items: MenuSpec[] = [
    { label: t('downloads.open'), enabled: done, click: () => downloadAction(id, 'open') },
    { label: t('downloads.show'), enabled: done, click: () => downloadAction(id, 'show') },
    sep,
    ...(entry.status === 'progressing' ? [{ label: t('downloads.pause'), click: () => downloadAction(id, 'pause') }] : []),
    ...(entry.status === 'paused' ? [{ label: t('downloads.resume'), click: () => downloadAction(id, 'resume') }] : []),
    ...(active ? [{ label: t('downloads.cancel'), click: () => downloadAction(id, 'cancel') }] : []),
    ...(!active
      ? [{ label: entry.status === 'completed' ? t('downloads.again') : t('downloads.retry'), click: () => downloadAction(id, 'retry') }]
      : []),
    { label: t('downloads.copyLink'), click: () => downloadAction(id, 'copy') },
    sep,
    {
      label: t('downloads.trash'),
      enabled: done,
      click: () =>
        void confirm(win, {
          title: t('downloads.trashConfirm', { name: entry.filename }),
          body: t('downloads.trashBody'),
          confirm: t('downloads.trash'),
          cancel: t('common.cancel'),
          danger: true,
        }).then((ok) => ok && downloadAction(id, 'trash')),
    },
    { label: t('downloads.remove'), enabled: !active, click: () => downloadAction(id, 'remove') },
  ]
  popupMenu(win, items, x, y)
}

/** Lets every window's history page know the list changed under it. */
export function historyChanged(): void {
  for (const win of BrowserWin.all.values()) win.send('command', { type: 'history-changed' })
}

export function showHistoryMenu(win: BrowserWin, id: number, url: string, x: number, y: number): void {
  if (!/^https?:/.test(url)) return
  const host = hostOf(url).replace(/^www\./, '')
  const items: MenuSpec[] = [
    { label: t('bookmarks.openNewTab'), click: () => win.openTab({ url, background: true }) },
    { label: t('bookmarks.openNewWindow'), click: () => createWindow({ incognito: false, urls: [url] }) },
    { label: t('bookmarks.openIncognito'), click: () => createWindow({ incognito: true, urls: [url] }) },
    { label: t('ctx.copyLink'), click: () => clipboard.writeText(url) },
    sep,
    { label: t('history.moreFromSite'), click: () => win.send('command', { type: 'history-search', text: host }) },
    sep,
    {
      label: t('history.remove'),
      click: () => {
        removeVisits([id])
        historyChanged()
      },
    },
    {
      label: t('history.removeSite'),
      click: () =>
        void confirm(win, {
          title: t('history.removeSiteConfirm', { site: host }),
          body: t('history.removeSiteBody'),
          confirm: t('history.removeSiteRun'),
          cancel: t('common.cancel'),
          danger: true,
        }).then((ok) => {
          if (!ok) return
          deleteHost(host)
          historyChanged()
        }),
    },
  ]
  popupMenu(win, items, x, y)
}
