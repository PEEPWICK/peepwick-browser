// Page zoom remembered per site, like Chrome: zoom in on a site and it opens at
// that size next time. Sites without their own size use the default from
// Settings › Appearance. Private windows remember only until they close.
import { hostOf } from '@shared/url'
import { bus } from './events'
import { userPath } from './paths'
import { settings } from './settings'
import { JsonFile } from './store'

const file = new JsonFile<Record<string, number>>(userPath('zoom.json'), 500)
let saved: Record<string, number> = {}
const incognito = new Map<string, number>()

export function loadZoom(): void {
  const data = file.read()
  if (data && typeof data === 'object') saved = data
  let defaultZoom = settings().defaultZoom
  bus.on('settings', () => {
    if (settings().defaultZoom === defaultZoom) return
    defaultZoom = settings().defaultZoom
    bus.emit('zoom')
  })
}

export function flushZoom(): void {
  file.flush()
}

export function zoomFor(url: string, privateWindow: boolean): number {
  const host = hostOf(url)
  if (!host) return settings().defaultZoom
  return (privateWindow ? incognito.get(host) : undefined) ?? saved[host] ?? settings().defaultZoom
}

/** Remembers a site's zoom; the default size isn't stored. */
export function rememberZoom(url: string, factor: number, privateWindow: boolean): void {
  const host = hostOf(url)
  if (!host || !/^https?:|^file:/.test(url)) return
  const atDefault = Math.abs(factor - settings().defaultZoom) < 0.001
  if (privateWindow) {
    if (atDefault) incognito.delete(host)
    else incognito.set(host, factor)
    return
  }
  if (atDefault) delete saved[host]
  else saved[host] = Math.round(factor * 100) / 100
  file.schedule(saved)
}

export function siteZooms(): { host: string; zoom: number }[] {
  return Object.entries(saved)
    .map(([host, zoom]) => ({ host, zoom }))
    .sort((a, b) => a.host.localeCompare(b.host))
}

export function forgetZoom(host: string | null): void {
  if (host === null) saved = {}
  else delete saved[host]
  file.schedule(saved)
  bus.emit('zoom')
}
