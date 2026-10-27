// Password CSV files: what Chrome, Firefox, Bitwarden, 1Password, LastPass and
// KeePassXC export, and Chrome's format for our own export. Pure functions.

export interface CsvLogin {
  url: string
  username: string
  password: string
  note: string
}

/** RFC 4180: quoted fields, doubled quotes, newlines inside quotes, a BOM at the start. */
export function parseCsv(text: string): string[][] {
  const rows: string[][] = []
  let row: string[] = []
  let field = ''
  let quoted = false
  const source = text.replace(/^﻿/, '')
  for (let i = 0; i < source.length; i++) {
    const c = source[i]
    if (quoted) {
      if (c === '"' && source[i + 1] === '"') {
        field += '"'
        i++
      } else if (c === '"') quoted = false
      else field += c
    } else if (c === '"') quoted = true
    else if (c === ',') {
      row.push(field)
      field = ''
    } else if (c === '\n' || c === '\r') {
      if (c === '\r' && source[i + 1] === '\n') i++
      row.push(field)
      field = ''
      if (row.some((cell) => cell !== '')) rows.push(row)
      row = []
    } else field += c
  }
  row.push(field)
  if (row.some((cell) => cell !== '')) rows.push(row)
  return rows
}

const COLUMNS: Record<keyof CsvLogin, string[]> = {
  url: ['url', 'login_uri', 'website', 'web site', 'login url', 'uri', 'origin', 'hostname', 'address'],
  username: ['username', 'login_username', 'user name', 'user', 'login', 'email', 'e-mail', 'login name'],
  password: ['password', 'login_password', 'pass'],
  note: ['note', 'notes', 'extra', 'comment', 'comments'],
}

function columnOf(header: string[], names: string[]): number {
  const normalized = header.map((h) => h.trim().toLowerCase())
  for (const name of names) {
    const index = normalized.indexOf(name)
    if (index >= 0) return index
  }
  return -1
}

/** Reads logins from a CSV export; rows without an address or password are left out. */
export function loginsFromCsv(text: string): { logins: CsvLogin[]; skipped: number } {
  const [header, ...rows] = parseCsv(text)
  if (!header) return { logins: [], skipped: 0 }
  const at = {
    url: columnOf(header, COLUMNS.url),
    username: columnOf(header, COLUMNS.username),
    password: columnOf(header, COLUMNS.password),
    note: columnOf(header, COLUMNS.note),
  }
  if (at.url < 0 || at.password < 0) return { logins: [], skipped: rows.length }
  const logins: CsvLogin[] = []
  let skipped = 0
  for (const row of rows) {
    const cell = (i: number) => (i >= 0 ? (row[i] ?? '').trim() : '')
    const login = { url: cell(at.url), username: cell(at.username), password: row[at.password] ?? '', note: cell(at.note) }
    if (!login.url || !login.password || !originOf(login.url)) skipped++
    else logins.push(login)
  }
  return { logins, skipped }
}

/** https://example.com for a login address; adds https:// to a bare host. */
export function originOf(url: string): string | null {
  let text = url.trim()
  if (!text) return null
  if (!/^[a-z][a-z0-9+.-]*:\/\//i.test(text)) text = `https://${text}`
  try {
    const parsed = new URL(text)
    if (parsed.protocol !== 'https:' && parsed.protocol !== 'http:') return null
    return parsed.origin
  } catch {
    return null
  }
}

function quote(value: string): string {
  return /[",\r\n]/.test(value) ? `"${value.replace(/"/g, '""')}"` : value
}

/** Chrome's columns, which every password manager reads. */
export function loginsToCsv(logins: { name: string; url: string; username: string; password: string; note: string }[]): string {
  const lines = ['name,url,username,password,note']
  for (const l of logins) lines.push([l.name, l.url, l.username, l.password, l.note].map(quote).join(','))
  return `${lines.join('\n')}\n`
}

const COMMON = new Set([
  '123456', '123456789', '12345678', 'password', 'qwerty', '12345', '1234567', '111111', '123123', 'abc123', 'password1',
  '1234567890', '000000', 'iloveyou', 'qwerty123', '1q2w3e4r', 'admin', 'welcome', 'letmein', 'monkey', 'dragon',
  'sunshine', 'football', 'baseball', 'superman', 'trustno1', 'parola', 'sifre', 'şifre', 'galatasaray', 'fenerbahce',
  'besiktas', 'asdasd', 'qweasd', '123qwe', 'zxcvbnm', 'passw0rd',
])

/** Short, common, or a single kind of character and not long enough to make up for it. */
export function isWeakPassword(password: string): boolean {
  if (password.length < 8) return true
  if (COMMON.has(password.toLocaleLowerCase('tr'))) return true
  const kinds = [/[a-zçğıöşü]/, /[A-ZÇĞİÖŞÜ]/, /\d/, /[^\p{L}\d]/u].filter((re) => re.test(password)).length
  if (kinds === 1 && password.length < 14) return true
  if (/^(.)\1+$/.test(password)) return true
  return false
}

/** A strong random password for sign-up forms: 16 characters from four kinds, no look-alikes. */
export function generatePassword(random: (n: number) => Uint8Array, length = 16): string {
  const sets = ['abcdefghjkmnpqrstuvwxyz', 'ABCDEFGHJKLMNPQRSTUVWXYZ', '23456789', '-_.!?#']
  const all = sets.join('')
  // Bytes above the last whole multiple of the alphabet are skipped, so every
  // character is equally likely.
  const limit = 256 - (256 % all.length)
  for (;;) {
    let out = ''
    while (out.length < length) for (const b of random(length)) if (b < limit && out.length < length) out += all[b % all.length]
    if (sets.every((set) => [...out].some((c) => set.includes(c)))) return out
  }
}
