// Saved passwords. Each one is encrypted with Electron's safeStorage, whose key
// lives in the desktop keyring (GNOME Keyring, KWallet); on a system without one it
// falls back to a fixed key, and the passwords page says so. Passwords leave this
// module only to be filled into a page of the same site, copied, or shown on the
// passwords page at the person's request.
import { clipboard, dialog, safeStorage } from 'electron'
import { randomUUID } from 'node:crypto'
import { writeFile } from 'node:fs/promises'
import { isWeakPassword, loginsToCsv, originOf } from '@shared/passwordcsv'
import type { LoginEntry, PasswordStatus } from '@shared/types'
import { hostOf } from '@shared/url'
import { userPath } from './paths'
import { t } from './settings'
import { siteOf } from './shield'
import { JsonFile } from './store'
import { BrowserWin } from './window'

interface Stored {
  id: string
  origin: string
  username: string
  /** base64 of safeStorage's ciphertext */
  secret: string
  created: number
  updated: number
  lastUsed: number
  timesUsed: number
  note: string
}

interface PasswordFile {
  version: 1
  logins: Stored[]
  /** Sites where the person chose "never save". */
  never: string[]
}

export interface PlainLogin {
  origin: string
  username: string
  password: string
  note?: string
  created?: number
  updated?: number
}

const file = new JsonFile<PasswordFile>(userPath('passwords.json'), 300)
let data: PasswordFile = { version: 1, logins: [], never: [] }
let lastRemoved: Stored[] = []

export function loadPasswords(): void {
  const saved = file.read()
  if (saved?.version === 1 && Array.isArray(saved.logins)) data = { version: 1, logins: saved.logins, never: Array.isArray(saved.never) ? saved.never : [] }
}

export function flushPasswords(): void {
  file.flush()
}

function changed(): void {
  file.schedule(data)
  // Passwords pages in every window refresh themselves.
  for (const win of BrowserWin.all.values()) win.send('command', { type: 'passwords-changed' })
}

function encrypt(password: string): string {
  return safeStorage.encryptString(password).toString('base64')
}

function decrypt(secret: string): string | null {
  try {
    return safeStorage.decryptString(Buffer.from(secret, 'base64'))
  } catch {
    return null
  }
}

export function passwordStatus(): PasswordStatus {
  let backend = 'unknown'
  try {
    backend = safeStorage.getSelectedStorageBackend()
  } catch {
    backend = safeStorage.isEncryptionAvailable() ? 'unknown' : 'basic_text'
  }
  return { count: data.logins.length, backend, never: [...data.never] }
}

export function listLogins(): LoginEntry[] {
  // Reuse: the same password on more than one site.
  const sites = new Map<string, Set<string>>()
  const plain = new Map<string, string | null>()
  for (const login of data.logins) {
    const password = decrypt(login.secret)
    plain.set(login.id, password)
    if (!password) continue
    const set = sites.get(password) ?? new Set()
    set.add(siteOf(login.origin) ?? login.origin)
    sites.set(password, set)
  }
  return data.logins
    .map((login) => {
      const password = plain.get(login.id) ?? ''
      return {
        id: login.id,
        origin: login.origin,
        username: login.username,
        created: login.created,
        updated: login.updated,
        lastUsed: login.lastUsed,
        timesUsed: login.timesUsed,
        note: login.note,
        weak: !!password && isWeakPassword(password),
        reused: !!password && (sites.get(password)?.size ?? 0) > 1,
      }
    })
    .sort((a, b) => hostOf(a.origin).localeCompare(hostOf(b.origin)) || a.username.localeCompare(b.username))
}

export function revealPassword(id: string): string | null {
  const login = data.logins.find((l) => l.id === id)
  return login ? decrypt(login.secret) : null
}

let clearTimer: NodeJS.Timeout | null = null

/** Copies a password; it leaves the clipboard after a minute unless something replaced it. */
export function copyPassword(id: string): boolean {
  const password = revealPassword(id)
  if (password === null) return false
  clipboard.writeText(password)
  if (clearTimer) clearTimeout(clearTimer)
  clearTimer = setTimeout(() => {
    void Promise.resolve(clipboard.readText()).then((text) => text === password && clipboard.clear())
  }, 60_000)
  return true
}

function find(origin: string, username: string): Stored | undefined {
  return data.logins.find((l) => l.origin === origin && l.username === username)
}

export function addLogin(input: PlainLogin): string | null {
  const origin = originOf(input.origin)
  if (!origin || !input.password) return null
  const now = Date.now()
  const existing = find(origin, input.username)
  if (existing) {
    existing.secret = encrypt(input.password)
    existing.updated = now
    if (input.note !== undefined) existing.note = input.note
    changed()
    return existing.id
  }
  const login: Stored = {
    id: randomUUID(),
    origin,
    username: input.username.slice(0, 500),
    secret: encrypt(input.password.slice(0, 1000)),
    created: input.created ?? now,
    updated: input.updated ?? now,
    lastUsed: 0,
    timesUsed: 0,
    note: (input.note ?? '').slice(0, 2000),
  }
  data.logins.push(login)
  data.never = data.never.filter((o) => o !== origin)
  changed()
  return login.id
}

