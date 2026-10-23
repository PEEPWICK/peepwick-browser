// Bookmarks: the tree model and the formats other browsers use. Pure functions only,
// so the main process, the UI and the tests share them.

export type BookmarkRootId = 'bar' | 'other'

export interface BookmarkLink {
  type: 'url'
  id: string
  title: string
  url: string
  addedAt: number
}

export interface BookmarkFolder {
  type: 'folder'
  id: string
  title: string
  addedAt: number
  children: BookmarkNode[]
}

export type BookmarkNode = BookmarkLink | BookmarkFolder

/** The two roots, as in Chromium: the bookmarks bar and "other bookmarks". */
export interface BookmarkTree {
  bar: BookmarkFolder
  other: BookmarkFolder
}

/** Imported data before ids are assigned. */
export type DraftNode =
  | { type: 'url'; title: string; url: string; addedAt?: number }
  | { type: 'folder'; title: string; addedAt?: number; children: DraftNode[] }

export interface DraftTree {
  bar: DraftNode[]
  other: DraftNode[]
}

const ALLOWED_URL = /^(https?|ftp|file):/i

export function isBookmarkableURL(url: string): boolean {
  return ALLOWED_URL.test(url.trim())
}

export function emptyTree(barTitle: string, otherTitle: string): BookmarkTree {
  return {
    bar: { type: 'folder', id: 'bar', title: barTitle, addedAt: 0, children: [] },
    other: { type: 'folder', id: 'other', title: otherTitle, addedAt: 0, children: [] },
  }
}

// ── Lookup ───────────────────────────────────────────────────────────────────

export interface Located {
  node: BookmarkNode
  parent: BookmarkFolder | null
  index: number
  /** Folders from the root down to the parent, for breadcrumbs. */
  path: BookmarkFolder[]
}

export function locate(tree: BookmarkTree, id: string): Located | null {
  for (const root of [tree.bar, tree.other]) {
    if (root.id === id) return { node: root, parent: null, index: -1, path: [] }
    const found = locateIn(root, id, [root])
    if (found) return found
  }
  return null
}

function locateIn(folder: BookmarkFolder, id: string, path: BookmarkFolder[]): Located | null {
  for (let i = 0; i < folder.children.length; i++) {
    const node = folder.children[i]
    if (node.id === id) return { node, parent: folder, index: i, path }
    if (node.type === 'folder') {
      const found = locateIn(node, id, [...path, node])
      if (found) return found
    }
  }
  return null
}

export function walk(tree: BookmarkTree, visit: (node: BookmarkNode, path: BookmarkFolder[]) => void): void {
  const go = (folder: BookmarkFolder, path: BookmarkFolder[]) => {
    for (const node of folder.children) {
      visit(node, path)
      if (node.type === 'folder') go(node, [...path, node])
    }
  }
  go(tree.bar, [tree.bar])
  go(tree.other, [tree.other])
}

export function links(tree: BookmarkTree): BookmarkLink[] {
  const out: BookmarkLink[] = []
  walk(tree, (node) => node.type === 'url' && out.push(node))
  return out
}

export function countNodes(nodes: (BookmarkNode | DraftNode)[]): { links: number; folders: number } {
  let linkCount = 0
  let folders = 0
  const go = (list: (BookmarkNode | DraftNode)[]) => {
    for (const node of list) {
      if (node.type === 'url') linkCount++
      else {
        folders++
        go(node.children)
      }
    }
  }
  go(nodes)
  return { links: linkCount, folders }
}

/** Folders as a flat list with depth, for folder pickers. */
export function folderList(tree: BookmarkTree): { folder: BookmarkFolder; depth: number }[] {
  const out: { folder: BookmarkFolder; depth: number }[] = []
  const go = (folder: BookmarkFolder, depth: number) => {
    out.push({ folder, depth })
    for (const node of folder.children) if (node.type === 'folder') go(node, depth + 1)
  }
  go(tree.bar, 0)
  go(tree.other, 0)
  return out
}

/** True when `id` is `ancestorId` or lies inside it. */
export function isInside(tree: BookmarkTree, id: string, ancestorId: string): boolean {
  if (id === ancestorId) return true
  const found = locate(tree, id)
  return !!found && found.path.some((folder) => folder.id === ancestorId)
}

/** Normalises a URL for duplicate checks: no trailing slash, no fragment, lower-case host. */
export function urlKey(url: string): string {
  try {
    const u = new URL(url)
    u.hash = ''
    let text = u.href
    if (text.endsWith('/') && u.pathname === '/' && !u.search) text = text.slice(0, -1)
    return text
  } catch {
    return url.trim()
  }
}

// ── Netscape bookmark file (the HTML every browser imports and exports) ───────

const ENTITIES: Record<string, string> = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: ' ' }

export function decodeEntities(text: string): string {
  return text.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (match, code: string) => {
    if (code[0] === '#') {
      const value = code[1] === 'x' || code[1] === 'X' ? parseInt(code.slice(2), 16) : parseInt(code.slice(1), 10)
      return Number.isFinite(value) && value > 0 && value < 0x110000 ? String.fromCodePoint(value) : match
    }
    return ENTITIES[code.toLowerCase()] ?? match
  })
}

