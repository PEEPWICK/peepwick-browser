// Three kinds of session: the UI (default session, never touches the web), browsing
// (persistent, where extensions live) and private windows (in memory, shared by all
// private windows, wiped when the last one closes).
import { app, protocol, session, type Session } from 'electron'
import { readFile } from 'node:fs/promises'
import { extname, normalize, sep } from 'node:path'
import { installDownloads } from './downloads'
import { getFavicon } from './history'
import { installNetwork } from './network'
import { devServerURL, rendererPath } from './paths'
import { installPermissions } from './permissions'
import { enableCosmetics } from './shield'

export const BROWSING_PARTITION = 'persist:peepwick'
const INCOGNITO_PARTITION = 'peepwick-incognito'

let browsing: Session | null = null
let incognito: Session | null = null

export function registerSchemes(): void {
  protocol.registerSchemesAsPrivileged([
    {
      scheme: 'peepwick',
      privileges: { standard: true, secure: true, supportFetchAPI: true, codeCache: true, allowServiceWorkers: false },
    },
  ])
}

/** A Chrome-like user agent without the Electron and app tokens, so sites treat us as Chrome. */
export function setUserAgent(): void {
  const chrome = process.versions.chrome.split('.')[0]
  app.userAgentFallback = app.userAgentFallback
    .replace(/\sElectron\/\S+/, '')
    .replace(/\s(peepwick-browser|PEEPWICK Browser)\/\S+/i, '')
    .replace(/Chrome\/[\d.]+/, `Chrome/${chrome}.0.0.0`)
}

export function uiSession(): Session {
  return session.defaultSession
}

const MIME: Record<string, string> = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.woff2': 'font/woff2',
  '.woff': 'font/woff',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.json': 'application/json',
}

const UI_CSP = [
  "default-src 'self'",
  "script-src 'self'",
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data: crx: peepwick:",
  "font-src 'self'",
  "connect-src 'self'",
  "object-src 'none'",
  "base-uri 'none'",
  "frame-src 'none'",
  "form-action 'none'",
].join('; ')

/** peepwick://favicon/<host>: the stored icon for a site, so lists can load icons lazily. */
function faviconResponse(host: string): Response {
  const data = getFavicon(host)
  const match = data ? /^data:([^;,]+)(;base64)?,(.*)$/s.exec(data) : null
  if (!match) return new Response('Not found', { status: 404 })
  const body = match[2] ? Buffer.from(match[3], 'base64') : Buffer.from(decodeURIComponent(match[3]))
  return new Response(body, { headers: { 'content-type': match[1], 'cache-control': 'max-age=3600' } })
}

export function configureUISession(): void {
  const ses = uiSession()
  ses.setPermissionRequestHandler((_wc, _permission, callback) => callback(false))
  ses.setPermissionCheckHandler(() => false)
  const dev = !!devServerURL()
  const root = rendererPath()
  ses.protocol.handle('peepwick', async (request) => {
    const url = new URL(request.url)
    if (url.host === 'favicon') return faviconResponse(decodeURIComponent(url.pathname.slice(1)).toLowerCase())
    if (url.host !== 'ui' || dev) return new Response('Not found', { status: 404 })
    const relative = decodeURIComponent(url.pathname === '/' ? '/index.html' : url.pathname)
    const file = normalize(root + relative)
    if (!file.startsWith(root + sep)) return new Response('Not found', { status: 404 })
    try {
      const body = await readFile(file)
      const headers: Record<string, string> = { 'content-type': MIME[extname(file)] ?? 'application/octet-stream' }
      if (file.endsWith('.html')) headers['content-security-policy'] = UI_CSP
      return new Response(body, { headers })
    } catch {
      return new Response('Not found', { status: 404 })
    }
  })
}

function configureBrowsingSession(ses: Session, kind: 'browsing' | 'incognito'): void {
  installNetwork(ses, kind)
  enableCosmetics(ses)
  installPermissions(ses, kind === 'incognito')
  installDownloads(ses, kind === 'incognito')
  const wanted = ['tr', 'en-US']
  const languages = wanted.filter((lang) => ses.availableSpellCheckerLanguages.includes(lang))
  if (languages.length > 0) ses.setSpellCheckerLanguages(languages)
}

export function browsingSession(): Session {
  if (!browsing) {
    browsing = session.fromPartition(BROWSING_PARTITION)
    configureBrowsingSession(browsing, 'browsing')
  }
  return browsing
}

export function incognitoSession(): Session {
  if (!incognito) {
    incognito = session.fromPartition(INCOGNITO_PARTITION)
    configureBrowsingSession(incognito, 'incognito')
  }
  return incognito
}

/** Called when the last private window closes. */
export async function wipeIncognito(): Promise<void> {
  if (!incognito) return
  await incognito.clearStorageData()
  await incognito.clearCache()
  await incognito.clearAuthCache()
}
