import { SEARCH_ENGINES } from './search'
import { CLEAR_KINDS, type BookmarksBarMode, type ClearKind, type Lang, type SearchEngineId, type SecureDns, type Settings, type ShieldLevel, type ThemePref } from './types'

export const DEFAULT_FOCUS_SITES = [
  'youtube.com',
  'x.com',
  'twitter.com',
  'instagram.com',
  'tiktok.com',
  'facebook.com',
  'reddit.com',
  'twitch.tv',
  'netflix.com',
]

export function defaultSettings(language: Lang): Settings {
  return {
    language,
    theme: 'dark',
    searchEngine: 'duckduckgo',
    searchSuggestions: false,
    shieldLevel: 'standard',
    blockThirdPartyCookies: true,
    sendGlobalPrivacyControl: true,
    shieldAllowlist: [],
    focusSites: [...DEFAULT_FOCUS_SITES],
    focusSilenceNotifications: true,
    focusMinutes: 50,
    sleepAfterMinutes: 60,
    restoreSession: true,
    askWhereToSave: false,
    sidebarCollapsed: false,
    bookmarksBar: 'always',
    onboarded: false,
    errorLog: false,
    clearOnExit: [],
    secureDns: 'automatic',
    downloadDir: '',
    hardwareAcceleration: true,
  }
}

/** DNS over HTTPS servers for each provider choice. */
export const SECURE_DNS_SERVERS: Record<Exclude<SecureDns, 'off' | 'automatic'>, { name: string; template: string }> = {
  cloudflare: { name: 'Cloudflare', template: 'https://cloudflare-dns.com/dns-query' },
  quad9: { name: 'Quad9', template: 'https://dns.quad9.net/dns-query' },
  mullvad: { name: 'Mullvad', template: 'https://dns.mullvad.net/dns-query' },
  google: { name: 'Google', template: 'https://dns.google/dns-query{?dns}' },
}

const LANGS: Lang[] = ['tr', 'en']
const THEMES: ThemePref[] = ['system', 'dark', 'light']
const LEVELS: ShieldLevel[] = ['off', 'standard', 'strict']
const BAR_MODES: BookmarksBarMode[] = ['always', 'newtab', 'never']
const DNS: SecureDns[] = ['off', 'automatic', 'cloudflare', 'quad9', 'mullvad', 'google']

/** Keeps only well-formed values from an untrusted settings patch. */
export function sanitizeSettingsPatch(patch: unknown): Partial<Settings> {
  if (!patch || typeof patch !== 'object') return {}
  const input = patch as Record<string, unknown>
  const out: Partial<Settings> = {}
  const bool = (key: keyof Settings) => {
    if (typeof input[key] === 'boolean') (out as Record<string, unknown>)[key] = input[key]
  }
  if (LANGS.includes(input.language as Lang)) out.language = input.language as Lang
  if (THEMES.includes(input.theme as ThemePref)) out.theme = input.theme as ThemePref
  if (typeof input.searchEngine === 'string' && Object.hasOwn(SEARCH_ENGINES, input.searchEngine)) {
    out.searchEngine = input.searchEngine as SearchEngineId
  }
  if (LEVELS.includes(input.shieldLevel as ShieldLevel)) out.shieldLevel = input.shieldLevel as ShieldLevel
  if (BAR_MODES.includes(input.bookmarksBar as BookmarksBarMode)) out.bookmarksBar = input.bookmarksBar as BookmarksBarMode
  bool('searchSuggestions')
  bool('blockThirdPartyCookies')
  bool('sendGlobalPrivacyControl')
  bool('focusSilenceNotifications')
  bool('restoreSession')
  bool('askWhereToSave')
  bool('sidebarCollapsed')
  bool('onboarded')
  bool('errorLog')
  bool('hardwareAcceleration')
  if (DNS.includes(input.secureDns as SecureDns)) out.secureDns = input.secureDns as SecureDns
  if (Array.isArray(input.clearOnExit)) {
    out.clearOnExit = CLEAR_KINDS.filter((kind) => (input.clearOnExit as unknown[]).includes(kind)) as ClearKind[]
  }
  // An absolute folder path, or empty for the system default.
  if (typeof input.downloadDir === 'string' && (input.downloadDir === '' || (input.downloadDir.startsWith('/') && input.downloadDir.length < 4096))) {
    out.downloadDir = input.downloadDir
  }
  if (Array.isArray(input.shieldAllowlist)) out.shieldAllowlist = cleanDomains(input.shieldAllowlist)
  if (Array.isArray(input.focusSites)) out.focusSites = cleanDomains(input.focusSites)
  if (typeof input.focusMinutes === 'number' && Number.isFinite(input.focusMinutes)) {
    out.focusMinutes = Math.min(240, Math.max(5, Math.round(input.focusMinutes)))
  }
  if (typeof input.sleepAfterMinutes === 'number' && Number.isFinite(input.sleepAfterMinutes)) {
    out.sleepAfterMinutes = Math.min(24 * 60, Math.max(0, Math.round(input.sleepAfterMinutes)))
  }
  return out
}

/** Normalises user-entered sites ("https://www.YouTube.com/watch" → "youtube.com"). */
export function normalizeDomain(value: string): string | null {
  let text = value.trim().toLowerCase()
  if (!text) return null
  if (!/^[a-z][a-z0-9+.-]*:\/\//.test(text)) text = `https://${text}`
  try {
    const host = new URL(text).hostname.replace(/^www\./, '').replace(/\.$/, '')
    return /^[a-z0-9-]+(\.[a-z0-9-]+)+$/.test(host) || host === 'localhost' ? host : null
  } catch {
    return null
  }
}

function cleanDomains(values: unknown[]): string[] {
  const out = new Set<string>()
  for (const value of values) {
    if (typeof value !== 'string') continue
    const domain = normalizeDomain(value)
    if (domain) out.add(domain)
    if (out.size >= 500) break
  }
  return [...out]
}

/** True when `host` is `site` or one of its subdomains. */
export function hostMatches(host: string, site: string): boolean {
  const h = host.toLowerCase().replace(/\.$/, '')
  return h === site || h.endsWith(`.${site}`)
}
