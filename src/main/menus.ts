// Native menus: the page context menu, the tab menu in the sidebar and the app menu.
import { app, clipboard, Menu, MenuItem, type ContextMenuParams, type MenuItemConstructorOptions, type WebContents } from 'electron'
import { searchURL } from '@shared/search'
import { focusState, startFocus, stopFocus } from './focus'
import { getExtensions } from './extensions'
import { isCommandAvailable, newTab, openDevTools, openPage, runCommand, savePage } from './actions'
import { createBookmark, exportBookmarks } from './bookmarks'
import { settings, t } from './settings'
import type { Tab } from './tab'
import type { BrowserWin } from './window'
import { createWindow } from './windows'

const sep: MenuItemConstructorOptions = { type: 'separator' }

function clean(items: MenuItemConstructorOptions[]): MenuItemConstructorOptions[] {
  const out: MenuItemConstructorOptions[] = []
  for (const item of items) {
    if (item.type === 'separator' && (out.length === 0 || out.at(-1)?.type === 'separator')) continue
    out.push(item)
  }
  while (out.at(-1)?.type === 'separator') out.pop()
  return out
}

export function showPageMenu(tab: Tab, wc: WebContents, params: ContextMenuParams): void {
  const win = tab.win
  const items: MenuItemConstructorOptions[] = []

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
      { label: t('ctx.cut'), role: 'cut', enabled: flags.canCut },
      { label: t('ctx.copy'), role: 'copy', enabled: flags.canCopy },
      { label: t('ctx.paste'), role: 'paste', enabled: flags.canPaste },
      { label: t('ctx.selectAll'), role: 'selectAll', enabled: flags.canSelectAll },
      sep,
    )
  } else if (params.selectionText.trim()) {
    const text = params.selectionText.trim()
    const short = text.length > 24 ? `${text.slice(0, 24)}…` : text
    items.push(
      { label: t('ctx.copy'), role: 'copy' },
      {
        label: t('ctx.searchFor', { text: short }),
        click: () => win.openTab({ url: searchURL(settings().searchEngine, text), opener: tab }),
      },
      sep,
    )
  }

  if (!link && !params.isEditable && !params.selectionText.trim() && params.mediaType === 'none') {
    items.push(
      { label: t('ctx.back'), enabled: tab.canGoBack(), click: () => tab.back() },
      { label: t('ctx.forward'), enabled: tab.canGoForward(), click: () => tab.forward() },
      { label: t('ctx.reload'), click: () => tab.reload() },
      sep,
      { label: t('ctx.savePage'), click: () => void savePage(win) },
      { label: t('ctx.print'), click: () => wc.print() },
      sep,
    )
  }

  const menu = Menu.buildFromTemplate(clean(items))
  const extensionItems = win.incognito ? [] : (getExtensions()?.getContextMenuItems(wc, params) ?? [])
  if (extensionItems.length > 0 && menu.items.length > 0) menu.append(new MenuItem(sep))
  for (const item of extensionItems) menu.append(item)
  if (menu.items.length > 0) menu.append(new MenuItem(sep))
  menu.append(
    new MenuItem({
      label: t('ctx.inspect'),
      click: () => {
        if (!wc.isDevToolsOpened()) wc.openDevTools({ mode: 'detach' })
        wc.inspectElement(params.x, params.y)
      },
    }),
  )
  menu.popup({ window: win.base })
}

export function showTabMenu(win: BrowserWin, tab: Tab, x: number, y: number): void {
  const workspace = win.workspaceOf(tab)
  const active = win.activeTab()
  const tabs = workspace ? win.tabsInOrder(workspace) : []
  const index = tabs.indexOf(tab)
  const inSplit = !!workspace?.split && (workspace.split.left === tab.id || workspace.split.right === tab.id)
  const others = win.workspaces.filter((w) => w !== workspace)

  const items: MenuItemConstructorOptions[] = [
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
  Menu.buildFromTemplate(clean(items)).popup({ window: win.base, x: Math.round(x), y: Math.round(y) })
}

export function showAppMenu(win: BrowserWin, x: number, y: number): void {
  const tab = win.activeTab()
  const focus = focusState()
  const items: MenuItemConstructorOptions[] = [
    { label: t('menu.newTab'), accelerator: 'Ctrl+T', click: () => newTab(win) },
    { label: t('menu.newWindow'), accelerator: 'Ctrl+N', click: () => createWindow({ incognito: false }) },
    { label: t('menu.newIncognito'), accelerator: 'Ctrl+Shift+N', click: () => createWindow({ incognito: true }) },
    sep,
    { label: t('menu.history'), accelerator: 'Ctrl+H', click: () => openPage(win, 'history') },
    ...(!win.incognito
      ? [
          {
            label: t('menu.bookmarks'),
            submenu: [
              { label: t('cmd.bookmark-page'), accelerator: 'Ctrl+D', enabled: isCommandAvailable(win, 'bookmark-page'), click: () => runCommand(win, 'bookmark-page') },
              { label: t('cmd.bookmark-tabs'), accelerator: 'Ctrl+Shift+D', enabled: isCommandAvailable(win, 'bookmark-tabs'), click: () => runCommand(win, 'bookmark-tabs') },
              { type: 'separator' as const },
              {
                label: t('bookmarks.showBar'),
                type: 'checkbox' as const,
                checked: settings().bookmarksBar !== 'never',
                accelerator: 'Ctrl+Shift+B',
                click: () => runCommand(win, 'toggle-bookmarks-bar'),
              },
              { label: t('bookmarks.manager'), accelerator: 'Ctrl+Shift+O', click: () => openPage(win, 'bookmarks') },
              { type: 'separator' as const },
              { label: t('import.menu'), click: () => runCommand(win, 'import-bookmarks') },
              { label: t('bookmarks.exportMenu'), click: () => void exportBookmarks(win) },
            ],
          },
        ]
      : []),
    { label: t('menu.downloads'), accelerator: 'Ctrl+J', click: () => win.command({ type: 'downloads' }) },
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
    { label: t('menu.about'), click: () => runCommand(win, 'check-updates') },
    sep,
    { label: t('menu.quit'), accelerator: 'Ctrl+Q', click: () => app.quit() },
  ]
  Menu.buildFromTemplate(clean(items)).popup({ window: win.base, x: Math.round(x), y: Math.round(y) })
}