export function updateLogin(id: string, patch: { origin?: string; username?: string; password?: string; note?: string }): boolean {
  const login = data.logins.find((l) => l.id === id)
  if (!login) return false
  if (patch.origin !== undefined) {
    const origin = originOf(patch.origin)
    if (!origin) return false
    login.origin = origin
  }
  if (patch.username !== undefined) login.username = patch.username.slice(0, 500)
  if (patch.password) login.secret = encrypt(patch.password.slice(0, 1000))
  if (patch.note !== undefined) login.note = patch.note.slice(0, 2000)
  login.updated = Date.now()
  changed()
  return true
}

export function removeLogins(ids: string[]): number {
  const remove = new Set(ids)
  lastRemoved = data.logins.filter((l) => remove.has(l.id))
  data.logins = data.logins.filter((l) => !remove.has(l.id))
  if (lastRemoved.length > 0) changed()
  return lastRemoved.length
}

export function undoRemoveLogins(): void {
  if (lastRemoved.length === 0) return
  data.logins.push(...lastRemoved.filter((r) => !data.logins.some((l) => l.id === r.id)))
  lastRemoved = []
  changed()
}

export function clearPasswords(): void {
  data = { version: 1, logins: [], never: data.never }
  changed()
}

// ── Filling and saving ───────────────────────────────────────────────────────

/** Logins for a page: this exact origin first, then others on the same site. */
export function credentialsFor(origin: string): { id: string; username: string; origin: string; exact: boolean }[] {
  const site = siteOf(origin)
  return data.logins
    .filter((l) => l.origin === origin || (site && siteOf(l.origin) === site && /^https:/.test(l.origin) === /^https:/.test(origin)))
    .map((l) => ({ id: l.id, username: l.username, origin: l.origin, exact: l.origin === origin }))
    .sort((a, b) => Number(b.exact) - Number(a.exact) || a.username.localeCompare(b.username))
}

/** The login to fill into a page of `origin`, if it belongs to the same site. */
export function loginFor(id: string, origin: string): { username: string; password: string } | null {
  const login = data.logins.find((l) => l.id === id)
  if (!login) return null
  if (login.origin !== origin && siteOf(login.origin) !== siteOf(origin)) return null
  const password = decrypt(login.secret)
  if (password === null) return null
  login.lastUsed = Date.now()
  login.timesUsed++
  file.schedule(data)
  return { username: login.username, password }
}

/** After a sign-in: is there anything to offer? */
export function afterSubmit(origin: string, username: string, password: string): 'none' | 'save' | 'update' {
  if (data.never.includes(origin)) return 'none'
  const same = find(origin, username)
  if (!same) return 'save'
  const current = decrypt(same.secret)
  if (current === password) {
    same.lastUsed = Date.now()
    same.timesUsed++
    file.schedule(data)
    return 'none'
  }
  return 'update'
}

export function neverSave(origin: string): void {
  if (!data.never.includes(origin)) data.never.push(origin)
  changed()
}

export function removeNever(origin: string): void {
  data.never = data.never.filter((o) => o !== origin)
  changed()
}

// ── Import, export, backup ───────────────────────────────────────────────────

/**
 * Adds logins from another browser, a CSV file or a backup. A login already here
 * (same site and username) keeps the newer password.
 */
export function importLogins(logins: PlainLogin[], never: string[] = []): { added: number; duplicates: number } {
  let added = 0
  let duplicates = 0
  const now = Date.now()
  for (const input of logins) {
    const origin = originOf(input.origin)
    if (!origin || !input.password) continue
    const existing = find(origin, input.username)
    if (existing) {
      const current = decrypt(existing.secret)
      if (current === input.password || (input.updated ?? 0) <= existing.updated) {
        duplicates++
        continue
      }
      existing.secret = encrypt(input.password)
      existing.updated = input.updated ?? now
      added++
      continue
    }
    data.logins.push({
      id: randomUUID(),
      origin,
      username: input.username.slice(0, 500),
      secret: encrypt(input.password.slice(0, 1000)),
      created: input.created ?? now,
      updated: input.updated ?? input.created ?? now,
      lastUsed: 0,
      timesUsed: 0,
      note: (input.note ?? '').slice(0, 2000),
    })
    added++
  }
  for (const origin of never) {
    const clean = originOf(origin)
    if (clean && !data.never.includes(clean) && !data.logins.some((l) => l.origin === clean)) data.never.push(clean)
  }
  if (added > 0 || never.length > 0) changed()
  return { added, duplicates }
}

export function plainLogins(): PlainLogin[] {
  return data.logins.flatMap((l) => {
    const password = decrypt(l.secret)
    return password === null ? [] : [{ origin: l.origin, username: l.username, password, note: l.note, created: l.created, updated: l.updated }]
  })
}

export async function exportPasswords(win: BrowserWin): Promise<boolean> {
  const { canceled, filePath } = await dialog.showSaveDialog(win.base, {
    title: t('passwords.export'),
    defaultPath: `${t('passwords.fileName')}-${new Date().toISOString().slice(0, 10)}.csv`,
    filters: [{ name: 'CSV', extensions: ['csv'] }],
  })
  if (canceled || !filePath) return false
  const csv = loginsToCsv(plainLogins().map((l) => ({ name: hostOf(l.origin), url: l.origin, username: l.username, password: l.password, note: l.note ?? '' })))
  await writeFile(filePath, csv, { mode: 0o600 })
  return true
}
