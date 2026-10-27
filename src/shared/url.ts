// Dependency-free URL helpers used by both the main process and the UI.
import type { InternalPage, TabSecurity } from './types'

export const INTERNAL_SCHEME = 'peepwick'
export const NEW_TAB_URL = 'peepwick://newtab'

const INTERNAL_PAGES: InternalPage[] = ['newtab', 'settings', 'history', 'extensions', 'bookmarks', 'downloads', 'passwords', 'reading']

export function internalPage(url: string): InternalPage | null {
  const match = /^peepwick:\/\/([a-z]+)\/?/i.exec(url.trim())
  if (!match) return null
  const page = match[1].toLowerCase() as InternalPage
  return INTERNAL_PAGES.includes(page) ? page : null
}

/** peepwick://settings/privacy: the part after the page, or the old #privacy form. */
export function internalSubpage(url: string): string {
  const match = /^peepwick:\/\/[a-z]+\/?([a-z-]*)(?:#([a-z-]+))?/i.exec(url.trim())
  return (match?.[1] || match?.[2] || '').toLowerCase()
}

export function internalURL(page: InternalPage, subpage = ''): string {
  return `peepwick://${page}${subpage ? `/${subpage}` : ''}`
}

export function securityOf(url: string): TabSecurity {
  if (url.startsWith('https:')) return 'secure'
  if (url.startsWith('http:')) {
    const host = hostOf(url)
    return isLocalHost(host) ? 'local' : 'insecure'
  }
  if (url.startsWith('file:')) return 'local'
  return 'internal'
}

/**
 * Hosts on this machine or the local network, which can't get a certificate from a
 * public authority and so are left on HTTP by HTTPS-Only mode.
 */
export function isLocalHost(host: string): boolean {
  const h = host.toLowerCase().replace(/^\[|\]$/g, '').replace(/\.$/, '')
  if (!h.includes('.') && !h.includes(':')) return true
  if (/\.(localhost|local|lan|home|test|internal|intranet|corp|home\.arpa|example|invalid)$/.test(h)) return true
  const v4 = h.match(/^(\d+)\.(\d+)\.(\d+)\.(\d+)$/)
  if (v4) {
    const [a, b] = [Number(v4[1]), Number(v4[2])]
    return a === 127 || a === 10 || a === 0 || (a === 169 && b === 254) || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168) || (a === 100 && b >= 64 && b <= 127)
  }
  if (h.includes(':')) return h === '::1' || /^f[cd][0-9a-f]{2}:/.test(h) || /^fe[89ab][0-9a-f]:/.test(h)
  return false
}

export function hostOf(url: string): string {
  try {
    return new URL(url).hostname
  } catch {
    return ''
  }
}

export function originOf(url: string): string {
  try {
    const u = new URL(url)
    return u.origin === 'null' ? `${u.protocol}//${u.host}` : u.origin
  } catch {
    return url
  }
}

/** Host without a leading "www." for display. */
export function displayHost(url: string): string {
  return hostOf(url).replace(/^www\./, '')
}

/**
 * Splits a URL into the part to emphasise (the host) and the rest, hiding the
 * scheme for secure pages and trailing slashes.
 */
export function displayParts(url: string): { prefix: string; host: string; rest: string } {
  try {
    const u = new URL(url)
    if (u.protocol !== 'http:' && u.protocol !== 'https:') return { prefix: '', host: url, rest: '' }
    const prefix = u.protocol === 'http:' ? 'http://' : ''
    let rest = u.pathname + u.search + u.hash
    if (rest === '/') rest = ''
    return { prefix, host: u.host.replace(/^www\./, ''), rest: decodeSafe(rest) }
  } catch {
    return { prefix: '', host: url, rest: '' }
  }
}

/** A data URL that actually carries an image (pages often declare an empty `data:,` icon). */
export function isImageDataURL(value: string): boolean {
  return /^data:image\/[\w.+-]+(;[\w=-]+)*[;,]/.test(value) && value.length > 40
}

function decodeSafe(value: string): string {
  try {
    return decodeURI(value)
  } catch {
    return value
  }
}

/**
 * Splits a data: URL into its type and bytes. Handles parameters before the data
 * (data:image/svg+xml;charset=utf-8;base64,…) and URL-encoded text.
 */
export function parseDataURL(url: string): { type: string; body: Uint8Array } | null {
  const match = /^data:([^;,]+)((?:;[^;,]*)*?)(;base64)?,(.*)$/s.exec(url)
  if (!match) return null
  try {
    const body = match[3] ? Uint8Array.from(atob(match[4]), (c) => c.charCodeAt(0)) : new TextEncoder().encode(decodeURIComponent(match[4]))
    return { type: match[1].toLowerCase(), body }
  } catch {
    return null
  }
}
