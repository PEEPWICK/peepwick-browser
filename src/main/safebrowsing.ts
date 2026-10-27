// Dangerous site warnings: lists of hosts that spread malware or phish for passwords,
// from the malware-filter project (URLhaus, OpenPhish, PhishTank and IPThreat data),
// downloaded every 12 hours and checked on this computer. Only the lists are fetched;
// the addresses you visit never leave the browser.
import { net } from 'electron'
import { readFile, stat, writeFile } from 'node:fs/promises'
import { hostOf } from '@shared/url'
import { bus } from './events'
import { userPath } from './paths'
import { settings } from './settings'

export type Threat = 'malware' | 'phishing'

const LISTS: Record<Threat, string> = {
  malware: 'urlhaus-filter-domains.txt',
  phishing: 'phishing-filter-domains.txt',
}
const MIRRORS = ['https://malware-filter.gitlab.io/malware-filter/', 'https://curbengh.github.io/malware-filter/']
const REFRESH_AFTER = 12 * 60 * 60 * 1000

const lists = new Map<Threat, Set<string>>()
/** Hosts the person chose to open despite the warning; forgotten when the browser closes. */
const passed = new Set<string>()
let updating: Promise<void> | null = null

export function parseHostList(text: string): Set<string> {
  const hosts = new Set<string>()
  for (const line of text.split('\n')) {
    const host = line.trim().toLowerCase()
    if (host && !host.startsWith('#') && !host.startsWith('!')) hosts.add(host)
  }
  return hosts
}

async function download(file: string): Promise<string | null> {
  for (const base of MIRRORS) {
    try {
      const response = await net.fetch(base + file, { bypassCustomProtocolHandlers: true, credentials: 'omit' } as RequestInit)
      if (!response.ok) continue
      const text = await response.text()
      // A captive portal or error page isn't a list.
      if (text.startsWith('# Title:')) return text
    } catch {
      // Try the next mirror.
    }
  }
  return null
}

async function update(threat: Threat): Promise<void> {
  const path = userPath(`safebrowsing-${threat}.txt`)
  if (!lists.has(threat)) {
    try {
      lists.set(threat, parseHostList(await readFile(path, 'utf8')))
    } catch {
      // Not downloaded yet.
    }
  }
  const age = await stat(path).then((s) => Date.now() - s.mtimeMs, () => Infinity)
  if (age < REFRESH_AFTER) return
  const text = await download(LISTS[threat])
  if (!text) return
  await writeFile(path, text)
  lists.set(threat, parseHostList(text))
}

function refresh(): void {
  if (!settings().safeBrowsing || updating) return
  updating = Promise.all([update('malware'), update('phishing')])
    .then(() => undefined)
    .catch((error) => console.warn('[safebrowsing] could not update lists', error))
    .finally(() => (updating = null))
}

export function startSafeBrowsing(): void {
  refresh()
  // Checks hourly; a list is only downloaded again once it is 12 hours old.
  setInterval(refresh, 60 * 60 * 1000).unref()
  bus.on('settings', refresh)
}

/** Which list a host is on, looking at the host itself and the domains above it. */
export function listedThreat(host: string, from: Map<Threat, Set<string>>): Threat | null {
  let name = host.toLowerCase().replace(/\.$/, '')
  const ip = /^[\d.]+$/.test(name) || name.includes(':')
  for (;;) {
    for (const [threat, hosts] of from) if (hosts.has(name)) return threat
    const dot = name.indexOf('.')
    // Stop before a bare top-level domain; addresses are only listed whole.
    if (ip || dot < 0 || !name.slice(dot + 1).includes('.')) return null
    name = name.slice(dot + 1)
  }
}

export function threatFor(url: string): Threat | null {
  if (!settings().safeBrowsing || !/^https?:/.test(url)) return null
  const host = hostOf(url)
  if (!host || passed.has(host)) return null
  return listedThreat(host, lists)
}

export function passDangerous(url: string): void {
  passed.add(hostOf(url))
}
