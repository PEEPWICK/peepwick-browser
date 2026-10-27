// Downloads: the list (saved for normal windows, kept in memory for private ones),
// progress with a smoothed speed, and the actions the downloads page and panel offer.
import { app, BaseWindow, clipboard, Notification, shell, type DownloadItem, type Session } from 'electron'
import { randomUUID } from 'node:crypto'
import { existsSync, mkdirSync } from 'node:fs'
import { basename, extname, join } from 'node:path'
import type { DownloadAction } from '@shared/ipc'
import type { DownloadState } from '@shared/types'
import { bus } from './events'
import { resourcePath, userPath } from './paths'
import { tabForContents } from './registry'
import { settings, t } from './settings'
import { JsonFile } from './store'

const file = new JsonFile<DownloadState[]>(userPath('downloads.json'), 1000)
const KEEP = 500
/** How many the panel and every window's state carry; the page asks for the rest. */
const RECENT = 30
let list: DownloadState[] = []
const items = new Map<string, DownloadItem>()
const sessions = new Map<string, Session>()
let normalSession: Session | null = null
let broadcastTimer: NodeJS.Timeout | null = null

export function loadDownloads(): void {
  const saved = file.read()
  if (Array.isArray(saved)) {
    list = saved.map((d) => (d.status === 'progressing' || d.status === 'paused' ? { ...d, status: 'interrupted', speed: 0 } : d))
  }
}

/** The most recent downloads, for the panel. */
export function downloads(): DownloadState[] {
  return list.slice(0, RECENT).map(withFileCheck)
}

/** Every download, newest first; `text` matches the file name or address. */
export function allDownloads(text = ''): DownloadState[] {
  const needle = text.trim().toLocaleLowerCase()
  const matching = needle ? list.filter((d) => d.filename.toLocaleLowerCase().includes(needle) || d.url.toLowerCase().includes(needle)) : list
  return matching.map(withFileCheck)
}

function withFileCheck(entry: DownloadState): DownloadState {
  return entry.status === 'completed' ? { ...entry, missing: !existsSync(entry.path) } : entry
}

export function downloadFolder(): string {
  const chosen = settings().downloadDir
  return chosen && existsSync(chosen) ? chosen : app.getPath('downloads')
}

export function activeDownloadCount(): number {
  return list.filter((d) => d.status === 'progressing' || d.status === 'paused').length
}

function changed(immediate = false): void {
  file.schedule(list.filter((d) => !d.incognito).slice(0, KEEP))
  if (immediate) {
    if (broadcastTimer) clearTimeout(broadcastTimer)
    broadcastTimer = null
    bus.emit('global')
    return
  }
  if (broadcastTimer) return
  broadcastTimer = setTimeout(() => {
    broadcastTimer = null
    bus.emit('global')
  }, 250)
}

function uniquePath(dir: string, name: string): string {
  const safe = basename(name).replace(/[/\\\0]/g, '_') || 'download'
  const ext = extname(safe)
  const stem = safe.slice(0, safe.length - ext.length)
  let candidate = join(dir, safe)
  for (let i = 1; existsSync(candidate) && i < 1000; i++) candidate = join(dir, `${stem} (${i})${ext}`)
  return candidate
}

