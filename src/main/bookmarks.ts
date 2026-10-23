// Bookmarks: a JSON tree in the profile (like Chromium's Bookmarks file), edited only
// here and pushed to every window's UI when it changes.
import { app, clipboard, dialog, Menu, nativeImage, type MenuItemConstructorOptions, type NativeImage } from 'electron'
import { randomUUID } from 'node:crypto'
import { writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import {
  emptyTree,
  isBookmarkableURL,
  isInside,
  locate,
  toNetscape,
  urlKey,
  walk,
  type BookmarkFolder,
  type BookmarkLink,
  type BookmarkNode,
  type BookmarkTree,
  type DraftNode,
  type DraftTree,
} from '@shared/bookmarks'
import type { BookmarkInput, BookmarkOpen } from '@shared/ipc'
import { hostOf } from '@shared/url'
import { bus } from './events'
import { getFavicon } from './history'
import { userPath } from './paths'
import { openPage } from './actions'
import { settings, t } from './settings'
import { JsonFile } from './store'
import type { BrowserWin } from './window'
import { createWindow } from './windows'

const file = new JsonFile<BookmarkTree>(userPath('bookmarks.json'), 400)
let tree: BookmarkTree = emptyTree('', '')
/** The last deletion, so it can be undone from the toast. */
let lastRemoved: { node: BookmarkNode; parentId: string; index: number }[] = []
/** Folder the last bookmark went into; Ctrl+D uses it again. */
let lastFolderId = 'bar'

const OPEN_ALL_CONFIRM = 15

function isNode(value: unknown): value is BookmarkNode {
  const node = value as BookmarkNode
  if (!node || typeof node.id !== 'string' || typeof node.title !== 'string') return false
  if (node.type === 'url') return typeof node.url === 'string'
  return node.type === 'folder' && Array.isArray(node.children) && node.children.every(isNode)
}

export function loadBookmarks(): void {
  const saved = file.read()
  if (saved && isNode(saved.bar) && isNode(saved.other) && saved.bar.type === 'folder' && saved.other.type === 'folder') {
    tree = saved
  }
}

export function flushBookmarks(): void {
  file.flush()
}

export function bookmarkTree(): BookmarkTree {
  return tree
}

function changed(): void {
  file.schedule(tree)
  bus.emit('bookmarks')
}

function folderById(id: string): BookmarkFolder | null {
  const found = locate(tree, id)
  return found && found.node.type === 'folder' ? found.node : null
}

function clampIndex(folder: BookmarkFolder, index: number | undefined): number {
  return index === undefined || !Number.isFinite(index) ? folder.children.length : Math.max(0, Math.min(index, folder.children.length))
}

// ── Editing ───────────────────────────────────────────────────────────────────

export function createBookmark(input: BookmarkInput): string | null {
  const parent = folderById(input.parentId)
  if (!parent) return null
  const title = input.title.trim().slice(0, 500)
  const now = Date.now()
  let node: BookmarkNode
  if (input.url !== undefined) {
    const url = input.url.trim()
    if (!isBookmarkableURL(url) || url.length > 8192) return null
    node = { type: 'url', id: randomUUID(), title: title || url, url, addedAt: now }
  } else {
    node = { type: 'folder', id: randomUUID(), title: title || t('bookmarks.newFolder'), addedAt: now, children: [] }
  }
  parent.children.splice(clampIndex(parent, input.index), 0, node)
  if (node.type === 'url') lastFolderId = parent.id
  changed()
  return node.id
}

export function updateBookmark(id: string, patch: { title?: string; url?: string }): void {
  const found = locate(tree, id)
  if (!found || !found.parent) return
  const node = found.node
  if (typeof patch.title === 'string') node.title = patch.title.trim().slice(0, 500) || node.title
  if (node.type === 'url' && typeof patch.url === 'string' && isBookmarkableURL(patch.url)) node.url = patch.url.trim()
  changed()
}

export function moveBookmarks(ids: string[], parentId: string, index: number): void {
  const target = folderById(parentId)
  if (!target) return
  let position = clampIndex(target, index)
  for (const id of ids) {
    const found = locate(tree, id)
    if (!found || !found.parent) continue
    // A folder can't move into itself or one of its own subfolders.
    if (found.node.type === 'folder' && isInside(tree, target.id, found.node.id)) continue
    found.parent.children.splice(found.index, 1)
    if (found.parent === target && found.index < position) position--
    target.children.splice(position, 0, found.node)
    position++
  }
  changed()
}

export function removeBookmarks(ids: string[]): number {
  const removed: typeof lastRemoved = []
  for (const id of ids) {
    const found = locate(tree, id)
    if (!found || !found.parent) continue
    found.parent.children.splice(found.index, 1)
    removed.push({ node: found.node, parentId: found.parent.id, index: found.index })
  }
  if (removed.length > 0) {
    lastRemoved = removed
    changed()
  }
  return removed.length
}

export function undoRemove(): void {
  for (const entry of [...lastRemoved].reverse()) {
    const parent = folderById(entry.parentId) ?? tree.other
    parent.children.splice(clampIndex(parent, entry.index), 0, entry.node)
  }
  if (lastRemoved.length > 0) changed()
  lastRemoved = []
}

/** Folders first, then links, each in alphabetical order for the user's language. */
export function sortFolder(id: string): void {
  const folder = folderById(id)
  if (!folder) return
  const collator = new Intl.Collator(settings().language, { sensitivity: 'base', numeric: true })
  folder.children.sort((a, b) => {
    if (a.type !== b.type) return a.type === 'folder' ? -1 : 1
    return collator.compare(a.title, b.title)
  })
  changed()
}

export function findByURL(url: string): BookmarkLink[] {
  const key = urlKey(url)
  const out: BookmarkLink[] = []
  walk(tree, (node) => node.type === 'url' && urlKey(node.url) === key && out.push(node))
  return out
}

export function bookmarkedKeys(): Set<string> {
  const keys = new Set<string>()
  walk(tree, (node) => node.type === 'url' && keys.add(urlKey(node.url)))
  return keys
}

/** Ctrl+D: bookmarks the page, or returns the existing bookmark. */
export function bookmarkPage(url: string, title: string): { id: string; created: boolean } | null {
  if (!isBookmarkableURL(url)) return null
  const existing = findByURL(url)[0]
  if (existing) return { id: existing.id, created: false }
  const parentId = folderById(lastFolderId) ? lastFolderId : 'bar'
  const id = createBookmark({ parentId, title, url })
  return id ? { id, created: true } : null
}

/** Ctrl+Shift+D: a new folder in the bar holding every open tab of the space. */
export function bookmarkTabs(tabs: { url: string; title: string }[], folderTitle: string): string | null {
  const usable = tabs.filter((tab) => isBookmarkableURL(tab.url))
  if (usable.length === 0) return null
  const folderId = createBookmark({ parentId: 'bar', title: folderTitle })
  if (!folderId) return null
  const folder = folderById(folderId) as BookmarkFolder
  const now = Date.now()
  for (const tab of usable) folder.children.push({ type: 'url', id: randomUUID(), title: tab.title || tab.url, url: tab.url, addedAt: now })
  changed()
  return folderId
}

/** Adds imported bookmarks, following Chromium: straight into empty roots, else into one folder in the bar. */
export function insertDraft(draft: DraftTree, folderTitle: string): void {
  const materialize = (nodes: DraftNode[]): BookmarkNode[] =>
    nodes.map((node) =>
      node.type === 'url'
        ? { type: 'url', id: randomUUID(), title: node.title, url: node.url, addedAt: node.addedAt ?? Date.now() }
        : { type: 'folder', id: randomUUID(), title: node.title, addedAt: node.addedAt ?? Date.now(), children: materialize(node.children) },
    )
  if (tree.bar.children.length === 0 && tree.other.children.length === 0) {
    tree.bar.children.push(...materialize(draft.bar))
    tree.other.children.push(...materialize(draft.other))
  } else {
    const children = materialize(draft.bar)
    if (draft.other.length > 0) {
      const other = materialize(draft.other)
      if (children.length > 0) children.push({ type: 'folder', id: randomUUID(), title: t('bookmarks.other'), addedAt: Date.now(), children: other })
      else children.push(...other)
    }
    tree.bar.children.push({ type: 'folder', id: randomUUID(), title: folderTitle, addedAt: Date.now(), children })
  }
  changed()
}

// ── Opening ───────────────────────────────────────────────────────────────────

function linksUnder(ids: string[]): BookmarkLink[] {
  const out: BookmarkLink[] = []
  const collect = (node: BookmarkNode) => {
    if (node.type === 'url') out.push(node)
    else node.children.forEach(collect)
  }
  for (const id of ids) {
    const found = locate(tree, id)
    if (found) collect(found.node)
  }
  return out
}

export async function openBookmarks(win: BrowserWin, ids: string[], how: BookmarkOpen): Promise<void> {
  const targets = linksUnder(ids)
  if (targets.length === 0) return
  if (targets.length > OPEN_ALL_CONFIRM) {
    const { response } = await dialog.showMessageBox(win.base, {
      type: 'question',
      buttons: [t('bookmarks.openAllConfirm', { n: targets.length }), t('common.cancel')],
      defaultId: 0,
      cancelId: 1,
      message: t('bookmarks.openAllTitle', { n: targets.length }),
    })
    if (response !== 0) return
  }
  const urls = targets.map((link) => link.url)
  if (how === 'window' || how === 'incognito') {
    createWindow({ incognito: how === 'incognito', urls })
    return
  }
  if (how === 'current' && urls.length === 1) {
    const tab = win.activeTab()
    if (tab) {
      tab.load(urls[0])
      win.focusActive()
      return
    }
  }
  urls.forEach((url, i) => win.openTab({ url, background: how === 'background' || i > 0 }))
}

// ── Native menus ──────────────────────────────────────────────────────────────

const iconCache = new Map<string, NativeImage | null>()

function iconFor(url: string): NativeImage | undefined {
  const host = hostOf(url)
  if (!iconCache.has(host)) {
    const data = getFavicon(host)
    iconCache.set(host, data ? nativeImage.createFromDataURL(data).resize({ width: 16, height: 16 }) : null)
    if (iconCache.size > 500) iconCache.delete(iconCache.keys().next().value as string)
  }
  return iconCache.get(host) ?? undefined
}

function label(text: string): string {
  // Menus treat "&" as a mnemonic marker.
  const clean = text.replace(/&/g, '&&')
  return clean.length > 60 ? `${clean.slice(0, 57)}…` : clean
}

function folderItems(win: BrowserWin, folder: BookmarkFolder, fromIndex = 0): MenuItemConstructorOptions[] {
  const items: MenuItemConstructorOptions[] = folder.children.slice(fromIndex).map((node) =>
    node.type === 'url'
      ? {
          label: label(node.title),
          icon: iconFor(node.url),
          click: (_item, _window, event) =>
            void openBookmarks(win, [node.id], event.ctrlKey || event.metaKey ? 'background' : event.shiftKey ? 'window' : 'current'),
        }
      : { label: label(node.title), submenu: folderItems(win, node) },
  )
  if (items.length === 0) items.push({ label: t('bookmarks.empty'), enabled: false })
  const linkCount = folder.children.slice(fromIndex).filter((node) => node.type === 'url').length
  if (linkCount > 1) {
    items.push(
      { type: 'separator' },
      { label: t('bookmarks.openAll', { n: linkCount }), click: () => void openBookmarks(win, folder.children.slice(fromIndex).filter((n) => n.type === 'url').map((n) => n.id), 'tab') },
    )
  }
  return items
}

export function showFolderMenu(win: BrowserWin, folderId: string, x: number, y: number, fromIndex = 0): void {
  const folder = folderById(folderId)
  if (!folder) return
  Menu.buildFromTemplate(folderItems(win, folder, fromIndex)).popup({ window: win.base, x: Math.round(x), y: Math.round(y) })
}

export function showBookmarkMenu(win: BrowserWin, id: string, x: number, y: number): void {
  const found = locate(tree, id)
  if (!found) return
  const node = found.node
  const parentId = node.type === 'folder' ? node.id : (found.parent?.id ?? 'bar')
  const isRoot = !found.parent
  const add: MenuItemConstructorOptions[] = [
    { label: t('bookmarks.addPage'), click: () => win.command({ type: 'bookmark-edit', parentId }) },
    { label: t('bookmarks.addFolder'), click: () => win.command({ type: 'bookmark-edit', parentId, folder: true }) },
  ]
  let items: MenuItemConstructorOptions[]
  if (node.type === 'url') {
    items = [
      { label: t('bookmarks.open'), click: () => void openBookmarks(win, [id], 'current') },
      { label: t('bookmarks.openNewTab'), click: () => void openBookmarks(win, [id], 'background') },
      { label: t('bookmarks.openNewWindow'), click: () => void openBookmarks(win, [id], 'window') },
      { label: t('bookmarks.openIncognito'), click: () => void openBookmarks(win, [id], 'incognito') },
      { type: 'separator' },
      { label: t('bookmarks.edit'), click: () => win.command({ type: 'bookmark-edit', id }) },
      { label: t('ctx.copyLink'), click: () => clipboard.writeText(node.url) },
      { label: t('bookmarks.delete'), click: () => removeAndTell(win, [id]) },
      { type: 'separator' },
      ...add,
    ]
  } else {
    const count = linksUnder([id]).length
    items = [
      { label: t('bookmarks.openAll', { n: count }), enabled: count > 0, click: () => void openBookmarks(win, [id], 'tab') },
      { label: t('bookmarks.openAllWindow'), enabled: count > 0, click: () => void openBookmarks(win, [id], 'window') },
      { label: t('bookmarks.openAllIncognito'), enabled: count > 0, click: () => void openBookmarks(win, [id], 'incognito') },
      { type: 'separator' },
      ...(isRoot ? [] : [{ label: t('bookmarks.rename'), click: () => win.command({ type: 'bookmark-edit', id }) }]),
      { label: t('bookmarks.sort'), enabled: node.children.length > 1, click: () => sortFolder(id) },
      ...(isRoot ? [] : [{ label: t('bookmarks.delete'), click: () => removeAndTell(win, [id]) }]),
      { type: 'separator' },
      ...add,
    ]
  }
  items.push({ type: 'separator' }, { label: t('bookmarks.manager'), click: () => openPage(win, 'bookmarks') })
  Menu.buildFromTemplate(items).popup({ window: win.base, x: Math.round(x), y: Math.round(y) })
}

export function removeAndTell(win: BrowserWin, ids: string[]): void {
  const count = removeBookmarks(ids)
  if (count > 0) win.send('command', { type: 'bookmarks-removed', count })
}

// ── Export ────────────────────────────────────────────────────────────────────

export async function exportBookmarks(win: BrowserWin): Promise<boolean> {
  const day = new Date().toISOString().slice(0, 10)
  const { canceled, filePath } = await dialog.showSaveDialog(win.base, {
    title: t('bookmarks.export'),
    defaultPath: join(app.getPath('documents'), `peepwick-bookmarks-${day}.html`),
    filters: [{ name: 'HTML', extensions: ['html', 'htm'] }],
  })
  if (canceled || !filePath) return false
  const titled: BookmarkTree = {
    bar: { ...tree.bar, title: t('bookmarks.bar') },
    other: { ...tree.other, title: t('bookmarks.other') },
  }
  await writeFile(filePath, toNetscape(titled), 'utf8')
  return true
}