function escapeHTML(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
}

function attribute(attrs: string, name: string): string | null {
  const match = new RegExp(`\\b${name}\\s*=\\s*("([^"]*)"|'([^']*)'|([^\\s>]+))`, 'i').exec(attrs)
  if (!match) return null
  return decodeEntities(match[2] ?? match[3] ?? match[4] ?? '')
}

function stripTags(text: string): string {
  return decodeEntities(text.replace(/<[^>]*>/g, '')).replace(/\s+/g, ' ').trim()
}

export interface ParsedFile {
  tree: DraftTree
  skipped: number
}

/**
 * Parses a Netscape bookmark file. The folder marked PERSONAL_TOOLBAR_FOLDER becomes
 * the bookmarks bar; everything else goes to "other bookmarks". Bookmarklets and
 * Firefox smart folders (place:) are skipped.
 */
export function parseNetscape(html: string): ParsedFile {
  const token = /<DL\b[^>]*>|<\/DL\s*>|<H3\b([^>]*)>([\s\S]*?)<\/H3\s*>|<A\b([^>]*)>([\s\S]*?)<\/A\s*>/gi
  const root: DraftNode[] = []
  const stack: DraftNode[][] = []
  let pending: (DraftNode & { type: 'folder' }) | null = null
  let toolbar: DraftNode[] | null = null
  let toolbarPending = false
  let skipped = 0
  let match: RegExpExecArray | null

  while ((match = token.exec(html))) {
    const text = match[0]
    if (/^<DL/i.test(text)) {
      if (pending) {
        stack.push(pending.children)
        if (toolbarPending) toolbar = pending.children
        pending = null
        toolbarPending = false
      } else {
        stack.push(stack.length === 0 ? root : (stack.at(-1) as DraftNode[]))
      }
    } else if (/^<\/DL/i.test(text)) {
      stack.pop()
    } else if (match[1] !== undefined) {
      const attrs = match[1]
      const folder: DraftNode & { type: 'folder' } = {
        type: 'folder',
        title: stripTags(match[2]) || 'Klasör',
        addedAt: secondsToMs(attribute(attrs, 'ADD_DATE')),
        children: [],
      }
      ;(stack.at(-1) ?? root).push(folder)
      pending = folder
      toolbarPending = /PERSONAL_TOOLBAR_FOLDER\s*=\s*"?true/i.test(attrs)
    } else if (match[3] !== undefined) {
      const url = attribute(match[3], 'HREF') ?? ''
      if (!isBookmarkableURL(url)) {
        skipped++
        continue
      }
      ;(stack.at(-1) ?? root).push({
        type: 'url',
        title: stripTags(match[4]) || url,
        url,
        addedAt: secondsToMs(attribute(match[3], 'ADD_DATE')),
      })
    }
  }

  if (toolbar) {
    const bar = toolbar
    const removeToolbar = (list: DraftNode[]): boolean => {
      const i = list.findIndex((n) => n.type === 'folder' && n.children === bar)
      if (i >= 0) {
        list.splice(i, 1)
        return true
      }
      return list.some((n) => n.type === 'folder' && removeToolbar(n.children))
    }
    removeToolbar(root)
    return { tree: { bar, other: root }, skipped }
  }
  return { tree: { bar: [], other: root }, skipped }
}

function secondsToMs(value: string | null): number | undefined {
  const n = value ? Number(value) : NaN
  return Number.isFinite(n) && n > 0 ? n * 1000 : undefined
}

/** Writes a Netscape bookmark file that Chrome, Firefox, Safari and Edge can import. */
export function toNetscape(tree: BookmarkTree): string {
  const lines = [
    '<!DOCTYPE NETSCAPE-Bookmark-file-1>',
    '<!-- This is an automatically generated file.',
    '     It will be read and overwritten.',
    '     DO NOT EDIT! -->',
    '<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">',
    '<TITLE>Bookmarks</TITLE>',
    '<H1>Bookmarks</H1>',
    '<DL><p>',
  ]
  const date = (ms: number) => Math.floor((ms || Date.now()) / 1000)
  const write = (nodes: BookmarkNode[], depth: number) => {
    const pad = '    '.repeat(depth)
    for (const node of nodes) {
      if (node.type === 'url') {
        lines.push(`${pad}<DT><A HREF="${escapeHTML(node.url)}" ADD_DATE="${date(node.addedAt)}">${escapeHTML(node.title)}</A>`)
      } else {
        lines.push(`${pad}<DT><H3 ADD_DATE="${date(node.addedAt)}">${escapeHTML(node.title)}</H3>`)
        lines.push(`${pad}<DL><p>`)
        write(node.children, depth + 1)
        lines.push(`${pad}</DL><p>`)
      }
    }
  }
  lines.push(`    <DT><H3 ADD_DATE="${date(tree.bar.addedAt)}" PERSONAL_TOOLBAR_FOLDER="true">${escapeHTML(tree.bar.title)}</H3>`)
  lines.push('    <DL><p>')
  write(tree.bar.children, 2)
  lines.push('    </DL><p>')
  write(tree.other.children, 1)
  lines.push('</DL><p>')
  return `${lines.join('\n')}\n`
}