export function installDownloads(session: Session, incognito: boolean): void {
  if (!incognito) normalSession = session
  session.on('will-download', (_event, item, contents) => {
    const id = randomUUID()
    // A tab that never showed a page (a link opened just to download) goes away, as in Chrome.
    const tab = contents ? tabForContents(contents.id) : undefined
    if (tab && contents && !contents.getURL()) {
      setImmediate(() => {
        if (tab.openerId !== null) tab.win.closeTab(tab, { remember: false })
        else tab.showPage('newtab')
      })
    }
    const dir = downloadFolder()
    mkdirSync(dir, { recursive: true })
    if (settings().askWhereToSave) item.setSaveDialogOptions({ defaultPath: join(dir, item.getFilename()) })
    else item.setSavePath(uniquePath(dir, item.getFilename()))

    const entry: DownloadState = {
      id,
      filename: item.getFilename(),
      path: item.getSavePath(),
      url: item.getURL(),
      received: 0,
      total: item.getTotalBytes(),
      status: 'progressing',
      startedAt: Date.now(),
      speed: 0,
      ...(incognito ? { incognito: true } : {}),
    }
    list = [entry, ...list].slice(0, KEEP)
    items.set(id, item)
    sessions.set(id, session)
    changed(true)

    // Speed from the bytes received between samples, smoothed so the time left doesn't jump.
    let sampleAt = Date.now()
    let sampleBytes = 0
    item.on('updated', (_e, state) => {
      const now = Date.now()
      const received = item.getReceivedBytes()
      if (item.isPaused() || state === 'interrupted') entry.speed = 0
      else if (now - sampleAt >= 500) {
        const current = ((received - sampleBytes) * 1000) / (now - sampleAt)
        entry.speed = entry.speed ? entry.speed * 0.7 + current * 0.3 : current
        sampleAt = now
        sampleBytes = received
      }
      entry.received = received
      entry.total = item.getTotalBytes()
      entry.path = item.getSavePath() || entry.path
      entry.filename = basename(entry.path) || entry.filename
      entry.status = state === 'interrupted' ? 'interrupted' : item.isPaused() ? 'paused' : 'progressing'
      changed()
    })
    item.once('done', (_e, state) => {
      items.delete(id)
      entry.received = item.getReceivedBytes()
      entry.path = item.getSavePath() || entry.path
      entry.filename = basename(entry.path) || entry.filename
      entry.status = state === 'completed' ? 'completed' : state === 'cancelled' ? 'cancelled' : 'interrupted'
      entry.speed = 0
      entry.endedAt = Date.now()
      if (state === 'cancelled' && !item.getSavePath()) list = list.filter((d) => d !== entry)
      changed(true)
      if (state === 'completed' && !BaseWindow.getFocusedWindow() && Notification.isSupported()) {
        new Notification({ title: t('downloads.done'), body: entry.filename, icon: resourcePath('icon.png'), silent: true }).show()
      }
    })
  })
}

export function downloadAction(id: string, action: DownloadAction): void {
  const entry = list.find((d) => d.id === id)
  if (!entry) return
  const item = items.get(id)
  switch (action) {
    case 'open':
      if (entry.status === 'completed') void shell.openPath(entry.path)
      break
    case 'show':
      if (existsSync(entry.path)) shell.showItemInFolder(entry.path)
      else void shell.openPath(downloadFolder())
      break
    case 'cancel':
      item?.cancel()
      break
    case 'pause':
      item?.pause()
      break
    case 'resume':
      if (item?.canResume()) item.resume()
      break
    case 'retry': {
      // A download that stopped but can carry on resumes; anything else starts over.
      if (item?.canResume()) {
        item.resume()
        break
      }
      const session = sessions.get(id) ?? (entry.incognito ? null : normalSession)
      if (!session) break
      item?.cancel()
      list = list.filter((d) => d !== entry)
      changed(true)
      session.downloadURL(entry.url)
      break
    }
    case 'copy':
      clipboard.writeText(entry.url)
      break
    case 'trash':
      if (entry.status !== 'completed' || !existsSync(entry.path)) break
      void shell
        .trashItem(entry.path)
        .then(() => {
          list = list.filter((d) => d !== entry)
          changed(true)
        })
        .catch(() => {})
      break
    case 'remove':
      if (item) item.cancel()
      list = list.filter((d) => d !== entry)
      changed(true)
      break
  }
}

export function openDownloadFolder(): void {
  void shell.openPath(downloadFolder())
}

/** Clears finished downloads from the list (files stay), all of them or those since `since`. */
export function clearDownloads(since = 0): number {
  const before = list.length
  list = list.filter((d) => d.status === 'progressing' || d.status === 'paused' || d.startedAt < since)
  changed(true)
  return before - list.length
}

export function countDownloads(since = 0): number {
  return list.filter((d) => d.status !== 'progressing' && d.status !== 'paused' && d.startedAt >= since && !d.incognito).length
}

export function cancelAllDownloads(): void {
  for (const item of items.values()) item.cancel()
}

export function flushDownloads(): void {
  file.flush()
}
