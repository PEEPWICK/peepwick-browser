// Importing from other browsers on this machine (Chromium family and Firefox
// family, including Snap and Flatpak installs): bookmarks, passwords, history,
// cookies and form data; bookmarks also from an HTML file, passwords from CSV.
// Browsers keep their databases locked while running, so SQLite files are copied
// to a private temporary folder and read from there. Only files on this computer
// are read; nothing is sent anywhere.
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
import { loginsFromCsv } from '@shared/passwordcsv'
import type { Address, ImportKind, ImportResult, ImportSource } from '@shared/types'
import { hostOf } from '@shared/url'
import { bookmarkedKeys, insertDraft } from './bookmarks'
import { chromiumKeys, decryptChromium, stripDomainHash, type ChromiumKeys } from './chromiumcrypto'
import { decryptLoginField, loginKeys, PrimaryPasswordError } from './firefoxcrypto'
import { importAddresses, importEntries, looksLikeCard, type FormEntry } from './formdata'
import { getFavicon, importVisits, setFavicon, type VisitRow } from './history'
import { importLogins, type PlainLogin } from './passwords'
import { browsingSession } from './sessions'
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
  if (chromiumKinds(root).length > 0) out.push({ dir: root, name: names.get('Default') ?? 'Default' })
  let entries: string[] = []
  try {
    entries = readdirSync(root)
  } catch {
    return out
  }
  for (const entry of entries) {
    if (entry !== 'Default' && !/^Profile \d+$/.test(entry)) continue
    const dir = join(root, entry)
    if (chromiumKinds(dir).length > 0) out.push({ dir, name: names.get(entry) ?? entry })
  }
  return out
}

function chromiumCookies(dir: string): string | null {
  if (existsSync(join(dir, 'Network', 'Cookies'))) return join(dir, 'Network')
  return existsSync(join(dir, 'Cookies')) ? dir : null
}

function chromiumKinds(dir: string): ImportKind[] {
  const kinds: ImportKind[] = []
  if (existsSync(join(dir, 'Bookmarks'))) kinds.push('bookmarks')
  if (existsSync(join(dir, 'Login Data'))) kinds.push('passwords')
  if (existsSync(join(dir, 'History'))) kinds.push('history')
  if (chromiumCookies(dir)) kinds.push('cookies')
  if (existsSync(join(dir, 'Web Data'))) kinds.push('autofill')
  return kinds
}

