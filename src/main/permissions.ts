// Site permissions. Sensitive requests become a prompt bar above the page; answers can
// be remembered per origin (in memory only for private windows). During a focus
// session, notification requests are quietly declined.
import { desktopCapturer, type Session } from 'electron'
import { originOf } from '@shared/url'
import type { PermissionKind, PermissionPrompt, SitePermission } from '@shared/types'
import { silencingNotifications } from './focus'
import { userPath } from './paths'
import { tabForContents } from './registry'
import { JsonFile } from './store'
import type { BrowserWin } from './window'

type Decisions = Record<string, Partial<Record<PermissionKind, boolean>>>

const file = new JsonFile<Decisions>(userPath('permissions.json'), 500)
let saved: Decisions = {}
const incognitoDecisions: Decisions = {}

interface Pending {
  prompt: PermissionPrompt
  incognito: boolean
  resolve: (allow: boolean) => void
  /** Redraws the window so the prompt bar appears or goes away. */
  notify: () => void
}

const pending: Pending[] = []
let nextId = 1

const ALWAYS_ALLOWED = new Set([
  'fullscreen',
  'pointerLock',
  'keyboardLock',
  'clipboard-sanitized-write',
  'speaker-selection',
  'fileSystem',
])

export function loadPermissions(): void {
  saved = file.read() ?? {}
}

function store(incognito: boolean): Decisions {
  return incognito ? incognitoDecisions : saved
}

function kindsFor(permission: string, details: Record<string, unknown>): PermissionKind[] | null {
  switch (permission) {
    case 'media': {
      const types = (details.mediaTypes as string[] | undefined) ?? []
      const kinds: PermissionKind[] = []
      if (types.includes('video')) kinds.push('camera')
      if (types.includes('audio')) kinds.push('microphone')
      return kinds.length > 0 ? kinds : ['camera', 'microphone']
    }
    case 'geolocation':
      return ['geolocation']
    case 'notifications':
      return ['notifications']
    case 'midi':
    case 'midiSysex':
      return ['midi']
    case 'clipboard-read':
      return ['clipboard']
    case 'display-capture':
      return ['screen']
    case 'openExternal':
      return ['external']
    default:
      return null
  }
}

function checkKind(permission: string, details: Record<string, unknown>): PermissionKind | null {
  if (permission === 'media') {
    const type = details.mediaType
    return type === 'video' ? 'camera' : type === 'audio' ? 'microphone' : null
  }
  const kinds = kindsFor(permission, details)
  return kinds?.[0] ?? null
}

export function installPermissions(session: Session, incognito: boolean): void {
  session.setPermissionRequestHandler((wc, permission, callback, details) => {
    if (ALWAYS_ALLOWED.has(permission)) return callback(true)
    const info = details as unknown as Record<string, unknown>
    const kinds = kindsFor(permission, info)
    if (!kinds) return callback(false)
    if (kinds.includes('notifications') && silencingNotifications()) return callback(false)

    const origin = originOf((info.requestingUrl as string) || wc.getURL())
    const decisions = store(incognito)[origin] ?? {}
    if (kinds.some((kind) => decisions[kind] === false)) return callback(false)
    if (kinds.every((kind) => decisions[kind] === true)) return callback(true)

    const tab = tabForContents(wc.id)
    if (!tab) return callback(false)
    const prompt: PermissionPrompt = { id: nextId++, tabId: tab.id, origin, kinds }
    pending.push({ prompt, incognito, resolve: callback, notify: () => tab.win.markDirty() })
    tab.win.markDirty()
  })

  session.setPermissionCheckHandler((wc, permission, requestingOrigin, details) => {
    if (ALWAYS_ALLOWED.has(permission)) return true
    const kind = checkKind(permission, details as unknown as Record<string, unknown>)
    if (!kind) return false
    if (kind === 'notifications' && silencingNotifications()) return false
    const origin = originOf(requestingOrigin || wc?.getURL() || '')
    return store(incognito)[origin]?.[kind] === true
  })

  // Screen sharing goes through the desktop portal on Wayland, which shows its own picker.
  session.setDisplayMediaRequestHandler(
    async (request, callback) => {
      const origin = originOf(request.securityOrigin || request.frame?.url || '')
      if (store(incognito)[origin]?.screen === false) return callback({})
      try {
        const sources = await desktopCapturer.getSources({ types: ['screen', 'window'] })
        if (sources.length === 0) return callback({})
        callback({ video: sources[0], audio: request.audioRequested ? 'loopback' : undefined })
      } catch {
        callback({})
      }
    },
    { useSystemPicker: true },
  )
}

