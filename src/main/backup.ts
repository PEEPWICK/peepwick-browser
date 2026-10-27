// Encrypted local backup: settings, bookmarks, spaces (and optionally their tabs),
// focus history, site permissions, the extension list and optionally browsing
// history, in one password-protected file the person keeps wherever they like.
// No cloud: moving it to another computer is up to them. Restoring merges into
// what is already there and never deletes anything.
import { dialog } from 'electron'
import { randomUUID } from 'node:crypto'
import { readFile, writeFile } from 'node:fs/promises'
import { basename } from 'node:path'
import { installExtension } from 'electron-chrome-web-store'
import { isDraftTree, links, toDraft, type DraftTree } from '@shared/bookmarks'
import { sanitizeSettingsPatch } from '@shared/settings'
import {
  BACKUP_CATEGORIES,
  WORKSPACE_ICONS,
  type BackupCategory,
  type BackupCounts,
  type BackupSummary,
  type RestoreResult,
  type Settings,
  type WorkspaceIcon,
} from '@shared/types'
import { NEW_TAB_URL } from '@shared/url'
import { bookmarkTree, mergeBookmarks } from './bookmarks'
import { BACKUP_EXTENSION, BackupError, decryptBackup, encryptBackup, looksLikeBackup } from './backupfile'
import { listExtensions } from './extensions'
import { countVisits, exportVisits, importVisits, type VisitRow } from './history'
import { shownVersion } from './paths'
import { exportPermissions, listPermissions, mergePermissions } from './permissions'
import { browsingSession } from './sessions'
import { settings, t, updateSettings } from './settings'
import { exportStats, mergeStats, type StatsData } from './stats'
import { BrowserWin } from './window'

interface BackupWorkspace {
  name: string
  icon: WorkspaceIcon
  tabs?: { url: string; title: string; pinned: boolean }[]
}

interface Payload {
  format: 'peepwick-backup'
  version: 1
  app: string
  createdAt: number
  settings?: Partial<Settings>
  bookmarks?: DraftTree
  workspaces?: BackupWorkspace[]
  focus?: StatsData
  permissions?: unknown
  extensions?: { id: string; name: string }[]
  history?: VisitRow[]
}

/** Settings that belong to one computer and never travel. */
const LOCAL_SETTINGS: (keyof Settings)[] = ['onboarded', 'downloadDir', 'hardwareAcceleration']

function normalWindows(): BrowserWin[] {
  return [...BrowserWin.all.values()].filter((w) => !w.incognito && !w.base.isDestroyed())
}

function savedWorkspaces(withTabs: boolean): BackupWorkspace[] {
  return normalWindows().flatMap((win) =>
    win.serialize().workspaces.map((w) => ({
      name: w.name,
      icon: w.icon,
      ...(withTabs ? { tabs: w.tabs.filter((tab) => /^(https?|file):/.test(tab.url)).map((tab) => ({ url: tab.url, title: tab.title, pinned: tab.pinned })) } : {}),
    })),
  )
}

/** How much of each category this computer has, for the export dialog. */
export function localCounts(): BackupCounts {
  const workspaces = savedWorkspaces(true)
  const stats = exportStats()
  return {
    settings: 1,
    bookmarks: links(bookmarkTree()).length,
    workspaces: workspaces.length,
    tabs: workspaces.reduce((n, w) => n + (w.tabs?.length ?? 0), 0),
    focus: Object.values(stats.days).filter((d) => d.focus > 0 || d.blocked > 0).length,
    permissions: listPermissions().length,
    extensions: listExtensions().length,
    history: countVisits(0),
  }
}

export function collectBackup(categories: BackupCategory[]): Payload {
  const has = (c: BackupCategory) => categories.includes(c)
  const current = settings()
  return {
    format: 'peepwick-backup',
    version: 1,
    app: shownVersion(),
    createdAt: Date.now(),
    ...(has('settings') ? { settings: Object.fromEntries(Object.entries(current).filter(([key]) => !LOCAL_SETTINGS.includes(key as keyof Settings))) } : {}),
    ...(has('bookmarks') ? { bookmarks: toDraft(bookmarkTree()) } : {}),
    ...(has('workspaces') ? { workspaces: savedWorkspaces(has('tabs')) } : {}),
    ...(has('focus') ? { focus: exportStats() } : {}),
    ...(has('permissions') ? { permissions: exportPermissions() } : {}),
    ...(has('extensions') ? { extensions: listExtensions().map(({ id, name }) => ({ id, name })) } : {}),
    ...(has('history') ? { history: exportVisits() } : {}),
  }
}

