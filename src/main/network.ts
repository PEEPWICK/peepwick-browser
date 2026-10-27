// One request pipeline per session. Electron keeps a single listener per webRequest
// event, so the focus gate, the shield, third-party cookie blocking and Global
// Privacy Control all run from here, in that order.
import type { Session } from 'electron'
import { shouldGate } from './focus'
import { httpsAnswered, httpsUpgrade } from './httpsonly'
import { threatFor } from './safebrowsing'
import { tabForContents } from './registry'
import { settings } from './settings'
import { cspFor, isThirdParty, matchRequest, shieldActiveFor } from './shield'
import { addBlocked } from './stats'
import type { Tab } from './tab'

const FILTER = { urls: ['http://*/*', 'https://*/*', 'ws://*/*', 'wss://*/*'] }

type Details = { frame?: Electron.WebFrameMain | null; webContentsId?: number }

function topURL(details: Details, tab: Tab | undefined): string | null {
  try {
    const top = details.frame?.top
    if (top?.url) return top.url
  } catch {
    // The frame went away while the request was in flight.
  }
  return tab?.pageURL() ?? null
}

function deleteHeader(headers: Record<string, unknown>, name: string): boolean {
  let removed = false
  for (const key of Object.keys(headers)) {
    if (key.toLowerCase() === name) {
      delete headers[key]
      removed = true
    }
  }
  return removed
}

export function installNetwork(session: Session, kind: 'browsing' | 'incognito' | 'reader'): void {
  session.webRequest.onBeforeRequest(FILTER, (details, callback) => {
    const tab = tabForContents(details.webContentsId)
    if (details.resourceType === 'mainFrame') {
      if (kind !== 'reader' && tab && shouldGate(details.url)) {
        tab.holdForFocus(details.url)
        callback({ cancel: true })
        return
      }
      if (kind !== 'reader' && tab) {
        const threat = threatFor(details.url)
        if (threat) {
          tab.showDangerous(details.url, threat)
          callback({ cancel: true })
          return
        }
        const upgrade = httpsUpgrade(details.webContentsId, details.url)
        if (upgrade === 'warn') {
          tab.httpsOnlyFailed(details.url)
          callback({ cancel: true })
          return
        }
        if (upgrade) {
          callback({ redirectURL: upgrade })
          return
        }
      }
      callback({})
      return
    }
    // Reader view shows images from the article; block trackers there whenever the shield is on.
    const active = kind === 'reader' ? settings().shieldLevel !== 'off' : shieldActiveFor(topURL(details, tab))
    if (!active) {
      callback({})
      return
    }
    const verdict = matchRequest(details)
    if (!verdict.block) {
      callback({})
      return
    }
    tab?.recordBlocked(details.url)
    addBlocked()
    callback(verdict.redirect ? { redirectURL: verdict.redirect } : { cancel: true })
  })

  session.webRequest.onBeforeSendHeaders(FILTER, (details, callback) => {
    const prefs = settings()
    const headers = details.requestHeaders
    let changed = false
    if (prefs.sendGlobalPrivacyControl) {
      headers['Sec-GPC'] = '1'
      changed = true
    }
    if (prefs.blockThirdPartyCookies && details.resourceType !== 'mainFrame') {
      const top = topURL(details, tabForContents(details.webContentsId))
      if (top && shieldActiveFor(top) && isThirdParty(details.url, top)) {
        changed = deleteHeader(headers, 'cookie') || changed
      }
    }
    callback(changed ? { requestHeaders: headers } : {})
  })

  session.webRequest.onHeadersReceived(FILTER, (details, callback) => {
    if (details.resourceType === 'mainFrame') httpsAnswered(details.webContentsId)
    const headers = details.responseHeaders
    const isDocument = details.resourceType === 'mainFrame' || details.resourceType === 'subFrame'
    const top = details.resourceType === 'mainFrame' ? details.url : topURL(details, tabForContents(details.webContentsId))
    if (!headers || !shieldActiveFor(top)) {
      callback({})
      return
    }
    let changed = false
    if (isDocument) {
      const csp = cspFor(details)
      if (csp) {
        // An extra policy header is enforced alongside the site's own policies.
        const key = Object.keys(headers).find((k) => k.toLowerCase() === 'content-security-policy')
        if (key) headers[key] = [...headers[key], csp]
        else headers['Content-Security-Policy'] = [csp]
        changed = true
      }
    }
    if (
      settings().blockThirdPartyCookies &&
      details.resourceType !== 'mainFrame' &&
      top &&
      isThirdParty(details.url, top)
    ) {
      changed = deleteHeader(headers, 'set-cookie') || changed
    }
    callback(changed ? { responseHeaders: headers } : {})
  })
}