/** The prompt to show for the window's active tab, if any. */
export function pendingPromptFor(win: BrowserWin): PermissionPrompt | null {
  const active = win.activeTab()
  if (!active) return null
  return pending.find((p) => p.prompt.tabId === active.id)?.prompt ?? null
}

export function respond(id: number, allow: boolean, remember: boolean): void {
  const index = pending.findIndex((p) => p.prompt.id === id)
  if (index < 0) return
  const [entry] = pending.splice(index, 1)
  if (remember) {
    const decisions = store(entry.incognito)
    const forOrigin = (decisions[entry.prompt.origin] ??= {})
    for (const kind of entry.prompt.kinds) forOrigin[kind] = allow
    if (!entry.incognito) file.schedule(saved)
  }
  entry.resolve(allow)
  entry.notify()
}

export function denyPendingFor(tabId: number): void {
  for (let i = pending.length - 1; i >= 0; i--) {
    if (pending[i].prompt.tabId === tabId) {
      const [entry] = pending.splice(i, 1)
      entry.resolve(false)
    }
  }
}

export function listPermissions(): SitePermission[] {
  const out: SitePermission[] = []
  for (const [origin, kinds] of Object.entries(saved)) {
    for (const [kind, allow] of Object.entries(kinds)) {
      if (typeof allow === 'boolean') out.push({ origin, kind: kind as PermissionKind, allow })
    }
  }
  return out.sort((a, b) => a.origin.localeCompare(b.origin))
}

export function removePermission(origin: string, kind: PermissionKind): void {
  const entry = saved[origin]
  if (!entry) return
  delete entry[kind]
  if (Object.keys(entry).length === 0) delete saved[origin]
  file.schedule(saved)
}

/** Sets one remembered answer for a site; null forgets it, so the site asks again. */
export function setPermission(origin: string, kind: PermissionKind, allow: boolean | null): void {
  const entry = (saved[origin] ??= {})
  if (allow === null) delete entry[kind]
  else entry[kind] = allow
  if (Object.keys(entry).length === 0) delete saved[origin]
  file.schedule(saved)
}

export function exportPermissions(): Decisions {
  return JSON.parse(JSON.stringify(saved)) as Decisions
}

const KINDS: PermissionKind[] = ['camera', 'microphone', 'geolocation', 'notifications', 'midi', 'clipboard', 'screen', 'external', 'other']

/** Restoring a backup: the backup's answers win for the sites it has. */
export function mergePermissions(incoming: unknown): number {
  if (!incoming || typeof incoming !== 'object') return 0
  let count = 0
  for (const [origin, kinds] of Object.entries(incoming as Record<string, unknown>)) {
    if (!/^https?:\/\/[^/\s]+$/.test(origin) || !kinds || typeof kinds !== 'object') continue
    for (const [kind, allow] of Object.entries(kinds as Record<string, unknown>)) {
      if (!KINDS.includes(kind as PermissionKind) || typeof allow !== 'boolean') continue
      ;(saved[origin] ??= {})[kind as PermissionKind] = allow
      count++
    }
  }
  file.schedule(saved)
  return count
}

export function clearPermissions(): void {
  saved = {}
  file.schedule(saved)
}

export function flushPermissions(): void {
  file.flush()
}
