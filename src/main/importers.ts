// Importing bookmarks from other browsers on this machine (Chromium family and
// Firefox family, including Snap and Flatpak installs) or from an HTML file.
// Browsers keep their databases locked while running, so SQLite files are copied
// to a private temporary folder and read from there.
import { dialog } from 'electron'
import { copyFileSync, existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync } from 'node:fs'
import { homedir, tmpdir } from 'node:os'
import { join } from 'node:path'
import { DatabaseSync } from 'node:sqlite'
import {
  countNodes,
  fromChromium,
  fromPlaces,
  parseNetscape,
  withoutDuplicates,
  type DraftNode,
  type DraftTree,
  type ParsedFile,
  type PlacesRow,
} from '@shared/bookmarks'
import type { ImportResult, ImportSource } from '@shared/types'
import { hostOf } from '@shared/url'
import { bookmarkedKeys, insertDraft } from './bookmarks'
import { getFavicon, setFavicon } from './history'
import { t } from './settings'
import type { BrowserWin } from './window'

interface Found extends ImportSource {
  dir: string
}

const home = homedir()

const CHROMIUM: { browser: string; roots: string[] }[] = [
  { browser: 'Google Chrome', roots: ['.config/google-chrome', '.var/app/com.google.Chrome/config/google-chrome'] },
  { browser: 'Google Chrome Beta', roots: ['.config/google-chrome-beta'] },
  { browser: 'Google Chrome Dev', roots: ['.config/google-chrome-unstable'] },
  {
    browser: 'Chromium',
    roots: ['.config/chromium', 'snap/chromium/common/chromium', '.var/app/org.chromium.Chromium/config/chromium'],
  },
  {
    browser: 'Brave',
    roots: ['.config/BraveSoftware/Brave-Browser', '.var/app/com.brave.Browser/config/BraveSoftware/Brave-Browser', 'snap/brave/current/.config/BraveSoftware/Brave-Browser'],
  },
  { browser: 'Microsoft Edge', roots: ['.config/microsoft-edge', '.var/app/com.microsoft.Edge/config/microsoft-edge'] },
  { browser: 'Vivaldi', roots: ['.config/vivaldi', '.var/app/com.vivaldi.Vivaldi/config/vivaldi'] },
  { browser: 'Opera', roots: ['.config/opera', '.var/app/com.opera.Opera/config/opera', 'snap/opera/current/.config/opera'] },
]

const FIREFOX: { browser: string; roots: string[] }[] = [
  { browser: 'Firefox', roots: ['.mozilla/firefox', 'snap/firefox/common/.mozilla/firefox', '.var/app/org.mozilla.firefox/.mozilla/firefox'] },
  { browser: 'LibreWolf', roots: ['.librewolf', '.var/app/io.gitlab.librewolf-community/.librewolf'] },
  { browser: 'Zen', roots: ['.zen', '.var/app/app.zen_browser.zen/.zen'] },
  { browser: 'Floorp', roots: ['.floorp', '.var/app/one.ablaze.floorp/.floorp'] },
]

let found: Found[] = []

// ── Chromium family ──────────────────────────────────────────────────────────

function chromiumProfiles(root: string): { dir: string; name: string }[] {
  const names = new Map<string, string>()
  try {
    const state = JSON.parse(readFileSync(join(root, 'Local State'), 'utf8')) as {
      profile?: { info_cache?: Record<string, { name?: string }> }
    }
    for (const [dir, info] of Object.entries(state.profile?.info_cache ?? {})) names.set(dir, info.name || dir)
  } catch {
    // No Local State: fall back to scanning folders.
  }
  const out: { dir: string; name: string }[] = []
  // Opera keeps a single profile in the root folder.
  if (existsSync(join(root, 'Bookmarks'))) out.push({ dir: root, name: names.get('Default') ?? 'Default' })
  let entries: string[] = []
  try {
    entries = readdirSync(root)
  } catch {
    return out
  }
  for (const entry of entries) {
    if (entry !== 'Default' && !/^Profile \d+$/.test(entry)) continue
    const dir = join(root, entry)
    if (existsSync(join(dir, 'Bookmarks'))) out.push({ dir, name: names.get(entry) ?? entry })
  }
  return out
}

