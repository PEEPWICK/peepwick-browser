// Turning what the user typed into a URL. Shared so it can be unit-tested; used by
// the main process only (it pulls in the public suffix list via tldts).
import { parse } from 'tldts'
import { searchURL } from './search'
import type { SearchEngineId } from './types'
import { internalPage } from './url'

const ALLOWED_SCHEMES = /^(https?|file|peepwick|chrome-extension|about|view-source|data):/i

/** Turns what the user typed into a URL to load (or a search URL). */
export function resolveInput(raw: string, engine: SearchEngineId): string {
  const input = raw.trim()
  if (!input) return ''
  if (internalPage(input)) return input
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(input) || /^(about|data|view-source):/i.test(input)) {
    return ALLOWED_SCHEMES.test(input) ? input : searchURL(engine, input)
  }
  if (/\s/.test(input)) return searchURL(engine, input)
  if (looksLikeHost(input)) {
    const host = input.split(/[/?#]/)[0].replace(/:\d+$/, '')
    const local = host === 'localhost' || isIP(host) || host.endsWith('.local') || host.endsWith('.test')
    return `${local ? 'http' : 'https'}://${input}`
  }
  return searchURL(engine, input)
}

function isIP(host: string): boolean {
  return /^(\d{1,3}\.){3}\d{1,3}$/.test(host) || /^\[[0-9a-f:]+\]$/i.test(host)
}

export function looksLikeHost(input: string): boolean {
  const hostPart = input.split(/[/?#]/)[0]
  const host = hostPart.replace(/:\d{1,5}$/, '')
  if (!host || host.startsWith('.') || host.endsWith('.')) return false
  if (host === 'localhost' || isIP(host)) return true
  if (!/^[\p{L}\p{N}.-]+$/u.test(host)) return false
  const parsed = parse(host, { allowPrivateDomains: true })
  if (parsed.isIcann || parsed.isPrivate) return !!parsed.domain
  return host.endsWith('.local') || host.endsWith('.test') || host.endsWith('.internal')
}
