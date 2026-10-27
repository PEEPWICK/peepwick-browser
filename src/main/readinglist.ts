// The reading list: pages saved to read later. When a page has an article, its text
// is kept on disk too, so it can be read offline at peepwick://saved/<id> — drawn
// like reader view, with scripts, forms and frames switched off.
import { WebContentsView, nativeTheme, type Session, type WebContents } from 'electron'
import { randomUUID } from 'node:crypto'
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { readFile } from 'node:fs/promises'
import type { ReadingItem } from '@shared/types'
import { displayHost } from '@shared/url'
import { userPath, resourcePath } from './paths'
import { extractArticle, isReaderable, renderArticle, type Article } from './reader'
import { settings } from './settings'
import { JsonFile } from './store'
import { BrowserWin } from './window'

const file = new JsonFile<{ items: ReadingItem[] }>(userPath('readinglist.json'), 300)
let items: ReadingItem[] = []

const articlePath = (id: string) => userPath('reading', `${id}.json`)

export function loadReadingList(): void {
  const data = file.read()
  if (data && Array.isArray(data.items)) items = data.items.filter((i) => i && typeof i.id === 'string' && typeof i.url === 'string')
}

export function flushReadingList(): void {
  file.flush()
}

function changed(): void {
  file.schedule({ items })
  for (const win of BrowserWin.all.values()) win.send('command', { type: 'reading-changed' })
}

export function readingList(): ReadingItem[] {
  return items
}

export function inReadingList(url: string): boolean {
  return items.some((i) => i.url === url)
}

function store(url: string, title: string, article: Article | null): ReadingItem {
  const existing = items.find((i) => i.url === url)
  const id = existing?.id ?? randomUUID()
  if (article) {
    mkdirSync(userPath('reading'), { recursive: true })
    writeFileSync(articlePath(id), JSON.stringify(article))
  }
  const item: ReadingItem = {
    id,
    url,
    title: article?.title || title || displayHost(url),
    site: article?.siteName || displayHost(url),
    words: article?.length ?? 0,
    saved: !!article || !!existing?.saved,
    addedAt: Date.now(),
    readAt: null,
  }
  items = [item, ...items.filter((i) => i.id !== id)]
  changed()
  return item
}

/** Saves the page a tab shows, with its article when it has one. */
export async function addPage(contents: WebContents): Promise<ReadingItem | null> {
  const url = contents.getURL()
  if (!/^https?:/.test(url)) return null
  // Sign-in forms and app pages have no article worth keeping, only their address.
  const article = (await isReaderable(contents)) ? await extractArticle(contents) : null
  return store(url, contents.getTitle(), article)
}

/** Saves a link: the page is loaded out of sight to keep its article. */
export async function addLink(url: string, title: string, session: Session): Promise<ReadingItem | null> {
  if (!/^https?:/.test(url)) return null
  const item = store(url, title, null)
  const view = new WebContentsView({ webPreferences: { session, sandbox: true, contextIsolation: true, nodeIntegration: false, spellcheck: false } })
  const contents = view.webContents
  contents.setAudioMuted(true)
  contents.setWindowOpenHandler(() => ({ action: 'deny' }))
  try {
    await Promise.race([contents.loadURL(url), new Promise((_, reject) => setTimeout(() => reject(new Error('timeout')), 20_000))])
    // Many pages fill in their text a moment after loading.
    await new Promise((resolve) => setTimeout(resolve, 1200))
    const article = (await isReaderable(contents)) ? await extractArticle(contents) : null
    if (article && items.some((i) => i.id === item.id)) return store(url, contents.getTitle() || title, article)
  } catch {
    // Kept as a plain link.
  } finally {
    contents.close()
  }
  return item
}

export function removeReading(id: string): void {
  items = items.filter((i) => i.id !== id)
  rmSync(articlePath(id), { force: true })
  changed()
}

export function markRead(id: string, read: boolean): void {
  const item = items.find((i) => i.id === id)
  if (!item || (item.readAt !== null) === read) return
  item.readAt = read ? Date.now() : null
  changed()
}

/** Where to open an item: its offline copy when there is one. */
export function readingURL(id: string): string | null {
  const item = items.find((i) => i.id === id)
  if (!item) return null
  return item.saved ? `peepwick://saved/${id}` : item.url
}

/** The original address of an offline copy, for the address bar and bookmarks. */
export function savedSource(url: string): string | null {
  const match = /^peepwick:\/\/saved\/([\w-]+)/.exec(url)
  return match ? (items.find((i) => i.id === match[1])?.url ?? null) : null
}

export function savedArticle(id: string): Article | null {
  try {
    return JSON.parse(readFileSync(articlePath(id), 'utf8')) as Article
  } catch {
    return null
  }
}

const SAVED_CSP =
  "default-src 'none'; img-src https: http: data:; media-src https: http:; style-src 'unsafe-inline'; font-src 'self'; form-action 'none'; base-uri 'none'; frame-ancestors 'none'"

/** peepwick://saved/<id> in browsing sessions: the offline copy, and the reader fonts it uses. */
export async function savedResponse(url: URL): Promise<Response> {
  const font = /^\/fonts\/([\w.-]+\.woff2)$/.exec(url.pathname)
  if (font) {
    try {
      return new Response(await readFile(resourcePath('fonts', font[1])), { headers: { 'content-type': 'font/woff2', 'cache-control': 'max-age=31536000' } })
    } catch {
      return new Response('Not found', { status: 404 })
    }
  }
  const id = url.pathname.slice(1)
  const item = items.find((i) => i.id === id)
  const article = item ? savedArticle(id) : null
  if (!item || !article) return new Response('Not found', { status: 404 })
  const theme = settings().theme
  const dark = theme === 'dark' || (theme === 'system' && nativeTheme.shouldUseDarkColors)
  const html = renderArticle(article, item.url, settings().language, dark)
  return new Response(html, { headers: { 'content-type': 'text/html; charset=utf-8', 'content-security-policy': SAVED_CSP, 'cache-control': 'no-store' } })
}