function firefoxKinds(dir: string): ImportKind[] {
  const kinds: ImportKind[] = []
  if (existsSync(join(dir, 'places.sqlite'))) kinds.push('bookmarks', 'history')
  if (existsSync(join(dir, 'logins.json')) && existsSync(join(dir, 'key4.db'))) kinds.push('passwords')
  if (existsSync(join(dir, 'cookies.sqlite'))) kinds.push('cookies')
  if (existsSync(join(dir, 'formhistory.sqlite')) || existsSync(join(dir, 'autofill-profiles.json'))) kinds.push('autofill')
  return kinds
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
    if (firefoxKinds(dir).length > 0) out.push({ dir, name: value('Name') ?? path })
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

/** Looks for other browsers' profiles that hold anything we can import. */
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
    let available = kind === 'chromium' ? chromiumKinds(profile.dir) : firefoxKinds(profile.dir)
    const links = available.includes('bookmarks') ? safeCount(() => (kind === 'chromium' ? readChromium(profile.dir) : readFirefox(profile.dir))) : 0
    if (links <= 0) available = available.filter((k) => k !== 'bookmarks')
    if (available.length === 0) return
    next.push({ id: `${kind}:${next.length}`, browser, profile: profile.name, kind, links: Math.max(0, links), available, dir: profile.dir })
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

// ── Passwords, history, cookies and form data ────────────────────────────────

/**
 * Chromium stores times as microseconds since 1601: about 1.3·10¹⁶, past what a
 * JavaScript number holds exactly, and node:sqlite refuses to read such integers.
 * The queries convert them in SQL to milliseconds (or seconds) since 1970.
 */
const WEBKIT_MS = '11644473600000'
const webkitMs = (column: string) => `CASE WHEN ${column} > 0 THEN ${column} / 1000 - ${WEBKIT_MS} ELSE 0 END`

function chromiumPasswords(dir: string, keys: ChromiumKeys): { logins: PlainLogin[]; never: string[]; undecrypted: number } {
  const logins: PlainLogin[] = []
  const never: string[] = []
  let undecrypted = 0
  // Signed-in Chrome keeps account passwords in a second database.
  for (const name of ['Login Data', 'Login Data For Account']) {
    if (!existsSync(join(dir, name))) continue
    withCopy(dir, name, (db) => {
      const columns = new Set((db.prepare('PRAGMA table_info(logins)').all() as { name: string }[]).map((c) => c.name))
      const modified = columns.has('date_password_modified') ? webkitMs('date_password_modified') : '0'
      const blocked = columns.has('blacklisted_by_user') ? 'blacklisted_by_user' : '0'
      const rows = db
        .prepare(
          `SELECT origin_url, username_value, password_value, ${webkitMs('date_created')} AS created, ${modified} AS modified, ${blocked} AS never FROM logins`,
        )
        .all() as { origin_url: string; username_value: string; password_value: Uint8Array; created: number; modified: number; never: number }[]
      for (const row of rows) {
        if (!/^https?:/.test(row.origin_url)) continue
        if (row.never) {
          never.push(row.origin_url)
          continue
        }
        const plain = decryptChromium(row.password_value, keys)
        if (plain === null) {
          undecrypted++
          continue
        }
        const created = row.created || undefined
        const updated = row.modified || created
        logins.push({ origin: row.origin_url, username: row.username_value, password: plain.toString('utf8'), created, updated })
      }
    })
  }
  return { logins, never, undecrypted }
}

function firefoxPasswords(dir: string, primaryPassword: string): { logins: PlainLogin[]; never: string[] } {
  const keys = withCopy(dir, 'key4.db', (db) => {
    const meta = db.prepare("SELECT item1, item2 FROM metadata WHERE id = 'password'").get() as { item1: Uint8Array; item2: Uint8Array }
    const privates = db.prepare('SELECT a11, a102 FROM nssPrivate').all() as { a11: Uint8Array; a102: Uint8Array }[]
    return loginKeys(meta, privates, primaryPassword)
  })
  const json = JSON.parse(readFileSync(join(dir, 'logins.json'), 'utf8')) as {
    logins?: { hostname: string; encryptedUsername: string; encryptedPassword: string; timeCreated?: number; timePasswordChanged?: number }[]
    disabledHosts?: string[]
  }
  const logins: PlainLogin[] = []
  for (const login of json.logins ?? []) {
    if (!/^https?:/.test(login.hostname)) continue
    try {
      logins.push({
        origin: login.hostname,
        username: decryptLoginField(login.encryptedUsername, keys),
        password: decryptLoginField(login.encryptedPassword, keys),
        created: login.timeCreated,
        updated: login.timePasswordChanged ?? login.timeCreated,
      })
    } catch {
      // One unreadable login doesn't stop the rest.
    }
  }
  return { logins, never: json.disabledHosts ?? [] }
}

function chromiumHistory(dir: string): VisitRow[] {
  return withCopy(dir, 'History', (db) =>
    (
      db
        .prepare(
          `SELECT u.url AS url, u.title AS title, ${webkitMs('v.visit_time')} AS time FROM visits v JOIN urls u ON u.id = v.url ORDER BY v.visit_time DESC LIMIT 100000`,
        )
        .all() as { url: string; title: string; time: number }[]
    ).map((r) => ({ url: r.url, title: r.title ?? '', visitedAt: r.time })),
  )
}

function firefoxHistory(dir: string): VisitRow[] {
  return withCopy(dir, 'places.sqlite', (db) =>
    (
      db
        .prepare(
          'SELECT p.url AS url, p.title AS title, v.visit_date AS time FROM moz_historyvisits v JOIN moz_places p ON p.id = v.place_id ORDER BY v.visit_date DESC LIMIT 100000',
        )
        .all() as { url: string; title: string | null; time: number }[]
    ).map((r) => ({ url: r.url, title: r.title ?? '', visitedAt: Math.round(Number(r.time) / 1000) })),
  )
}

interface CookieRow {
  host: string
  name: string
  value: string
  path: string
  /** Seconds since 1970. */
  expires: number
  secure: boolean
  httpOnly: boolean
  sameSite: 'unspecified' | 'no_restriction' | 'lax' | 'strict'
}

const SAME_SITE: CookieRow['sameSite'][] = ['no_restriction', 'lax', 'strict']

function chromiumCookieRows(dir: string, keys: ChromiumKeys): { rows: CookieRow[]; undecrypted: number } {
  const folder = chromiumCookies(dir)
  if (!folder) return { rows: [], undecrypted: 0 }
  let undecrypted = 0
  const rows = withCopy(folder, 'Cookies', (db) => {
    const version = Number((db.prepare("SELECT value FROM meta WHERE key = 'version'").get() as { value: string } | undefined)?.value ?? 0)
    const now = Date.now() / 1000
    return (
      db
        .prepare(
          `SELECT host_key, name, value, encrypted_value, path, CASE WHEN expires_utc > 0 THEN expires_utc / 1000000 - 11644473600 ELSE 0 END AS expires, is_secure, is_httponly, samesite, is_persistent FROM cookies`,
        )
        .all() as {
        host_key: string
        name: string
        value: string
        encrypted_value: Uint8Array
        path: string
        expires: number
        is_secure: number
        is_httponly: number
        samesite: number
        is_persistent: number
      }[]
    ).flatMap((r): CookieRow[] => {
      const expires = r.expires
      if (!r.is_persistent || expires < now) return []
      let value = r.value
      if (r.encrypted_value && r.encrypted_value.byteLength > 0) {
        const plain = decryptChromium(r.encrypted_value, keys)
        if (plain === null) {
          undecrypted++
          return []
        }
        value = (version >= 24 ? stripDomainHash(plain, r.host_key) : plain).toString('utf8')
      }
      return [
        {
          host: r.host_key,
          name: r.name,
          value,
          path: r.path || '/',
          expires,
          secure: !!r.is_secure,
          httpOnly: !!r.is_httponly,
          sameSite: SAME_SITE[r.samesite] ?? 'unspecified',
        },
      ]
    })
  })
  return { rows, undecrypted }
}

function firefoxCookieRows(dir: string): CookieRow[] {
  return withCopy(dir, 'cookies.sqlite', (db) => {
    const now = Date.now() / 1000
    return (
      db.prepare("SELECT host, name, value, path, expiry, isSecure, isHttpOnly, sameSite FROM moz_cookies WHERE originAttributes = ''").all() as {
        host: string
        name: string
        value: string
        path: string
        expiry: number
        isSecure: number
        isHttpOnly: number
        sameSite: number
      }[]
    ).flatMap((r): CookieRow[] => {
      // Newer Firefox stores the expiry in milliseconds.
      const expires = Number(r.expiry) > 1e11 ? Number(r.expiry) / 1000 : Number(r.expiry)
      if (expires < now) return []
      return [
        {
          host: r.host,
          name: r.name,
          value: r.value,
          path: r.path || '/',
          expires,
          secure: !!r.isSecure,
          httpOnly: !!r.isHttpOnly,
          sameSite: SAME_SITE[r.sameSite] ?? 'unspecified',
        },
      ]
    })
  })
}

async function setCookies(rows: CookieRow[]): Promise<number> {
  const cookies = browsingSession().cookies
  let set = 0
  for (const row of rows.slice(0, 20_000)) {
    const domain = row.host.startsWith('.')
    const host = row.host.replace(/^\./, '')
    try {
      await cookies.set({
        url: `${row.secure ? 'https' : 'http'}://${host}${row.path}`,
        name: row.name,
        value: row.value,
        // Host-only cookies stay host-only: no domain attribute.
        ...(domain ? { domain: row.host } : {}),
        path: row.path,
        secure: row.secure,
        httpOnly: row.httpOnly,
        expirationDate: row.expires,
        sameSite: row.sameSite,
      })
      set++
    } catch {
      // Cookies the browser refuses (bad names, odd domains) are skipped.
    }
  }
  return set
}

/** Chromium's address field types (server field types) we keep. */
const CHROMIUM_ADDRESS_TYPES: Record<number, keyof Omit<Address, 'id'>> = {
  7: 'name',
  9: 'email',
  14: 'phone',
  60: 'organization',
  77: 'street',
  30: 'street',
  33: 'city',
  34: 'state',
  35: 'postalCode',
  36: 'country',
}

function chromiumAutofill(dir: string): { entries: FormEntry[]; addresses: Partial<Address>[] } {
  return withCopy(dir, 'Web Data', (db) => {
    let entries: FormEntry[] = []
    try {
      entries = (db.prepare('SELECT name, value, count, date_last_used FROM autofill').all() as { name: string; value: string; count: number; date_last_used: number }[]).map(
        (r) => ({ key: r.name.toLowerCase(), value: r.value, count: r.count, lastUsed: Number(r.date_last_used) * 1000 }),
      )
    } catch {
      entries = []
    }
    const addresses: Partial<Address>[] = []
    try {
      // Current schema: one row per field.
      const tokens = db.prepare('SELECT guid, type, value FROM address_type_tokens').all() as { guid: string; type: number; value: string }[]
      const byGuid = new Map<string, Partial<Address>>()
      for (const token of tokens) {
        const field = CHROMIUM_ADDRESS_TYPES[token.type]
        if (!field || !token.value) continue
        const address = byGuid.get(token.guid) ?? {}
        // A street address beats a first line.
        if (field !== 'street' || token.type === 77 || !address.street) address[field] = token.value
        byGuid.set(token.guid, address)
      }
      addresses.push(...byGuid.values())
    } catch {
      try {
        // Older schema.
        const rows = db
          .prepare(
            `SELECT p.guid, p.company_name, p.street_address, p.city, p.state, p.zipcode, p.country_code,
               (SELECT full_name FROM autofill_profile_names n WHERE n.guid = p.guid LIMIT 1) AS name,
               (SELECT email FROM autofill_profile_emails e WHERE e.guid = p.guid LIMIT 1) AS email,
               (SELECT number FROM autofill_profile_phones f WHERE f.guid = p.guid LIMIT 1) AS phone
             FROM autofill_profiles p`,
          )
          .all() as Record<string, string>[]
        for (const r of rows) {
          addresses.push({ name: r.name, organization: r.company_name, street: r.street_address, city: r.city, state: r.state, postalCode: r.zipcode, country: r.country_code, email: r.email, phone: r.phone })
        }
      } catch {
        // No address tables in this profile.
      }
    }
    return { entries, addresses }
  })
}

function firefoxAutofill(dir: string): { entries: FormEntry[]; addresses: Partial<Address>[] } {
  let entries: FormEntry[] = []
  if (existsSync(join(dir, 'formhistory.sqlite'))) {
    entries = withCopy(dir, 'formhistory.sqlite', (db) =>
      (db.prepare('SELECT fieldname, value, timesUsed, lastUsed FROM moz_formhistory').all() as { fieldname: string; value: string; timesUsed: number; lastUsed: number }[]).map((r) => ({
        key: r.fieldname.toLowerCase(),
        value: r.value,
        count: r.timesUsed,
        lastUsed: Math.round(Number(r.lastUsed) / 1000),
      })),
    )
  }
  const addresses: Partial<Address>[] = []
  try {
    const json = JSON.parse(readFileSync(join(dir, 'autofill-profiles.json'), 'utf8')) as { addresses?: Record<string, string>[] }
    for (const a of json.addresses ?? []) {
      const name = a['name'] || [a['given-name'], a['additional-name'], a['family-name']].filter(Boolean).join(' ')
      addresses.push({
        name,
        organization: a['organization'],
        street: a['street-address'],
        city: a['address-level2'],
        state: a['address-level1'],
        postalCode: a['postal-code'],
        country: a['country'],
        phone: a['tel'],
        email: a['email'],
      })
    }
  } catch {
    // No saved addresses.
  }
  return { entries, addresses }
}

export interface ImportOptions {
  kinds: ImportKind[]
  skipDuplicates: boolean
  /** Firefox's primary password, when the profile has one. */
  primaryPassword?: string
}

/** Imports the chosen kinds of data from a detected profile. */
export async function importFrom(win: BrowserWin, sourceId: string, options: ImportOptions): Promise<ImportResult | null> {
  const source = found.find((s) => s.id === sourceId)
  if (!source) return null
  const result: ImportResult = { links: 0, folders: 0, duplicates: 0, skipped: 0, favicons: 0, failed: [] }
  const wants = (kind: ImportKind) => options.kinds.includes(kind) && source.available.includes(kind)
  const fail = (kind: ImportKind, error: unknown) => {
    console.error(`[import] ${kind} failed`, error instanceof Error ? error.message : error)
    result.failed!.push(kind)
  }

  if (wants('bookmarks')) {
    try {
      Object.assign(result, await importBookmarks(win, sourceId, options.skipDuplicates))
    } catch (error) {
      fail('bookmarks', error)
    }
  }

  // Chromium's key comes from the keyring once, for passwords and cookies both.
  let keys: ChromiumKeys | null = null
  const chromium = async () => (keys ??= await chromiumKeys(source.browser))

  if (wants('passwords')) {
    try {
      if (source.kind === 'chromium') {
        const { logins, never, undecrypted } = chromiumPasswords(source.dir, await chromium())
        const { added, duplicates } = importLogins(logins, never)
        result.passwords = added
        result.passwordDuplicates = duplicates
        if (undecrypted > 0 && logins.length === 0) result.failed!.push('passwords')
      } else {
        const { logins, never } = firefoxPasswords(source.dir, options.primaryPassword ?? '')
        const { added, duplicates } = importLogins(logins, never)
        result.passwords = added
        result.passwordDuplicates = duplicates
      }
    } catch (error) {
      if (error instanceof PrimaryPasswordError) result.needsPrimaryPassword = true
      else fail('passwords', error)
    }
  }
  if (wants('history')) {
    try {
      result.history = importVisits(source.kind === 'chromium' ? chromiumHistory(source.dir) : firefoxHistory(source.dir))
    } catch (error) {
      fail('history', error)
    }
  }
  if (wants('cookies')) {
    try {
      if (source.kind === 'chromium') {
        const { rows, undecrypted } = chromiumCookieRows(source.dir, await chromium())
        result.cookies = await setCookies(rows)
        if (undecrypted > 0 && rows.length === 0) result.failed!.push('cookies')
      } else result.cookies = await setCookies(firefoxCookieRows(source.dir))
    } catch (error) {
      fail('cookies', error)
    }
  }
  if (wants('autofill')) {
    try {
      const { entries, addresses } = source.kind === 'chromium' ? chromiumAutofill(source.dir) : firefoxAutofill(source.dir)
      result.formEntries = importEntries(entries.filter((e) => !looksLikeCard(e.value)))
      result.addresses = importAddresses(addresses)
    } catch (error) {
      fail('autofill', error)
    }
  }
  return result
}

/** Passwords from a CSV export (Chrome, Firefox, Bitwarden, 1Password, LastPass, KeePassXC). */
export async function importPasswordCsv(win: BrowserWin): Promise<ImportResult | null> {
  const { canceled, filePaths } = await dialog.showOpenDialog(win.base, {
    title: t('passwords.import'),
    properties: ['openFile'],
    filters: [{ name: 'CSV', extensions: ['csv'] }],
  })
  if (canceled || !filePaths[0]) return null
  const { logins, skipped } = loginsFromCsv(readFileSync(filePaths[0], 'utf8'))
  const { added, duplicates } = importLogins(logins.map((l) => ({ origin: l.url, username: l.username, password: l.password, note: l.note })))
  return { links: 0, folders: 0, duplicates: 0, skipped, favicons: 0, passwords: added, passwordDuplicates: duplicates }
}