function fileName(): string {
  const date = new Date().toISOString().slice(0, 10)
  return `${t('backup.fileName')}-${date}.${BACKUP_EXTENSION}`
}

export async function exportBackup(win: BrowserWin, categories: BackupCategory[], password: string): Promise<'saved' | 'cancelled'> {
  const payload = collectBackup(categories.filter((c) => BACKUP_CATEGORIES.includes(c)))
  const { canceled, filePath } = await dialog.showSaveDialog(win.base, {
    title: t('backup.export'),
    defaultPath: fileName(),
    filters: [{ name: t('backup.fileType'), extensions: [BACKUP_EXTENSION] }],
  })
  if (canceled || !filePath) return 'cancelled'
  const target = filePath.endsWith(`.${BACKUP_EXTENSION}`) ? filePath : `${filePath}.${BACKUP_EXTENSION}`
  await writeFile(target, await encryptBackup(payload, password), { mode: 0o600 })
  return 'saved'
}

// ── Restoring ─────────────────────────────────────────────────────────────────

/** Opened files wait here between "choose file", "password" and "restore", for ten minutes at most. */
const opened = new Map<string, { data: Buffer; name: string; payload?: Payload; timer: NodeJS.Timeout }>()

function forget(token: string): void {
  const entry = opened.get(token)
  if (!entry) return
  clearTimeout(entry.timer)
  opened.delete(token)
}

export async function openBackup(win: BrowserWin): Promise<{ token: string; name: string } | { error: 'format' } | null> {
  const { canceled, filePaths } = await dialog.showOpenDialog(win.base, {
    title: t('backup.import'),
    properties: ['openFile'],
    filters: [{ name: t('backup.fileType'), extensions: [BACKUP_EXTENSION] }],
  })
  if (canceled || !filePaths[0]) return null
  const data = await readFile(filePaths[0])
  if (!looksLikeBackup(data)) return { error: 'format' }
  return { token: holdBackup(data, basename(filePaths[0])), name: basename(filePaths[0]) }
}

/** Keeps an opened file until it's unlocked and restored, or ten minutes pass. */
export function holdBackup(data: Buffer, name: string): string {
  const token = randomUUID()
  opened.set(token, { data, name, timer: setTimeout(() => forget(token), 10 * 60_000) })
  return token
}

function isPayload(value: unknown): value is Payload {
  const p = value as Payload
  return !!p && p.format === 'peepwick-backup' && p.version === 1 && typeof p.createdAt === 'number'
}

function counts(payload: Payload): BackupCounts {
  const out: BackupCounts = {}
  if (payload.settings) out.settings = 1
  if (payload.bookmarks && isDraftTree(payload.bookmarks)) {
    let n = 0
    const count = (nodes: DraftTree['bar']) => nodes.forEach((node) => (node.type === 'url' ? n++ : count(node.children)))
    count(payload.bookmarks.bar)
    count(payload.bookmarks.other)
    out.bookmarks = n
  }
  if (Array.isArray(payload.workspaces)) {
    out.workspaces = payload.workspaces.length
    const tabs = payload.workspaces.reduce((n, w) => n + (Array.isArray(w.tabs) ? w.tabs.length : 0), 0)
    if (tabs > 0) out.tabs = tabs
  }
  if (payload.focus?.days) out.focus = Object.keys(payload.focus.days).length
  if (payload.permissions && typeof payload.permissions === 'object') {
    out.permissions = Object.values(payload.permissions as Record<string, object>).reduce((n, kinds) => n + Object.keys(kinds ?? {}).length, 0)
  }
  if (Array.isArray(payload.extensions)) out.extensions = payload.extensions.length
  if (Array.isArray(payload.history)) out.history = payload.history.length
  return out
}

export async function unlockBackup(token: string, password: string): Promise<BackupSummary | { error: 'password' | 'format' | 'expired' }> {
  const entry = opened.get(token)
  if (!entry) return { error: 'expired' }
  try {
    const payload = await decryptBackup(entry.data, password)
    if (!isPayload(payload)) return { error: 'format' }
    entry.payload = payload
    const installed = new Set(listExtensions().map((e) => e.id))
    return {
      token,
      createdAt: payload.createdAt,
      version: typeof payload.app === 'string' ? payload.app.slice(0, 20) : '',
      counts: counts(payload),
      extensions: (payload.extensions ?? [])
        .filter((e) => typeof e?.id === 'string' && /^[a-p]{32}$/.test(e.id) && !installed.has(e.id))
        .map((e) => ({ id: e.id, name: String(e.name ?? e.id).slice(0, 80) })),
    }
  } catch (error) {
    return { error: error instanceof BackupError ? error.reason : 'format' }
  }
}

