// Dependency-free URL helpers used by both the main process and the UI.
import type { InternalPage, TabSecurity } from './types'

export const INTERNAL_SCHEME = 'peepwick'
export const NEW_TAB_URL = 'peepwick://newtab'

const INTERNAL_PAGES: InternalPage[] = ['newtab', 'settings', 'history', 'extensions']

export function internalPage(url: string): InternalPage | null {
  const match = /^peepwick:\/\/([a-z]+)\/?/i.exec(url.trim())
  if (!match) return null
  const page = match[1].toLowerCase() as InternalPage
  return INTERNAL_PAGES.includes(page) ? page : null
}

export function internalURL(page: InternalPage, hash = ''): string {
  return `peepwick://${page}${hash ? `#${hash}` : ''}`
}

export function securityOf(url: string): TabSecurity {
  if (url.startsWith('https:')) return 'secure'
  if (url.startsWith('http:')) {
    const host = hostOf(url)
    return host === 'localhost' || host === '127.0.0.1' || host === '[::1]' ? 'local' : 'insecure'
  }
  if (url.startsWith('file:')) return 'local'
  return 'internal'
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
