// HTTPS-Only mode: top-level loads over plain HTTP are tried over HTTPS first. When
// the secure version doesn't answer, fails, or redirects back to HTTP, the tab shows
// a warning and the person decides whether to continue unencrypted for this session.
import { hostOf, isLocalHost } from '@shared/url'
import { tabForContents } from './registry'
import { settings } from './settings'
import { siteOf } from './shield'

/** How long an upgraded page gets to start answering before we offer HTTP. */
const ANSWER_MS = 8000

interface Attempt {
  /** HTTP addresses upgraded since the last page was shown, to spot redirect loops. */
  upgraded: Map<string, string>
  answered: boolean
  timer: NodeJS.Timeout | null
}

const attempts = new Map<number, Attempt>()
/**
 * Sites the person chose to open over HTTP, with their subdomains (HTTP-only sites
 * often send you on to one); forgotten when the browser closes.
 */
const allowed = new Set<string>()

export function allowHttp(url: string): void {
  const site = siteOf(url)
  if (site) allowed.add(site)
}

function upgradable(url: string): boolean {
  if (!url.startsWith('http:') || !settings().httpsOnly) return false
  const host = hostOf(url)
  return !!host && !isLocalHost(host) && !allowed.has(siteOf(url) ?? host)
}

function toHttps(url: string): string {
  const u = new URL(url)
  u.protocol = 'https:'
  // An explicit :80 belongs to HTTP; HTTPS on that port would never answer.
  if (u.port === '80') u.port = ''
  return u.toString()
}

function clear(id: number): void {
  const attempt = attempts.get(id)
  if (attempt?.timer) clearTimeout(attempt.timer)
  attempts.delete(id)
}

/**
 * For a top-level request: the HTTPS address to load instead, 'warn' when the
 * secure version sent us back to HTTP, or null to leave the request alone.
 */
export function httpsUpgrade(contentsId: number | undefined, url: string): string | 'warn' | null {
  if (contentsId === undefined || !upgradable(url)) return null
  let attempt = attempts.get(contentsId)
  if (attempt?.upgraded.has(url)) {
    clear(contentsId)
    return 'warn'
  }
  if (!attempt) {
    attempt = { upgraded: new Map(), answered: false, timer: null }
    attempts.set(contentsId, attempt)
  }
  const secure = toHttps(url)
  attempt.upgraded.set(url, secure)
  attempt.answered = false
  if (attempt.timer) clearTimeout(attempt.timer)
  attempt.timer = setTimeout(() => {
    const current = attempts.get(contentsId)
    if (!current || current.answered) return
    clear(contentsId)
    tabForContents(contentsId)?.httpsOnlyFailed(url)
  }, ANSWER_MS)
  return secure
}

/** The secure version sent response headers; slow pages are no reason to warn. */
export function httpsAnswered(contentsId: number | undefined): void {
  const attempt = contentsId === undefined ? undefined : attempts.get(contentsId)
  if (!attempt) return
  attempt.answered = true
  if (attempt.timer) clearTimeout(attempt.timer)
  attempt.timer = null
}

/** A page was shown; whatever happens next is a new attempt. */
export function httpsSettled(contentsId: number): void {
  clear(contentsId)
}

/**
 * When a load we upgraded fails, the HTTP address it came from. Failures to find the
 * host at all would fail over HTTP too, so they get the normal error page.
 */
export function httpsFallback(contentsId: number, failedURL: string, code: number): string | null {
  const attempt = attempts.get(contentsId)
  if (!attempt || [-105, -137, -106, -21].includes(code)) return null
  for (const [http, secure] of attempt.upgraded) {
    if (secure === failedURL || hostOf(secure) === hostOf(failedURL)) {
      clear(contentsId)
      return http
    }
  }
  return null
}
