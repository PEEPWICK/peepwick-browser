// The privacy shield: Ghostery's filter engine (EasyList, EasyPrivacy and friends)
// driven by our own request pipeline instead of ElectronBlocker's session hooks,
// because Electron allows only one webRequest listener per event and session.
import { ipcMain, net, type IpcMainInvokeEvent, type Session } from 'electron'
import { readFile, stat, writeFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import { ElectronBlocker, fromElectronDetails } from '@ghostery/adblocker-electron'
import { getDomain } from 'tldts'
import type { ShieldLevel } from '@shared/types'
import { userPath } from './paths'
import { settings } from './settings'

const REFRESH_AFTER = 3 * 24 * 60 * 60 * 1000
const require = createRequire(import.meta.url)
const COSMETIC_PRELOAD = require.resolve('@ghostery/adblocker-electron-preload')

let engine: ElectronBlocker | null = null
let engineLevel: ShieldLevel | null = null
let loading: Promise<void> | null = null

function cachePath(level: ShieldLevel): string {
  return userPath(`shield-${level}.bin`)
}

const fetchImpl = (url: string) => net.fetch(url, { bypassCustomProtocolHandlers: true })

async function build(level: ShieldLevel): Promise<ElectronBlocker> {
  return level === 'strict'
    ? ElectronBlocker.fromPrebuiltFull(fetchImpl)
    : ElectronBlocker.fromPrebuiltAdsAndTracking(fetchImpl)
}

async function loadCached(level: ShieldLevel): Promise<ElectronBlocker | null> {
  try {
    return ElectronBlocker.deserialize(new Uint8Array(await readFile(cachePath(level))))
  } catch {
    return null
  }
}

async function refresh(level: ShieldLevel): Promise<void> {
  try {
    const fresh = await build(level)
    await writeFile(cachePath(level), fresh.serialize())
    if (settings().shieldLevel === level) {
      engine = fresh
      engineLevel = level
    }
  } catch (error) {
    console.warn('[shield] could not refresh filter lists', error)
  }
}

/** Loads the engine for the current level: from cache first, refreshing stale lists in the background. */
export function ensureShield(): Promise<void> {
  const level = settings().shieldLevel
  if (level === 'off') {
    engine = null
    engineLevel = null
    return Promise.resolve()
  }
  if (engineLevel === level && engine) return Promise.resolve()
  loading = (async () => {
    const cached = await loadCached(level)
    if (cached) {
      engine = cached
      engineLevel = level
      const age = await stat(cachePath(level)).then((s) => Date.now() - s.mtimeMs, () => Infinity)
      if (age > REFRESH_AFTER) void refresh(level)
    } else {
      await refresh(level)
    }
  })()
  return loading
}

export function siteOf(url: string): string | null {
  try {
    const host = new URL(url).hostname
    return getDomain(host, { allowPrivateDomains: true }) ?? host
  } catch {
    return null
  }
}

/** Whether the shield protects pages on `topURL` (on, and the site isn't allowlisted). */
export function shieldActiveFor(topURL: string | null | undefined): boolean {
  if (settings().shieldLevel === 'off' || !topURL || !/^https?:/.test(topURL)) return false
  const site = siteOf(topURL)
  return !site || !settings().shieldAllowlist.includes(site)
}

export function isThirdParty(requestURL: string, topURL: string): boolean {
  const a = siteOf(requestURL)
  const b = siteOf(topURL)
  return !!a && !!b && a !== b
}

export interface Verdict {
  block: boolean
  redirect?: string
}

export function matchRequest(details: Electron.OnBeforeRequestListenerDetails): Verdict {
  if (!engine) return { block: false }
  const request = fromElectronDetails(details)
  if (request.type === 'other') request.guessTypeOfRequest()
  const { match, redirect } = engine.match(request)
  if (redirect) return { block: true, redirect: redirect.dataUrl }
  return { block: !!match }
}

/** Extra CSP directives some filters inject into documents (e.g. to stop inline miners). */
export function cspFor(details: Electron.OnHeadersReceivedListenerDetails): string | undefined {
  if (!engine) return undefined
  return engine.getCSPDirectives(fromElectronDetails(details))
}

/** Cosmetic filtering (hiding ad slots, cookie notices) runs through a frame preload. */
export function enableCosmetics(session: Session): void {
  session.registerPreloadScript({ type: 'frame', filePath: COSMETIC_PRELOAD })
}

let cosmeticHandlersInstalled = false

export function installCosmeticHandlers(topURLFor: (event: IpcMainInvokeEvent) => string | null): void {
  if (cosmeticHandlersInstalled) return
  cosmeticHandlersInstalled = true
  ipcMain.handle('@ghostery/adblocker/inject-cosmetic-filters', (event, url: string, msg?: unknown) => {
    if (!engine || !shieldActiveFor(topURLFor(event) ?? url)) return
    return engine.onInjectCosmeticFilters(event, url, msg as never)
  })
  ipcMain.handle('@ghostery/adblocker/is-mutation-observer-enabled', (event) => {
    return engine ? engine.onIsMutationObserverEnabled(event) : false
  })
}
