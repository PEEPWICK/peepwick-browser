// Clearing browsing data: from the dialog (with a time range) and, if the person
// asks for it, every time the browser closes. Cookies, site storage and the cache
// can only be cleared for all time: Electron offers no time range for them.
import type { ClearCounts, ClearKind, ClearRange, ClearRequest } from '@shared/types'
import { CLEAR_KINDS } from '@shared/types'
import { links } from '@shared/bookmarks'
import { hostOf } from '@shared/url'
import { bookmarkTree } from './bookmarks'
import { clearDownloads, countDownloads } from './downloads'
import { clearFormEntries, countFormEntries } from './formdata'
import { clearPasswords, passwordStatus } from './passwords'
import { countVisits, deleteVisitsSince } from './history'
import { clearPermissions, listPermissions } from './permissions'
import { browsingSession } from './sessions'
import { settings } from './settings'

const RANGES: ClearRange[] = [1, 24, 168, 672, 0]

export function sinceFor(range: ClearRange): number {
  return range > 0 ? Date.now() - range * 3_600_000 : 0
}

/** Only well-formed requests from the UI. */
export function sanitizeClearRequest(input: unknown): ClearRequest {
  const value = (input ?? {}) as { range?: unknown; kinds?: unknown }
  const range = RANGES.includes(value.range as ClearRange) ? (value.range as ClearRange) : 0
  const kinds = Array.isArray(value.kinds) ? CLEAR_KINDS.filter((kind) => (value.kinds as unknown[]).includes(kind)) : []
  return { range, kinds }
}

export async function clearCounts(range: ClearRange): Promise<ClearCounts> {
  const since = sinceFor(range)
  let cookies = 0
  try {
    cookies = (await browsingSession().cookies.get({})).length
  } catch {
    cookies = 0
  }
  return {
    history: countVisits(since),
    downloads: countDownloads(since),
    cookies,
    permissions: listPermissions().length,
    forms: countFormEntries(since),
    passwords: passwordStatus().count,
  }
}

export async function clearBrowsingData({ range, kinds }: ClearRequest): Promise<void> {
  const since = sinceFor(range)
  const has = (kind: ClearKind) => kinds.includes(kind)
  const session = browsingSession()
  // Bookmarked sites keep their icons when the history goes.
  if (has('history')) deleteVisitsSince(since, new Set(links(bookmarkTree()).map((link) => hostOf(link.url))))
  if (has('downloads')) clearDownloads(since)
  if (has('permissions')) clearPermissions()
  if (has('forms')) clearFormEntries(since)
  if (has('passwords')) clearPasswords()
  if (has('cookies')) await session.clearStorageData()
  if (has('cache')) await session.clearCache()
}

/** Runs the "clear when the browser closes" choices, giving up after a few seconds. */
export async function clearOnExit(): Promise<void> {
  // Passwords are never wiped on exit, whatever a hand-edited settings file says.
  const kinds = settings().clearOnExit.filter((kind) => kind !== 'passwords')
  if (kinds.length === 0) return
  await Promise.race([clearBrowsingData({ range: 0, kinds }).catch(() => {}), new Promise((resolve) => setTimeout(resolve, 4000))])
}