function readChromium(dir: string): ParsedFile {
  return fromChromium(JSON.parse(readFileSync(join(dir, 'Bookmarks'), 'utf8')), t('bookmarks.mobile'))
}

// ── Firefox family ───────────────────────────────────────────────────────────

function firefoxProfiles(root: string): { dir: string; name: string }[] {
  const out: { dir: string; name: string }[] = []
  let ini = ''
  try {
    ini = readFileSync(join(root, 'profiles.ini'), 'utf8')
  } catch {
    return out
  }
  for (const section of ini.split(/^\[/m).slice(1)) {
    if (!/^Profile/i.test(section)) continue
    const value = (key: string) => new RegExp(`^${key}=(.*)$`, 'm').exec(section)?.[1]?.trim()
    const path = value('Path')
    if (!path) continue
    const dir = value('IsRelative') === '0' ? path : join(root, path)
    if (existsSync(join(dir, 'places.sqlite'))) out.push({ dir, name: value('Name') ?? path })
  }
  return out
}

/** Copies a SQLite database (and its write-ahead log) somewhere we can open it. */
function withCopy<T>(dir: string, name: string, read: (db: DatabaseSync) => T): T {
  const temp = mkdtempSync(join(tmpdir(), 'peepwick-import-'))
  try {
    copyFileSync(join(dir, name), join(temp, name))
    for (const suffix of ['-wal', '-shm']) {
      if (existsSync(join(dir, name + suffix))) copyFileSync(join(dir, name + suffix), join(temp, name + suffix))
    }
    const db = new DatabaseSync(join(temp, name))
    try {
      return read(db)
    } finally {
      db.close()
    }
  } finally {
    rmSync(temp, { recursive: true, force: true })
  }
}

function readFirefox(dir: string): ParsedFile {
  const rows = withCopy(dir, 'places.sqlite', (db) =>
    db
      .prepare(
        `SELECT b.id, b.type, b.parent, b.position, b.title, p.url, b.dateAdded, b.guid
         FROM moz_bookmarks b LEFT JOIN moz_places p ON p.id = b.fk`,
      )
      .all() as unknown as PlacesRow[],
  )
  return fromPlaces(rows, t('bookmarks.firefoxMenu'), t('bookmarks.mobile'))
}

// ── Favicons ─────────────────────────────────────────────────────────────────

function mimeOf(data: Uint8Array): string | null {
  if (data[0] === 0x89 && data[1] === 0x50) return 'image/png'
  if (data[0] === 0xff && data[1] === 0xd8) return 'image/jpeg'
  if (data[0] === 0x47 && data[1] === 0x49) return 'image/gif'
  if (data[0] === 0x00 && data[1] === 0x00 && data[2] === 0x01) return 'image/x-icon'
  if (data[0] === 0x3c) return 'image/svg+xml'
  return null
}

/** Copies icons for imported sites we have no icon for yet. Best effort. */
function importFavicons(source: Found, hosts: Set<string>): number {
  const wanted = new Set([...hosts].filter((host) => host && !getFavicon(host)))
  if (wanted.size === 0) return 0
  const best = new Map<string, { width: number; data: Uint8Array }>()
  const consider = (pageURL: string, data: Uint8Array | null, width: number) => {
    const host = hostOf(pageURL)
    if (!wanted.has(host) || !data || data.byteLength === 0 || data.byteLength > 100_000) return
    const current = best.get(host)
    // Prefer 32 px, then the closest size.
    if (!current || Math.abs(width - 32) < Math.abs(current.width - 32)) best.set(host, { width, data })
  }
  try {
    if (source.kind === 'chromium' && existsSync(join(source.dir, 'Favicons'))) {
      withCopy(source.dir, 'Favicons', (db) => {
        const rows = db
          .prepare(
            `SELECT m.page_url AS url, b.image_data AS data, b.width AS width
             FROM icon_mapping m JOIN favicon_bitmaps b ON b.icon_id = m.icon_id WHERE b.width <= 64`,
          )
          .all() as { url: string; data: Uint8Array; width: number }[]
        for (const row of rows) consider(row.url, row.data, row.width)
      })
    } else if (source.kind === 'firefox' && existsSync(join(source.dir, 'favicons.sqlite'))) {
      withCopy(source.dir, 'favicons.sqlite', (db) => {
        const rows = db
          .prepare(
            `SELECT p.page_url AS url, i.data AS data, i.width AS width
             FROM moz_pages_w_icons p JOIN moz_icons_to_pages ip ON ip.page_id = p.id
             JOIN moz_icons i ON i.id = ip.icon_id WHERE i.width <= 64`,
          )
          .all() as { url: string; data: Uint8Array; width: number }[]
        for (const row of rows) consider(row.url, row.data, row.width)
      })
    }
  } catch (error) {
    console.warn('[import] favicons skipped', error)
  }
  let count = 0
  for (const [host, { data }] of best) {
    const mime = mimeOf(data)
    if (!mime) continue
    setFavicon(host, `data:${mime};base64,${Buffer.from(data).toString('base64')}`)
    count++
  }
  return count
}

// ── Public API ───────────────────────────────────────────────────────────────

function safeCount(read: () => ParsedFile): number {
  try {
    const { tree } = read()
    return countNodes([...tree.bar, ...tree.other]).links
  } catch {
    return -1
  }
}

/** Looks for other browsers' profiles. Profiles without bookmarks are left out. */
export function detectSources(): ImportSource[] {
  const next: Found[] = []
  const seen = new Set<string>()
  const add = (kind: 'chromium' | 'firefox', browser: string, profile: { dir: string; name: string }) => {
    let key = profile.dir
    try {
      key = `${statSync(profile.dir).ino}`
    } catch {
      // Keep the path as the key.
    }
    if (seen.has(key)) return
    seen.add(key)
    const links = safeCount(() => (kind === 'chromium' ? readChromium(profile.dir) : readFirefox(profile.dir)))
    if (links <= 0) return
    next.push({ id: `${kind}:${next.length}`, browser, profile: profile.name, kind, links, dir: profile.dir })
  }
  for (const { browser, roots } of CHROMIUM) {
    for (const root of roots) for (const profile of chromiumProfiles(join(home, root))) add('chromium', browser, profile)
  }
  for (const { browser, roots } of FIREFOX) {
    for (const root of roots) for (const profile of firefoxProfiles(join(home, root))) add('firefox', browser, profile)
  }
  found = next
  return next.map(({ dir: _dir, ...source }) => source)
}

function hostsOf(tree: DraftTree): Set<string> {
  const hosts = new Set<string>()
  const go = (nodes: DraftNode[]) => {
    for (const node of nodes) {
      if (node.type === 'url') hosts.add(hostOf(node.url))
      else go(node.children)
    }
  }
  go(tree.bar)
  go(tree.other)
  return hosts
}

/** Imports from a detected profile, or from an HTML file when `sourceId` is "html". */
export async function importBookmarks(win: BrowserWin, sourceId: string, skipDuplicates: boolean): Promise<ImportResult | null> {
  let parsed: ParsedFile
  let label: string
  let source: Found | undefined
  if (sourceId === 'html') {
    const { canceled, filePaths } = await dialog.showOpenDialog(win.base, {
      title: t('import.title'),
      properties: ['openFile'],
      filters: [{ name: 'HTML', extensions: ['html', 'htm'] }],
    })
    if (canceled || !filePaths[0]) return null
    parsed = parseNetscape(readFileSync(filePaths[0], 'utf8'))
    label = t('import.fromFile')
  } else {
    source = found.find((s) => s.id === sourceId)
    if (!source) return null
    parsed = source.kind === 'chromium' ? readChromium(source.dir) : readFirefox(source.dir)
    label = t('import.folder', { browser: source.browser })
  }

  let draft = parsed.tree
  let duplicates = 0
  if (skipDuplicates) {
    const result = withoutDuplicates(draft, bookmarkedKeys())
    draft = result.tree
    duplicates = result.duplicates
  }
  const counts = countNodes([...draft.bar, ...draft.other])
  if (counts.links + counts.folders > 0) insertDraft(draft, label)
  const favicons = source ? importFavicons(source, hostsOf(draft)) : 0
  return { links: counts.links, folders: counts.folders, duplicates, skipped: parsed.skipped, favicons }
}
