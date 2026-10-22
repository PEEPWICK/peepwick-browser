import { app, BaseWindow, Notification, shell, type DownloadItem, type Session } from 'electron'
import { randomUUID } from 'node:crypto'
import { existsSync } from 'node:fs'
import { basename, extname, join } from 'node:path'
import type { DownloadState } from '@shared/types'
import { bus } from './events'
import { resourcePath, userPath } from './paths'
import { tabForContents } from './registry'
import { settings, t } from './settings'
import { JsonFile } from './store'

const file = new JsonFile<DownloadState[]>(userPath('downloads.json'), 1000)
let list: DownloadState[] = []
const items = new Map<string, DownloadItem>()
/** Downloads from private windows are listed while the browser runs but never saved. */
const privateIds = new Set<string>()
let broadcastTimer: NodeJS.Timeout | null = null

export function loadDownloads(): void {
  const saved = file.read()
  if (Array.isArray(saved)) {
    list = saved.map((d) => (d.status === 'progressing' || d.status === 'paused' ? { ...d, status: 'interrupted' } : d))
  }
}

export function downloads(): DownloadState[] {
  return list
}

export function activeDownloadCount(): number {
  return list.filter((d) => d.status === 'progressing' || d.status === 'paused').length
}

function changed(immediate = false): void {
  file.schedule(list.filter((d) => !privateIds.has(d.id)).slice(0, 50))
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
  session.on('will-download', (_event, item, contents) => {
    const id = randomUUID()
    if (incognito) privateIds.add(id)
    // A tab that never showed a page (a link opened just to download) goes away, as in Chrome.
    const tab = contents ? tabForContents(contents.id) : undefined
    if (tab && contents && !contents.getURL()) {
      setImmediate(() => {
        if (tab.openerId !== null) tab.win.closeTab(tab, { remember: false })
        else tab.showPage('newtab')
      })
    }
    const dir = app.getPath('downloads')
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
    }
    list = [entry, ...list].slice(0, 100)
    items.set(id, item)
    changed(true)

    item.on('updated', (_e, state) => {
      entry.received = item.getReceivedBytes()
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
      if (state === 'cancelled' && !item.getSavePath()) list = list.filter((d) => d !== entry)
      changed(true)
      if (state === 'completed' && !BaseWindow.getFocusedWindow() && Notification.isSupported()) {
        new Notification({ title: t('downloads.done'), body: entry.filename, icon: resourcePath('icon.png'), silent: true }).show()
      }
    })
  })
}

export function downloadAction(id: string, action: 'open' | 'show' | 'cancel' | 'pause' | 'resume' | 'remove'): void {
  const entry = list.find((d) => d.id === id)
  if (!entry) return
  const item = items.get(id)
  switch (action) {
    case 'open':
      if (entry.status === 'completed') void shell.openPath(entry.path)
      break
    case 'show':
      if (existsSync(entry.path)) shell.showItemInFolder(entry.path)
      else void shell.openPath(app.getPath('downloads'))
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
    case 'remove':
      if (item) item.cancel()
      list = list.filter((d) => d !== entry)
      changed(true)
      break
  }
}

export function clearDownloads(): void {
  list = list.filter((d) => d.status === 'progressing' || d.status === 'paused')
  changed(true)
}

export function cancelAllDownloads(): void {
  for (const item of items.values()) item.cancel()
}

export function flushDownloads(): void {
  file.flush()
}