function restoreSettings(incoming: Partial<Settings>): boolean {
  const patch = sanitizeSettingsPatch(incoming)
  for (const key of LOCAL_SETTINGS) delete patch[key]
  const current = settings()
  // Lists are joined rather than replaced: restoring never takes anything away.
  if (patch.focusSites) patch.focusSites = [...new Set([...current.focusSites, ...patch.focusSites])]
  if (patch.shieldAllowlist) patch.shieldAllowlist = [...new Set([...current.shieldAllowlist, ...patch.shieldAllowlist])]
  const changed = (Object.keys(patch) as (keyof Settings)[]).filter((key) => JSON.stringify(patch[key]) !== JSON.stringify(current[key]))
  if (changed.length === 0) return false
  updateSettings(patch)
  return true
}

function restoreWorkspaces(win: BrowserWin, workspaces: BackupWorkspace[], withTabs: boolean): { workspaces: number; tabs: number } {
  const startedIn = win.activeWorkspace().id
  let created = 0
  let opened = 0
  for (const saved of workspaces.slice(0, 12)) {
    if (typeof saved?.name !== 'string' || !saved.name.trim()) continue
    let workspace = win.workspaces.find((w) => w.name === saved.name)
    let fresh = false
    if (!workspace) {
      const before = win.workspaces.length
      win.createWorkspace(saved.name, WORKSPACE_ICONS.includes(saved.icon) ? saved.icon : 'circle')
      if (win.workspaces.length === before) break
      workspace = win.workspaces.at(-1)!
      fresh = true
      created++
    }
    if (!withTabs || !Array.isArray(saved.tabs)) continue
    const open = new Set(win.tabsInOrder(workspace).map((tab) => tab.url))
    let added = 0
    for (const tab of saved.tabs.slice(0, 200)) {
      if (typeof tab?.url !== 'string' || !/^(https?|file):/.test(tab.url) || open.has(tab.url)) continue
      win.openTab({ url: tab.url, title: typeof tab.title === 'string' ? tab.title : '', pinned: tab.pinned === true, workspaceId: workspace.id, background: true })
      open.add(tab.url)
      added++
    }
    opened += added
    // A new space starts with a new tab page; with restored tabs it isn't needed.
    if (fresh && added > 0) {
      const first = win.tabsInOrder(workspace).find((tab) => tab.url === NEW_TAB_URL)
      if (first) win.closeTab(first, { remember: false })
    }
  }
  win.activateWorkspace(startedIn)
  return { workspaces: created, tabs: opened }
}

async function restoreExtensions(ids: string[]): Promise<number> {
  const session = browsingSession()
  let installed = 0
  for (const id of ids) {
    if (!/^[a-p]{32}$/.test(id) || session.extensions.getExtension(id)) continue
    try {
      await installExtension(id, { session })
      installed++
    } catch (error) {
      console.error('[backup] could not install extension', error)
    }
  }
  return installed
}

export async function restoreBackup(win: BrowserWin, token: string, categories: BackupCategory[], extensionIds: string[]): Promise<RestoreResult | null> {
  const payload = opened.get(token)?.payload
  if (!payload) return null
  forget(token)
  const has = (c: BackupCategory) => categories.includes(c)
  const result: RestoreResult = { settings: false, bookmarks: 0, workspaces: 0, tabs: 0, focusDays: 0, permissions: 0, extensions: 0, history: 0 }
  if (has('settings') && payload.settings && typeof payload.settings === 'object') result.settings = restoreSettings(payload.settings)
  if (has('bookmarks') && isDraftTree(payload.bookmarks)) result.bookmarks = mergeBookmarks(payload.bookmarks)
  if (has('workspaces') && Array.isArray(payload.workspaces) && !win.incognito) {
    const restored = restoreWorkspaces(win, payload.workspaces, has('tabs'))
    result.workspaces = restored.workspaces
    result.tabs = restored.tabs
  }
  if (has('focus') && payload.focus?.days) result.focusDays = mergeStats(payload.focus)
  if (has('permissions')) result.permissions = mergePermissions(payload.permissions)
  if (has('history') && Array.isArray(payload.history)) {
    result.history = importVisits(
      payload.history.filter((v) => typeof v?.url === 'string' && typeof v.visitedAt === 'number').map((v) => ({ url: v.url, title: String(v.title ?? ''), visitedAt: v.visitedAt })),
    )
  }
  if (has('extensions')) result.extensions = await restoreExtensions(extensionIds)
  return result
}

export function cancelRestore(token: string): void {
  forget(token)
}
