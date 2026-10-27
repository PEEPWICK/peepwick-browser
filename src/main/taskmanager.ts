// The task manager: every process of the browser with its memory and processor use,
// tabs grouped by the process they share, and a way to end a tab's process when a
// page hangs or eats memory.
import { app, webContents } from 'electron'
import type { TaskRow } from '@shared/types'
import { displayHost } from '@shared/url'
import { BrowserWin } from './window'

const SERVICES: Record<string, string> = {
  'network.mojom.NetworkService': 'network',
  'audio.mojom.AudioService': 'audio',
  'storage.mojom.StorageService': 'storage',
  'video_capture.mojom.VideoCaptureService': 'video',
  'data_decoder.mojom.DataDecoderService': 'decoder',
}

function pidOf(contents: Electron.WebContents): number {
  try {
    return contents.isDestroyed() ? 0 : contents.getOSProcessId()
  } catch {
    return 0
  }
}

export function taskRows(): TaskRow[] {
  const tabs = new Map<number, TaskRow['tabs']>()
  const ui = new Set<number>()
  for (const win of BrowserWin.all.values()) {
    ui.add(pidOf(win.ui.webContents))
    for (const tab of win.tabs.values()) {
      const contents = tab.webContents
      if (!contents) continue
      const pid = pidOf(contents)
      if (!pid) continue
      const list = tabs.get(pid) ?? []
      list.push({ id: tab.id, title: tab.title || displayHost(tab.url) || tab.url, url: tab.url, favicon: tab.favicon })
      tabs.set(pid, list)
    }
  }
  const extensions = new Map<number, string>()
  for (const contents of webContents.getAllWebContents()) {
    const url = contents.getURL()
    if (url.startsWith('chrome-extension://')) extensions.set(pidOf(contents), contents.getTitle() || new URL(url).hostname)
  }

  const rows = app.getAppMetrics().map((metric): TaskRow => {
    const base = { pid: metric.pid, name: '', tabs: [] as TaskRow['tabs'], memory: metric.memory.workingSetSize * 1024, cpu: metric.cpu.percentCPUUsage }
    if (metric.type === 'Browser') return { ...base, kind: 'browser' }
    if (metric.type === 'GPU') return { ...base, kind: 'gpu' }
    if (metric.type === 'Utility') return { ...base, kind: 'service', name: SERVICES[metric.serviceName ?? ''] ?? metric.name ?? '' }
    if (metric.type === 'Tab') {
      const shown = tabs.get(metric.pid)
      if (shown) return { ...base, kind: 'tab', tabs: shown }
      if (ui.has(metric.pid)) return { ...base, kind: 'ui' }
      const extension = extensions.get(metric.pid)
      if (extension) return { ...base, kind: 'extension', name: extension }
    }
    return { ...base, kind: 'other' }
  })
  const order: TaskRow['kind'][] = ['browser', 'gpu', 'ui']
  return rows.sort((a, b) => {
    const rank = (row: TaskRow) => (order.includes(row.kind) ? order.indexOf(row.kind) : order.length)
    return rank(a) - rank(b) || b.memory - a.memory
  })
}

/** Ends a tab's process; its tabs show the "page stopped" screen until reloaded. */
export function endTask(pid: number): void {
  for (const win of BrowserWin.all.values()) {
    for (const tab of win.tabs.values()) {
      const contents = tab.webContents
      if (contents && pidOf(contents) === pid) {
        contents.forcefullyCrashRenderer()
        return
      }
    }
  }
}