// ── Chromium "Bookmarks" JSON ─────────────────────────────────────────────────

interface ChromiumNode {
  type?: string
  name?: string
  url?: string
  date_added?: string
  children?: ChromiumNode[]
}

/** Chromium stores times as microseconds since 1601-01-01. */
function webkitToMs(value: string | undefined): number | undefined {
  const n = value ? Number(value) : NaN
  if (!Number.isFinite(n) || n <= 0) return undefined
  return Math.round(n / 1000 - 11_644_473_600_000)
}

function fromChromiumNodes(nodes: ChromiumNode[] | undefined, skipped: { count: number }): DraftNode[] {
  const out: DraftNode[] = []
  for (const node of nodes ?? []) {
    if (node.type === 'folder') {
      out.push({
        type: 'folder',
        title: node.name || 'Klasör',
        addedAt: webkitToMs(node.date_added),
        children: fromChromiumNodes(node.children, skipped),
      })
    } else if (node.type === 'url' && node.url) {
      if (!isBookmarkableURL(node.url)) {
        skipped.count++
        continue
      }
      out.push({ type: 'url', title: node.name || node.url, url: node.url, addedAt: webkitToMs(node.date_added) })
    }
  }
  return out
}

export function fromChromium(json: unknown, mobileTitle: string): ParsedFile {
  const roots = (json as { roots?: Record<string, ChromiumNode> })?.roots
  if (!roots || typeof roots !== 'object') throw new Error('Not a Chromium bookmarks file')
  const skipped = { count: 0 }
  const other = fromChromiumNodes(roots.other?.children, skipped)
  const mobile = fromChromiumNodes(roots.synced?.children, skipped)
  if (mobile.length > 0) other.push({ type: 'folder', title: mobileTitle, children: mobile })
  return { tree: { bar: fromChromiumNodes(roots.bookmark_bar?.children, skipped), other }, skipped: skipped.count }
}

// ── Firefox places ───────────────────────────────────────────────────────────

export interface PlacesRow {
  id: number
  type: number
  parent: number
  position: number
  title: string | null
  url: string | null
  dateAdded: number | null
  guid: string
}

/** Builds a draft tree from moz_bookmarks rows (Firefox, LibreWolf, Zen…). */
export function fromPlaces(rows: PlacesRow[], menuTitle: string, mobileTitle: string): ParsedFile {
  const byParent = new Map<number, PlacesRow[]>()
  for (const row of rows) {
    const list = byParent.get(row.parent) ?? []
    list.push(row)
    byParent.set(row.parent, list)
  }
  for (const list of byParent.values()) list.sort((a, b) => a.position - b.position)
  const skipped = { count: 0 }
  const build = (parent: number): DraftNode[] => {
    const out: DraftNode[] = []
    for (const row of byParent.get(parent) ?? []) {
      // Firefox stores dateAdded in microseconds since the Unix epoch.
      const addedAt = row.dateAdded ? Math.round(row.dateAdded / 1000) : undefined
      if (row.type === 2) {
        out.push({ type: 'folder', title: row.title || 'Klasör', addedAt, children: build(row.id) })
      } else if (row.type === 1 && row.url) {
        if (!isBookmarkableURL(row.url)) {
          skipped.count++
          continue
        }
        out.push({ type: 'url', title: row.title || row.url, url: row.url, addedAt })
      }
    }
    return out
  }
  const root = (guid: string) => rows.find((r) => r.guid === guid)
  const toolbar = root('toolbar_____')
  const menu = root('menu________')
  const unfiled = root('unfiled_____')
  const mobile = root('mobile______')
  const other: DraftNode[] = []
  const menuItems = menu ? build(menu.id) : []
  if (menuItems.length > 0) other.push({ type: 'folder', title: menuTitle, children: menuItems })
  if (unfiled) other.push(...build(unfiled.id))
  const mobileItems = mobile ? build(mobile.id) : []
  if (mobileItems.length > 0) other.push({ type: 'folder', title: mobileTitle, children: mobileItems })
  return { tree: { bar: toolbar ? build(toolbar.id) : [], other }, skipped: skipped.count }
}

/** Drops links whose address is already bookmarked (or repeated in the import). */
export function withoutDuplicates(tree: DraftTree, existing: Set<string>): { tree: DraftTree; duplicates: number } {
  const seen = new Set(existing)
  let duplicates = 0
  const filter = (nodes: DraftNode[]): DraftNode[] => {
    const out: DraftNode[] = []
    for (const node of nodes) {
      if (node.type === 'url') {
        const key = urlKey(node.url)
        if (seen.has(key)) {
          duplicates++
          continue
        }
        seen.add(key)
        out.push(node)
      } else {
        const children = filter(node.children)
        if (children.length > 0) out.push({ ...node, children })
      }
    }
    return out
  }
  return { tree: { bar: filter(tree.bar), other: filter(tree.other) }, duplicates }
}
